use multi_agent_on_chat::{
    domain::*,
    presentation::{self, render, split_raw},
    service,
    store::Store,
};
use serde_json::{json, Value};
use tempfile::TempDir;
#[tokio::test]
async fn bare_commands_show_current_context_without_mutation() {
    let (_d, s) = setup();
    let before = s.list::<Value>("agent").unwrap();
    for command in [
        "/agent",
        "/bot",
        "/profile",
        "/model",
        "/system",
        "/cd",
        "/template",
        "/cron",
    ] {
        let value = check(&s, command).await;
        let text = render(command, &value);
        assert!(!text.contains("已更新"), "{command}: {text}");
        assert!(!text.contains("已修改"), "{command}: {text}");
        if command == "/agent" {
            assert!(
                text.contains("当前 Agent") && text.contains("a-main") && text.contains("工作目录")
            );
            assert!(!text.contains("Agent 用法"));
            assert_eq!(
                serde_json::from_str::<Value>(&render("/agent -r", &value)).unwrap(),
                value
            );
        }
        if command == "/model" {
            assert!(text.contains("继承 Profile / Codex 默认"));
        }
        if command == "/cron" {
            assert!(text.contains("/cron add"));
            assert_eq!(value, check(&s, "/cron list").await);
        }
    }
    for command in [
        "/retry",
        "/sh",
        "/ask",
        "/ask a-main",
        "/collaboration",
        "/collaboration show",
    ] {
        let error = service::chat(&s, "a-main", "test", command, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("usage:"), "{command}: {error}");
    }
    assert_eq!(s.list::<Value>("agent").unwrap(), before);
    assert!(s.list::<Task>("task").unwrap().is_empty());
    assert!(s.list::<Value>("draft").unwrap().is_empty());
    check(&s, "/agent create other --template basic@1").await;
    let switched = check(&s, "/agent use other").await;
    let current = service::chat(
        &s,
        switched["agent"].as_str().unwrap(),
        "test",
        "/agent",
        None,
    )
    .await
    .unwrap();
    assert_eq!(current["id"], "other");
    assert!(render("/agent", &current).contains("other"));
}
#[test]
fn skills_list_is_compact_and_show_expands_full_details() {
    let description = "详细说明".repeat(100);
    let skills = (0..47).map(|i| json!({"name":format!("skill-{i}"),"enabled":true,"description":description,"path":"/private/skill/SKILL.md"})).collect::<Vec<_>>();
    let value = json!({"agent_id":"a","mode":"inherit","skills":skills});
    let list = render("/skills", &value);
    assert!(list.contains("skill-46"));
    assert!(!list.contains(&description));
    assert!(!list.contains("/private/skill"));
    assert!(list.contains("/skills show 名称"));
    let show = render("/skills show skill-0", &value);
    assert!(show.contains(&description));
    assert!(show.contains("/private/skill/SKILL.md"));
}
fn setup() -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path()).unwrap();
    s.register_template(&Template {
        id: "basic".into(),
        version: 1,
        description: "test".into(),
        system_prompt: "role".into(),
        profile: None,
        model: None,
        effort: None,
        skills: vec!["test-skill".into()],
        skills_mode: Default::default(),
    })
    .unwrap();
    for id in ["a", "b"] {
        s.add_bot_with_defaults(&Bot {
            id: id.into(),
            channel: "local".into(),
            account_id: id.into(),
            credential_ref: None,
            enabled: true,
            owner: Some("owner".into()),
        })
        .unwrap();
    }
    (d, s)
}
async fn check(s: &Store, command: &str) -> Value {
    let value = service::chat(s, "a-main", "test", command, None)
        .await
        .unwrap_or_else(|e| panic!("{command}: {e}"));
    let formatted = render(command, &value);
    assert!(formatted.starts_with("**"), "{command}: {formatted}");
    assert!(!formatted.contains("credential_ref"), "{command}");
    let rich = presentation::message(command, &value);
    assert!(
        matches!(rich, multi_agent_on_chat::message::Message::RichText(_)),
        "{command}"
    );
    let plain = rich.plain_text();
    assert!(
        !plain.contains("&lt;") && !plain.contains("&gt;") && !plain.contains("\\["),
        "{command}: {plain}"
    );
    if command == "/bot list" || command == "/bot show" {
        assert!(formatted.contains("local"), "{command}: {formatted}");
    }
    let raw = render(&format!("{command} -r"), &value);
    assert_eq!(
        serde_json::from_str::<Value>(&raw).unwrap(),
        value,
        "{command}"
    );
    value
}
#[tokio::test]
async fn agent_list_defaults_to_own_bot_and_global_flag_expands_scope() {
    let (_d, s) = setup();
    let local = check(&s, "/agent list").await;
    assert_eq!(local.as_array().unwrap().len(), 1);
    assert_eq!(local[0]["bot_id"], "a");
    let global = check(&s, "/agent list -g").await;
    assert_eq!(global.as_array().unwrap().len(), 2);
    let raw = service::chat(&s, "a-main", "test", "/agent list -g -r", None)
        .await
        .unwrap();
    assert_eq!(raw, global);
    for invalid in ["/agent list -x", "/agent list -g extra", "/agents"] {
        assert!(service::chat(&s, "a-main", "test", invalid, None)
            .await
            .is_err());
    }
}
#[test]
fn flag_quoting_and_markdown_safety() {
    for c in presentation::COMMANDS {
        assert_eq!(split_raw(&format!("{c} -r")), (*c, true));
    }
    for input in [
        "hello -r",
        "/profile --raw",
        "/sh echo '-r'",
        "/sh echo \\-r",
        "/sh echo 'unfinished -r",
        "/system raw detail",
    ] {
        assert!(!split_raw(input).1, "{input}");
    }
    assert_eq!(split_raw("/sh echo '-r' -r"), ("/sh echo '-r'", true));
    let out = render("/skills", &json!(["a|b\nc<script>*_"]));
    assert!(out.contains("a\\|b c&lt;script&gt;\\*\\_"));
    let out = render("/skills", &json!(["x".repeat(500)]));
    assert!(out.contains("-r"));
    let out = render("/skills", &json!((0..21).collect::<Vec<_>>()));
    assert!(out.contains("21 项") && out.contains("前 20"));
}
#[tokio::test]
async fn complete_help_and_unified_template_listing() {
    let (_d, s) = setup();
    let help = check(&s, "/help").await;
    let text = render("/help", &help);
    assert!(text.starts_with("**/help "));
    assert!(!text.contains("常用命令") && !text.contains("/templates"));
    let entries = help["commands"].as_array().unwrap();
    assert_eq!(entries.len(), multi_agent_on_chat::help::HELP.len());
    let blocks: Vec<_> = text.split("\n\n").collect();
    assert_eq!(blocks.len(), entries.len());
    assert!(blocks.iter().all(|block| block.lines().count() == 2));
    let mut commands = std::collections::HashSet::new();
    for entry in entries {
        let first = entry["usage"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert!(commands.insert(first), "duplicated help entry: {first}");
    }
    for cmd in presentation::COMMANDS {
        assert!(
            entries.iter().any(|e| e["usage"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .any(|w| w == *cmd)),
            "missing {cmd}"
        );
    }
    for entry in entries {
        assert!(!entry["description"].as_str().unwrap().is_empty());
        assert!(
            text.contains(entry["description"].as_str().unwrap()),
            "description missing: {entry}"
        );
    }
    assert!(text.contains("show/pause/resume/history/delete ID"));
    assert!(render("/help -r", &help).contains("/collaboration show"));
    let list = check(&s, "/template").await;
    assert_eq!(list, check(&s, "/template list").await);
    assert!(service::chat(&s, "a-main", "test", "/templates", None)
        .await
        .is_err());
    let raw = service::chat(&s, "a-main", "test", "/template -r", None)
        .await
        .unwrap();
    assert_eq!(raw, list);
    let out = render("/template", &list);
    assert!(out.contains("basic") && out.contains("模板列表"));
}
#[tokio::test]
async fn from_agent_overwrites_default_version_without_mutating_existing_agents() {
    let (_d, s) = setup();
    check(&s, "/template from-agent reusable").await;
    check(&s, "/template from-agent reusable@2").await;
    check(&s, "/agent create old --template reusable@1").await;
    let old: Agent = s.get("agent", "old").unwrap();
    let second: Template = s.get("template", "reusable@2").unwrap();
    s.update_agent("a-main", |a| {
        a.config.system_prompt = "new prompt".into();
        Ok(())
    })
    .unwrap();
    let saved = check(&s, "/template from-agent reusable").await;
    assert_eq!(saved["system_prompt"], "new prompt");
    assert_eq!(
        s.get::<Template>("template", "reusable@1")
            .unwrap()
            .system_prompt,
        "new prompt"
    );
    assert_eq!(
        serde_json::to_value(s.get::<Template>("template", "reusable@2").unwrap()).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    assert_eq!(
        serde_json::to_value(s.get::<Agent>("agent", "old").unwrap()).unwrap(),
        serde_json::to_value(old).unwrap()
    );
    check(&s, "/agent create fresh --template reusable@1").await;
    assert_eq!(
        s.get::<Agent>("agent", "fresh")
            .unwrap()
            .config
            .system_prompt,
        "new prompt"
    );
}
#[tokio::test]
async fn deleting_one_template_version_preserves_agents_and_other_versions() {
    let (_d, s) = setup();
    check(&s, "/template from-agent disposable@1").await;
    check(&s, "/template from-agent disposable@2").await;
    check(&s, "/agent create kept --template disposable@1").await;
    let before: Agent = s.get("agent", "kept").unwrap();
    let deleted = check(&s, "/template delete disposable@1").await;
    assert_eq!(deleted["deleted"], "disposable@1");
    assert!(s.get::<Template>("template", "disposable@1").is_err());
    assert!(s.get::<Template>("template", "disposable@2").is_ok());
    let after: Agent = s.get("agent", "kept").unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    for command in [
        "/template delete",
        "/template delete default@1",
        "/template delete disposable",
        "/template delete disposable@1",
        "/template delete disposable@2 extra",
    ] {
        assert!(service::chat(&s, "a-main", "test", command, None)
            .await
            .is_err());
    }
    assert!(s.get::<Template>("template", "disposable@2").is_ok());
    let help = service::chat(&s, "a-main", "test", "/template help", None)
        .await
        .unwrap();
    let text = presentation::message("/template help", &help).plain_text();
    assert!(text.contains("/template show <name@version>"));
    assert!(text.contains("[agent-id]"));
    assert!(!text.contains("**") && !text.contains("&lt;") && !text.contains("\\["));
}
#[tokio::test]
async fn every_chat_branch_uses_real_results() {
    let (d, s) = setup();
    for c in [
        "/help",
        "/bot",
        "/bot show",
        "/bot show b",
        "/bot list",
        "/bot help",
        "/bot disable b",
        "/bot enable b",
        "/profile",
        "/profile list",
        "/profile default",
        "/model",
        "/model gpt-test low",
        "/model effort high",
        "/model default",
        "/system",
        "/system custom prompt",
        "/system default",
        "/cd",
        "/skills",
        "/agent list",
        "/agent list -g",
        "/template",
        "/template list",
        "/template help",
        "/template show basic@1",
        "/template from-agent copy@1",
        "/template from-agent a-main copy@2",
        "/agent",
        "/agent create extra --template basic@1",
        "/agent use extra",
        "/agent delete extra",
        "/agent create forced --template basic@1",
        "/agent delete forced --force",
        "/status",
        "/tasks",
        "/collaborations",
        "/inbox",
        "/recv",
        "/cron",
        "/cron list",
    ] {
        check(&s, c).await;
    }
    check(&s, &format!("/cd {}", d.path().display())).await;
    let task = service::chat(&s, "a-main", "test", "/sh echo '-r' -r", None)
        .await
        .unwrap();
    assert_eq!(task["prompt"], "echo '-r'");
    let canceled = check(&s, &format!("/cancel {}", task["id"].as_str().unwrap())).await;
    assert_eq!(canceled["state"], "cancelled");
    let mut failed: Task = s.get("task", task["id"].as_str().unwrap()).unwrap();
    failed.state = "failed".into();
    s.save("task", &failed.id, &failed).unwrap();
    check(&s, &format!("/retry {}", failed.id)).await;
    let child = check(&s, "/ask b-main test collaboration").await;
    check(&s, "/collaborations").await;
    check(
        &s,
        &format!(
            "/collaboration show {}",
            child["session_id"].as_str().unwrap()
        ),
    )
    .await;
    check(&s, "/clear").await;
    assert!(service::chat(&s, "a-main", "test", "/compact", None)
        .await
        .is_err());
    let compact_task = s.submit("a-main", "test", "fixture", None, None).unwrap();
    s.cancel(&compact_task.id).unwrap();
    let mut session: Session = s.get("session", &compact_task.session_id).unwrap();
    session.thread_id = Some("fixture-thread".into());
    s.save("session", &session.id, &session).unwrap();
    check(&s, "/compact").await;
    // Scheduling fixtures stay in an isolated temporary runtime.
    check(&s, "/cron add every 1h -- test").await;
    check(&s, "cancel").await;
    check(&s, "/cron add every 1h -- test").await;
    check(&s, "取消").await;
    check(&s, "/cron add every 1h -- test").await;
    let confirmed = check(&s, "confirm").await;
    let id = confirmed["job"]["id"].as_str().unwrap();
    for action in ["show", "pause", "resume", "history", "delete"] {
        check(&s, &format!("/cron {action} {id}")).await;
    }
    check(&s, "/cron add every 1h -- test").await;
    check(&s, "确认").await;
    let add = service::chat(&s, "a-main", "test", "/bot add local --name added -r", None)
        .await
        .unwrap();
    let done = service::wait_bot_add(&s, add).await.unwrap();
    assert_eq!(done["state"], "completed");
    check(&s, "/bot add-status added").await;
    for c in [
        "/bot invalid",
        "/bot add",
        "/bot add lark --name fail",
        "/template show missing@1",
        "/template show basic@1 extra",
        "/template list extra",
        "/template help extra",
        "/profile ../secret",
        "/profile default extra",
        "/agent use missing",
        "/cancel missing",
        "/retry missing",
        "/cd /does-not-exist-mac-test",
        "/sh",
        "/ask",
        "/collaboration show unknown",
        "/cron show missing",
        "/unknown",
    ] {
        let e = service::chat(&s, "a-main", "test", c, None)
            .await
            .unwrap_err();
        assert!(presentation::error(c, &e.to_string()).contains("执行失败"));
    }
}
#[test]
fn tools_models_and_async_states() {
    let cases = [
        (
            "agent_profile",
            json!({"action":"list"}),
            json!({"profiles":["default","fast"]}),
        ),
        (
            "bot_command",
            json!({"action":"list"}),
            json!([{ "id":"b","enabled":true}]),
        ),
        (
            "bot_add",
            json!({}),
            json!({"accepted":true,"name":"b","status":"running","check":"/bot add-status b"}),
        ),
        (
            "template_from_agent",
            json!({}),
            json!({"id":"t","version":1}),
        ),
        (
            "agent_list",
            json!({}),
            json!({"agents":[{"id":"a","bot_id":"b"}]}),
        ),
        (
            "agent_query",
            json!({}),
            json!({"accepted":true,"request_id":"t","collaboration_id":"s","state":"queued"}),
        ),
    ];
    for (name, mut args, v) in cases {
        assert!(presentation::tool(name, &args, &v)
            .as_str()
            .unwrap()
            .starts_with("**"));
        args["raw"] = json!(true);
        assert_eq!(presentation::tool(name, &args, &v), v);
    }
    for v in [
        json!({"state":"running","id":"b"}),
        json!({"state":"failed","id":"b","error":"bad app"}),
        json!({"state":"completed","id":"b","result":{"bot_id":"b","pending_owner":true,"default_agent":"b-main"}}),
    ] {
        let s = render("/bot add-status b", &v);
        assert!(s.starts_with("**"));
        if v["state"] == "failed" {
            assert!(s.contains("bad app"));
        }
    }
    let v = json!({"data":[{"id":"test","displayName":"Test","defaultReasoningEffort":"low","supportedReasoningEfforts":[{"reasoningEffort":"high"}]}]});
    let out = render("/model list", &v);
    assert!(out.contains("Test") && out.contains("high"));
    let history = render(
        "/cron history id",
        &json!([{"scheduled_for":42,"task_id":"task1","status":"enqueued"}]),
    );
    assert!(history.contains("42") && history.contains("enqueued"));
    let inbox = render(
        "/recv",
        &json!([{"task_id":"task1","text":"hello","state":"sent"}]),
    );
    assert!(inbox.contains("hello") && inbox.contains("task1"));
}
