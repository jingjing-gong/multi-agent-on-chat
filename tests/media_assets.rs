use multi_agent_on_chat::{
    application::{media as app, Scope},
    domain::*,
    media,
    store::Store,
};
use std::path::Path;
fn fixture(root: &Path) -> Store {
    let s = Store::open(root).unwrap();
    let t: Template = serde_json::from_value(serde_json::json!({"id":"default"})).unwrap();
    s.register_template(&t).unwrap();
    for id in ["one", "two"] {
        s.add_bot_with_defaults(&Bot {
            id: id.into(),
            channel: "local".into(),
            account_id: id.into(),
            credential_ref: None,
            enabled: true,
            owner: Some("local".into()),
        })
        .unwrap();
    }
    s
}
#[test]
fn snapshots_scope_and_delivery_references_survive_source_changes() {
    let d = tempfile::tempdir().unwrap();
    let s = fixture(d.path());
    let input = d.path().join("notes.txt");
    std::fs::write(&input, b"hello").unwrap();
    let a = media::import(&s, "one", "chat-a", &input, "attachment", None).unwrap();
    std::fs::write(&input, b"changed").unwrap();
    assert_eq!(std::fs::read(&a.path).unwrap(), b"hello");
    assert!(media::check(&a, "two", None).is_err());
    assert!(media::check(&a, "one", Some("chat-b")).is_err());
    assert!(app::show(&s, &Scope::Bot("two".into()), &a.id).is_err());
    let reply = ReplyTarget {
        chat_id: "chat-a".into(),
        thread_id: None,
        context_token: None,
    };
    let first = media::enqueue(&s, "one", &reply, &a, "same-send").unwrap();
    assert_eq!(
        first,
        media::enqueue(&s, "one", &reply, &a, "same-send").unwrap()
    );
    assert_eq!(
        s.conn()
            .unwrap()
            .query_row("SELECT count(*) FROM deliveries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn submit_keeps_attachments_and_does_not_turn_them_into_text_steering() {
    let d = tempfile::tempdir().unwrap();
    let s = fixture(d.path());
    let input = d.path().join("notes.txt");
    std::fs::write(&input, b"hello").unwrap();
    let a = media::import(&s, "one", "chat", &input, "attachment", None).unwrap();
    let old = s.submit("one-main", "chat", "first", None, None).unwrap();
    let t = s
        .submit_media(
            "one-main",
            "chat",
            "",
            None,
            std::slice::from_ref(&a.id),
            None,
        )
        .unwrap();
    assert_ne!(old.id, t.id);
    assert_eq!(media::task_assets(&s, &t.id).unwrap()[0].id, a.id);
    assert!(media::input(&s, &t).unwrap().0.contains("hello"));
    assert!(s
        .submit_media("two-main", "chat", "x", None, &[a.id], None)
        .is_err());
}
#[test]
fn expiry_and_limits_fail_explicitly() {
    let d = tempfile::tempdir().unwrap();
    let s = fixture(d.path());
    let input = d.path().join("notes.txt");
    std::fs::write(&input, b"hello").unwrap();
    let a = media::import(&s, "one", "chat", &input, "auth_qr", Some(now() - 1)).unwrap();
    assert!(media::check(&a, "one", None).is_err());
    std::fs::write(d.path().join("media-limits.json"), r#"{"file_bytes":3}"#).unwrap();
    assert!(media::import(&s, "one", "chat", &input, "attachment", None)
        .unwrap_err()
        .to_string()
        .contains("size_exceeded"));
    let count = s
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM media_assets", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}
