use multi_agent_on_chat::{
    application::media as app, domain::*, media, message::Message, store::Store,
};

fn fixture(root: &std::path::Path) -> (Store, Task, std::path::PathBuf) {
    let store = Store::open(root).unwrap();
    let template: Template = serde_json::from_value(serde_json::json!({"id":"default"})).unwrap();
    store.register_template(&template).unwrap();
    store
        .add_bot_with_defaults(&Bot {
            id: "test".into(),
            channel: "local".into(),
            account_id: "test".into(),
            credential_ref: None,
            enabled: true,
            owner: Some("local".into()),
        })
        .unwrap();
    let task = store
        .submit(
            "test-main",
            "chat",
            "publish a file",
            Some(ReplyTarget {
                chat_id: "chat".into(),
                thread_id: None,
                context_token: None,
            }),
            None,
        )
        .unwrap();
    let task = store.claim(1).unwrap().filter(|t| t.id == task.id).unwrap();
    let session: Session = store.get("session", &task.session_id).unwrap();
    std::fs::create_dir_all(&session.workspace).unwrap();
    let path = session.workspace.join("report.txt");
    std::fs::write(&path, "report content").unwrap();
    (store, task, path)
}

#[test]
fn publication_is_idempotent_and_delivers_caption_with_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (store, task, path) = fixture(dir.path());
    let first = app::publish(&store, &task, &path, Some("Your report")).unwrap();
    let again = app::publish(&store, &task, &path, Some("Your report")).unwrap();
    assert_eq!(first, again);
    assert_eq!(media::list(&store, Some("test"), None).unwrap().len(), 1);
    let delivery = first["delivery_id"].as_str().unwrap();
    let payload: Message = store.get("delivery_payload", delivery).unwrap();
    let Message::Bundle(parts) = payload else {
        panic!("caption must be delivered");
    };
    assert_eq!(parts[0], Message::Plain("Your report".into()));
    let Message::Media { path: stored, .. } = &parts[1] else {
        panic!("missing artifact");
    };
    std::fs::write(&path, "changed").unwrap();
    assert_eq!(std::fs::read_to_string(stored).unwrap(), "report content");
    let newer = app::publish(&store, &task, &path, Some("Your report")).unwrap();
    assert_ne!(first["delivery_id"], newer["delivery_id"]);
}

#[test]
fn publication_rejects_stale_generation_and_outside_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let (store, task, path) = fixture(dir.path());
    let outside = tempfile::NamedTempFile::new().unwrap();
    assert!(app::publish(&store, &task, outside.path(), None).is_err());
    let mut stale = task.clone();
    stale.generation += 1;
    assert!(app::publish(&store, &stale, &path, None).is_err());
    let mut cancelled = task.clone();
    cancelled.state = "cancel_requested".into();
    store.save("task", &cancelled.id, &cancelled).unwrap();
    assert!(app::publish(&store, &task, &path, None).is_err());
    assert!(media::list(&store, None, None).unwrap().is_empty());
}
