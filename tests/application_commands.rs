use multi_agent_on_chat::{
    application::{self, agents::ConfigPatch, Scope},
    domain::*,
    presentation, service,
    store::Store,
};
use serde_json::{json, Value};
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path()).unwrap();
    for id in ["a", "b"] {
        s.add_bot_with_defaults(&Bot {
            id: id.into(),
            channel: "local".into(),
            account_id: id.into(),
            credential_ref: None,
            owner: Some("owner".into()),
            enabled: true,
        })
        .unwrap();
    }
    (dir, s)
}
#[tokio::test]
async fn invalid_parameters_do_not_mutate_config_or_create_work() {
    let (_d, s) = setup();
    let before = s.get::<Value>("agent", "a-main").unwrap();
    for text in [
        "/model test low extra",
        "/agent use a-main extra",
        "/agent disable a-main --oops",
        "/cron list extra",
        "/clear extra",
        "/tasks --page 0",
        "/tasks --limit 101",
        "/agent create nope --template default@1 extra",
    ] {
        assert!(
            service::chat(&s, "a-main", "scope", text, None)
                .await
                .is_err(),
            "{text}"
        );
    }
    assert_eq!(before, s.get::<Value>("agent", "a-main").unwrap());
    assert!(s.list::<Task>("task").unwrap().is_empty());
    assert!(s.list::<Value>("draft").unwrap().is_empty());
}
#[tokio::test]
async fn free_text_and_missing_skill_recovery_survive_adapter_parsing() {
    let (_d, s) = setup();
    service::chat(
        &s,
        "a-main",
        "scope",
        "/system Don't parse this --page as options",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        s.get::<Agent>("agent", "a-main")
            .unwrap()
            .config
            .system_prompt,
        "Don't parse this --page as options"
    );
    s.update_agent("a-main", |a| {
        a.config.skills_mode = SkillsMode::Allowlist;
        a.config.skills = vec!["/missing/SKILL.md".into()];
        Ok(())
    })
    .unwrap();
    let listed = service::chat(&s, "a-main", "scope", "/skills", None)
        .await
        .unwrap();
    assert!(listed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["missing"] == true));
    service::chat(
        &s,
        "a-main",
        "scope",
        "/skills disable /missing/SKILL.md",
        None,
    )
    .await
    .unwrap();
    assert!(s
        .get::<Agent>("agent", "a-main")
        .unwrap()
        .config
        .skills
        .is_empty());
}
#[tokio::test]
async fn help_details_and_pagination_preserve_scope() {
    let (_d, s) = setup();
    assert_eq!(
        service::chat(&s, "a-main", "scope", "/help agent", None)
            .await
            .unwrap(),
        service::chat(&s, "a-main", "scope", "/agent help", None)
            .await
            .unwrap()
    );
    for i in 0..25 {
        s.create_agent(&format!("a{i}"), "a", "default@1", &s.root)
            .unwrap();
    }
    let page = service::chat(
        &s,
        "a-main",
        "scope",
        "/agent list --page 2 --limit 10 -r",
        None,
    )
    .await
    .unwrap();
    assert_eq!(page["total"], 26);
    assert_eq!(page["items"].as_array().unwrap().len(), 10);
    assert!(page["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["bot_id"] == "a"));
    assert!(presentation::render("/agent list --page 2", &page).contains("第 2 页"));
    assert!(
        service::chat(&s, "a-main", "scope", "/agent show b-main", None)
            .await
            .is_err()
    );
    let task = s.submit("b-main", "scope", "secret", None, None).unwrap();
    assert!(service::chat(
        &s,
        "a-main",
        "scope",
        &format!("/task show {}", task.id),
        None
    )
    .await
    .is_err());
}
#[test]
fn shared_configuration_and_deleted_job_guards() {
    let (_d, s) = setup();
    let changed = application::agents::configure(
        &s,
        &Scope::Bot("a".into()),
        "a-main",
        ConfigPatch {
            model: Some("test".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(changed.config.model.as_deref(), Some("test"));
    assert!(application::agents::configure(
        &s,
        &Scope::Bot("b".into()),
        "a-main",
        ConfigPatch::default()
    )
    .is_err());
    let j = s
        .job_create("a-main", "every 1h", "UTC", "fixture", None)
        .unwrap();
    s.job_action(&j.id, "delete").unwrap();
    for action in ["pause", "resume", "delete"] {
        assert!(s.job_action(&j.id, action).is_err());
    }
    assert_eq!(s.get::<Job>("job", &j.id).unwrap().state, "deleted");
}
#[tokio::test]
async fn result_and_delivery_history_are_distinct() {
    let (_d, s) = setup();
    let reply = ReplyTarget {
        chat_id: "owner".into(),
        context_token: None,
        thread_id: None,
    };
    let j = s
        .job_create("a-main", "every 1h", "UTC", "fixture", Some(reply))
        .unwrap();
    s.fire_due(j.next_fire.unwrap()).unwrap();
    let task = s.claim(1).unwrap().unwrap();
    s.finish(&task.id, task.generation, "succeeded", "result")
        .unwrap();
    let h = s.job_history(&j.id).unwrap();
    assert_eq!(h[0]["execution_state"], "succeeded");
    assert_eq!(h[0]["delivery_state"], "pending");
    let details = application::tasks::details(&s, &Scope::Bot("a".into()), &task.id).unwrap();
    assert_eq!(details["result"], "result");
    let plain = "literal **text** <angle> &lt;";
    let output = application::output::Output::Detail {
        title: "Title".into(),
        fields: vec![("value".into(), plain.into())],
        hints: vec![],
    }
    .message();
    assert!(output.plain_text().contains(plain));
    let bot: Bot = s.get("bot", "a").unwrap();
    let target = ReplyTarget {
        chat_id: "owner".into(),
        context_token: None,
        thread_id: None,
    };
    multi_agent_on_chat::delivery::send(&s, &bot, &target, &output, "test-delivery")
        .await
        .unwrap();
    assert!(s
        .get::<bool>("delivery_part_sent", "test-delivery:part:0")
        .unwrap());
    multi_agent_on_chat::delivery::send(&s, &bot, &target, &output, "test-delivery")
        .await
        .unwrap();
    assert_eq!(s.list::<Value>("delivery_plan").unwrap().len(), 1);
    assert_eq!(
        json!(application::execution::FailureKind::classify(
            &anyhow::anyhow!("failed to load configuration")
        )),
        "outcome_uncertain"
    );
}
