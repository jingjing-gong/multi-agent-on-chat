use multi_agent_on_chat::{domain::*, store::Store};
use tempfile::TempDir;
fn setup() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let s = Store::open(dir.path()).unwrap();
    let t = Template {
        id: "basic".into(),
        version: 1,
        description: String::new(),
        system_prompt: "original".into(),
        profile: None,
        model: None,
        effort: None,
        skills: vec![],
        skills_mode: Default::default(),
    };
    s.register_template(&t).unwrap();
    for b in ["x", "y"] {
        s.add_bot(&Bot {
            id: b.into(),
            channel: "local".into(),
            account_id: b.into(),
            credential_ref: None,
            enabled: true,
            owner: Some("local".into()),
        })
        .unwrap();
        let workspace = dir.path().join(b);
        std::fs::create_dir(&workspace).unwrap();
        s.create_agent(b, b, "basic@1", &workspace).unwrap();
    }
    (dir, s)
}
#[test]
fn template_versions_are_immutable() {
    let (_d, s) = setup();
    let mut t: Template = s.get("template", "basic@1").unwrap();
    s.register_template(&t).unwrap();
    t.system_prompt = "mutated".into();
    assert!(s.register_template(&t).is_err());
}
#[test]
fn ordinary_text_cannot_become_shell_and_cwd_is_frozen() {
    let (d, s) = setup();
    let a = s
        .submit("x", "chat", "__MAC_SHELL__echo injected", None, None)
        .unwrap();
    assert_eq!(a.kind, "agent");
    let shell = s.submit_shell("x", "chat", "pwd", None).unwrap();
    assert_eq!(shell.kind, "shell");
    let sess: Session = s.get("session", &shell.session_id).unwrap();
    s.update_agent("x", |a| {
        a.workspace = d.path().join("y");
        Ok(())
    })
    .unwrap();
    assert_eq!(sess.workspace, d.path().join("x").canonicalize().unwrap());
}
#[test]
fn shell_lane_has_independent_capacity_and_workspace_serialization() {
    let (_d, s) = setup();
    s.submit("x", "chat", "busy agent", None, None).unwrap();
    let agent = s.claim_agent(1).unwrap().unwrap();
    let first = s.submit_shell("x", "chat", "pwd", None).unwrap();
    let one = s.claim_shell(1).unwrap().unwrap();
    assert_eq!(one.id, first.id);
    let second = s.submit_shell("x", "chat", "pwd", None).unwrap();
    let other = s.submit_shell("y", "chat", "pwd", None).unwrap();
    assert!(s.claim_shell(1).unwrap().is_none());
    assert_eq!(s.claim_shell(4).unwrap().unwrap().id, other.id);
    assert!(s.claim_shell(4).unwrap().is_none());
    s.finish(&one.id, one.generation, "succeeded", "done")
        .unwrap();
    assert_eq!(s.claim_shell(4).unwrap().unwrap().id, second.id);
    assert_eq!(s.get::<Task>("task", &agent.id).unwrap().state, "running");
    s.finish(&agent.id, agent.generation, "succeeded", "done")
        .unwrap();
    let next = s.submit("x", "chat", "next agent", None, None).unwrap();
    assert_eq!(s.claim_agent(1).unwrap().unwrap().id, next.id);
}
#[test]
fn pending_response_wakes_waiter_without_consuming_worker() {
    let (_d, s) = setup();
    let parent = s.submit("x", "chat", "analyze", None, None).unwrap();
    let running = s.claim(20).unwrap().unwrap();
    let child = s.query(&parent.id, "y", "question", None, "key").unwrap();
    s.end_turn(&running, "succeeded", "waiting").unwrap();
    assert_eq!(
        s.get::<Task>("task", &parent.id).unwrap().state,
        "waiting_dependency"
    );
    let r = s.claim(20).unwrap().unwrap();
    assert_eq!(r.id, child.id);
    s.finish(&r.id, r.generation, "succeeded", "answer")
        .unwrap();
    assert_eq!(s.get::<Task>("task", &parent.id).unwrap().state, "queued");
    assert!(s.inputs(&parent.id).unwrap()[0].1.contains("answer"));
}
#[test]
fn input_steers_existing_task_and_deduplicates() {
    let (_d, s) = setup();
    let a = s.submit("x", "chat", "first", None, Some("e1")).unwrap();
    let b = s
        .submit("x", "chat", "correction", None, Some("e2"))
        .unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(s.inputs(&a.id).unwrap()[0].1, "correction");
    assert!(s.submit("x", "chat", "repeat", None, Some("e2")).is_err());
}
#[test]
fn collaboration_is_config_only_reuses_context_and_fences_scope() {
    let (_d, s) = setup();
    let parent = s
        .submit("x", "chat", "private history", None, None)
        .unwrap();
    let q = s.query(&parent.id, "y", "question", None, "key").unwrap();
    let session: Session = s.get("session", &q.session_id).unwrap();
    assert_eq!(session.kind, "collaboration");
    assert!(session.thread_id.is_none());
    assert_eq!(q.prompt, "question");
    assert_eq!(q.bot_id, "y");
    assert_eq!(
        s.query(&parent.id, "y", "question", None, "key")
            .unwrap()
            .id,
        q.id
    );
    s.update_agent("y", |a| {
        a.config.system_prompt = "new role".into();
        Ok(())
    })
    .unwrap();
    let follow = s
        .query(&parent.id, "y", "followup", Some(&session.id), "key2")
        .unwrap();
    assert_eq!(follow.session_id, session.id);
    assert_eq!(
        s.get::<Session>("session", &session.id)
            .unwrap()
            .config
            .system_prompt,
        "original"
    );
    let other = s
        .submit("x", "different-chat", "other", None, None)
        .unwrap();
    assert!(s
        .query(&other.id, "y", "steal", Some(&session.id), "k")
        .is_err());
}
#[test]
fn claims_serialize_same_session_and_allow_other_sessions() {
    let (_d, s) = setup();
    let parent = s.submit("x", "chat", "a", None, None).unwrap();
    let q = s.query(&parent.id, "y", "b", None, "1").unwrap();
    let _follow = s
        .query(&parent.id, "y", "c", Some(&q.session_id), "2")
        .unwrap();
    assert!(s.claim(2).unwrap().is_some());
    assert!(s.claim(2).unwrap().is_some());
    assert!(s.claim(2).unwrap().is_none());
}
#[test]
fn terminal_event_is_fenced_and_delivery_is_atomic() {
    let (_d, s) = setup();
    let t = s
        .submit(
            "x",
            "chat",
            "hello",
            Some(ReplyTarget {
                chat_id: "dest".into(),
                context_token: None,
                thread_id: None,
            }),
            None,
        )
        .unwrap();
    let run = s.claim(20).unwrap().unwrap();
    assert!(!s
        .finish(&t.id, run.generation + 1, "succeeded", "bad")
        .unwrap());
    assert!(s
        .finish(&t.id, run.generation, "succeeded", "good")
        .unwrap());
    assert!(!s
        .finish(&t.id, run.generation, "succeeded", "again")
        .unwrap());
    let c = s.conn().unwrap();
    let n: i64 = c
        .query_row("SELECT count(*) FROM deliveries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}
#[test]
fn cron_is_frozen_and_skips_overlap_and_duplicate_ticks() {
    let (_d, s) = setup();
    let mut bot: Bot = s.get("bot", "x").unwrap();
    bot.channel = "lark".into();
    bot.owner = Some("ou_owner".into());
    s.save("bot", "x", &bot).unwrap();
    let mut j = s
        .job_create("x", "every 10s", "UTC", "scheduled", None)
        .unwrap();
    j.next_fire = Some(now() - 1);
    s.save("job", &j.id, &j).unwrap();
    s.update_agent("x", |a| {
        a.config.system_prompt = "changed".into();
        Ok(())
    })
    .unwrap();
    let at = now();
    assert_eq!(s.fire_due(at).unwrap(), 1);
    assert_eq!(s.fire_due(at).unwrap(), 0);
    let tasks = s.list::<Task>("task").unwrap();
    assert_eq!(tasks[0].reply.as_ref().unwrap().chat_id, "ou_owner");
    let sess: Session = s.get("session", &tasks[0].session_id).unwrap();
    assert_eq!(sess.config.system_prompt, "original");
    assert_eq!(s.fire_due(at + 20).unwrap(), 0);
    assert!(s
        .job_history(&j.id)
        .unwrap()
        .iter()
        .any(|v| v["status"] == "skipped_overlap"));
}
#[test]
fn cron_configuration_rejection_does_not_block_future_runs() {
    let (_d, s) = setup();
    let j = s
        .job_create("x", "every 10s", "UTC", "hello", None)
        .unwrap();
    let due = j.next_fire.unwrap();
    assert_eq!(s.fire_due(due).unwrap(), 1);
    let task = s.claim(1).unwrap().unwrap();
    let error: anyhow::Error = multi_agent_on_chat::application::execution::BeforeTurn(
        anyhow::anyhow!("arbitrary startup failure"),
    )
    .into();
    let kind = multi_agent_on_chat::application::execution::FailureKind::classify(&error);
    s.event(
        &task.id,
        "execution_error",
        &serde_json::json!({"failure_kind":kind}),
    )
    .unwrap();
    s.finish(&task.id, task.generation, kind.state(), "config failure")
        .unwrap();
    assert_eq!(s.fire_due(due + 11).unwrap(), 1);
    assert_eq!(s.get::<Task>("task", &task.id).unwrap().state, "failed");
}
#[test]
fn crash_recovery_does_not_automatically_replay() {
    let (_d, s) = setup();
    let t = s.submit("x", "chat", "write", None, None).unwrap();
    s.claim(20).unwrap();
    assert_eq!(s.recover().unwrap(), 1);
    assert_eq!(s.get::<Task>("task", &t.id).unwrap().state, "interrupted");
    assert!(s.retry(&t.id).is_err());
}
#[test]
fn draft_confirmation_is_idempotent() {
    let (_d, s) = setup();
    let a: Agent = s.get("agent", "x").unwrap();
    s.save("draft","d1",&serde_json::json!({"status":"pending","expires_at":now()+900,"agent":a,"schedule":"every 1h","timezone":"UTC","prompt":"do work"})).unwrap();
    assert_eq!(
        multi_agent_on_chat::service::confirm_draft(&s, "d1").unwrap()["created"],
        true
    );
    assert_eq!(
        multi_agent_on_chat::service::confirm_draft(&s, "d1").unwrap()["already_confirmed"],
        true
    );
    assert_eq!(s.list::<Job>("job").unwrap().len(), 1);
}

#[test]
fn ask_is_atomic_and_cancelled_query_wakes_requester() {
    let (_d, s) = setup();
    let q = s.ask("x", "chat", "y", "question", None).unwrap();
    let parent = q.parent_id.clone().unwrap();
    assert_eq!(
        s.get::<Task>("task", &parent).unwrap().state,
        "waiting_dependency"
    );
    s.cancel(&q.id).unwrap();
    assert_eq!(s.get::<Task>("task", &parent).unwrap().state, "queued");
    assert!(s.inputs(&parent).unwrap()[0].1.contains("cancelled"));
}
#[test]
fn user_guidance_wakes_waiting_parent_and_late_input_continues() {
    let (_d, s) = setup();
    let parent = s.submit("x", "chat", "original", None, None).unwrap();
    let running = s.claim(20).unwrap().unwrap();
    s.query(&parent.id, "y", "question", None, "one").unwrap();
    s.end_turn(&running, "succeeded", "waiting").unwrap();
    s.submit("x", "chat", "change direction", None, None)
        .unwrap();
    assert_eq!(s.get::<Task>("task", &parent.id).unwrap().state, "queued");
    let claimed = s.claim(20).unwrap().unwrap();
    assert_eq!(claimed.id, parent.id);
    s.end_turn(&claimed, "succeeded", "done").unwrap();
    assert_eq!(s.get::<Task>("task", &parent.id).unwrap().state, "queued");
    assert_eq!(s.inputs(&parent.id).unwrap().len(), 1);
}
#[test]
fn recovery_returns_interrupted_query_to_requester() {
    let (_d, s) = setup();
    let q = s.ask("x", "chat", "y", "question", None).unwrap();
    let running = s.claim(20).unwrap().unwrap();
    assert_eq!(running.id, q.id);
    s.recover().unwrap();
    assert_eq!(
        s.get::<Task>("task", q.parent_id.as_ref().unwrap())
            .unwrap()
            .state,
        "queued"
    );
    assert!(s.inputs(q.parent_id.as_ref().unwrap()).unwrap()[0]
        .1
        .contains("interrupted"));
}

#[test]
fn initialization_and_new_bot_defaults_are_idempotent_and_inherit_codex() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path()).unwrap();
    assert!(s.initialize_defaults().unwrap().is_empty());
    let t: Template = s.get("template", "default@1").unwrap();
    assert!(t.system_prompt.is_empty());
    assert!(t.profile.is_none() && t.model.is_none() && t.effort.is_none());
    let bot = Bot {
        id: "new-bot".into(),
        channel: "lark".into(),
        account_id: "cli_test".into(),
        credential_ref: None,
        enabled: false,
        owner: None,
    };
    let a = s.add_bot_with_defaults(&bot).unwrap();
    assert_eq!(a.id, "new-bot-main");
    assert_eq!(a.bot_id, bot.id);
    assert!(a.workspace.is_dir());
    assert!(a.enabled);
    assert!(s.initialize_defaults().unwrap().is_empty());
    assert_eq!(s.list::<Agent>("agent").unwrap().len(), 1);
    s.update_agent(&a.id, |a| {
        a.enabled = false;
        a.config.system_prompt = "custom".into();
        Ok(())
    })
    .unwrap();
    assert!(s.initialize_defaults().unwrap().is_empty());
    assert_eq!(
        s.get::<Agent>("agent", &a.id).unwrap().config.system_prompt,
        "custom"
    );
    assert!(s.add_bot_with_defaults(&bot).is_err());
}
#[test]
fn initialization_backfills_only_bots_without_any_agent() {
    let (_d, s) = setup();
    s.add_bot(&Bot {
        id: "legacy".into(),
        channel: "local".into(),
        account_id: "legacy".into(),
        credential_ref: None,
        enabled: true,
        owner: Some("local".into()),
    })
    .unwrap();
    let created = s.initialize_defaults().unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].bot_id, "legacy");
    assert_eq!(s.get::<Agent>("agent", "x").unwrap().config.id, "basic");
    assert!(s.initialize_defaults().unwrap().is_empty());
}
