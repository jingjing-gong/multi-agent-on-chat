use multi_agent_on_chat::{
    application::{media as app, Scope},
    channels,
    domain::*,
    media,
    store::Store,
};
use serde_json::json;
fn fixture(root: &std::path::Path) -> Store {
    let s = Store::open(root).unwrap();
    s.register_template(&serde_json::from_value(json!({"id":"default"})).unwrap())
        .unwrap();
    s.add_bot_with_defaults(&Bot {
        id: "one".into(),
        channel: "local".into(),
        account_id: "one".into(),
        credential_ref: None,
        enabled: true,
        owner: Some("local".into()),
    })
    .unwrap();
    s
}
#[test]
fn flat_lark_media_survives_without_resource_keys() {
    for kind in ["image", "file", "post", "audio", "media"] {
        let event = json!({"type":"im.message.receive_v1","sender_type":"user","sender_id":"owner","message_id":"om_test","chat_id":"oc_test","chat_type":"p2p","message_type":kind,"content":"[rendered attachment]"});
        let parsed = channels::parse_lark(&event).expect("flat media must not disappear");
        assert_eq!(
            parsed.attachments[0].source["hydrate_message_id"],
            "om_test"
        );
    }
}
#[test]
fn original_post_preserves_text_and_image_positions() {
    let content = json!({"zh_cn":{"title":"title","content":[[{"tag":"text","text":"before"},{"tag":"img","image_key":"first"},{"tag":"text","text":"between"},{"tag":"img","image_key":"second"},{"tag":"text","text":"after"}]]}});
    let event = json!({"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"owner"}},"message":{"message_id":"om_test","chat_id":"oc_test","chat_type":"p2p","message_type":"post","content":content.to_string()}}});
    let parsed = channels::parse_lark(&event).unwrap();
    assert!(parsed.text.contains("before[附件 1]between[附件 2]after"));
    assert_eq!(parsed.attachments[0].source["key"], "first");
    assert_eq!(parsed.attachments[1].source["key"], "second");
}
#[test]
fn images_are_decoded_and_qr_cannot_enter_model_input() {
    let d = tempfile::tempdir().unwrap();
    let s = fixture(d.path());
    let p = d.path().join("img.png");
    std::fs::write(&p, [137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
    assert!(media::import(&s, "one", "chat", &p, "attachment", None).is_err());
    image::RgbImage::new(2, 2).save(&p).unwrap();
    let a = media::import(&s, "one", "chat", &p, "auth_qr", Some(now() + 100)).unwrap();
    assert!(s
        .submit_media("one-main", "chat", "qr", None, &[a.id], None)
        .is_err());
    let a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    let task = s
        .submit_media(
            "one-main",
            "chat",
            "read",
            None,
            std::slice::from_ref(&a.id),
            None,
        )
        .unwrap();
    assert_eq!(media::input(&s, &task).unwrap().1, vec![a.path]);
}
#[test]
fn gc_keeps_active_tasks_and_expires_terminal_references() {
    let d = tempfile::tempdir().unwrap();
    let s = fixture(d.path());
    let p = d.path().join("data.txt");
    std::fs::write(&p, "hello").unwrap();
    let mut a = media::import(&s, "one", "chat", &p, "attachment", None).unwrap();
    a.created = now() - 8 * 86400;
    s.conn()
        .unwrap()
        .execute(
            "UPDATE media_assets SET data=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&a).unwrap(), a.id],
        )
        .unwrap();
    let t = s
        .submit_media(
            "one-main",
            "chat",
            "read",
            None,
            std::slice::from_ref(&a.id),
            None,
        )
        .unwrap();
    assert_eq!(
        app::gc(&s, &Scope::Local, false).unwrap()["assets"],
        json!([])
    );
    s.cancel(&t.id).unwrap();
    app::gc(&s, &Scope::Local, false).unwrap();
    s.save("media_unreferenced", &a.id, &(now() - 8 * 86400))
        .unwrap();
    assert_eq!(
        app::gc(&s, &Scope::Local, true).unwrap()["assets"],
        json!([a.id])
    );
    assert!(a.path.exists());
    app::gc(&s, &Scope::Local, false).unwrap();
    assert!(!a.path.exists());
    assert_eq!(media::get(&s, &a.id).unwrap().state, "expired");
}
#[test]
fn clear_fences_preparing_inbox_and_retains_receipt() {
    let d = tempfile::tempdir().unwrap();
    let s = fixture(d.path());
    let bot = s.get("bot", "one").unwrap();
    let m = channels::Incoming {
        untranscribed_voice: 0,
        event_id: "event".into(),
        is_private: true,
        sender: "local".into(),
        chat: "chat".into(),
        text: "read".into(),
        attachments: vec![],
        reply: ReplyTarget {
            chat_id: "chat".into(),
            thread_id: None,
            context_token: None,
        },
    };
    let id = media::ingress::register(&s, &bot, "one-main", "chat", &m).unwrap();
    s.conn()
        .unwrap()
        .execute(
            "UPDATE media_inbox SET state='preparing',generation=1 WHERE id=?1",
            [&id],
        )
        .unwrap();
    s.clear("one-main", "chat").unwrap();
    assert!(s
        .submit_media("one-main", "chat", "late", None, &[], Some((&id, 1)))
        .is_err());
    assert_eq!(
        media::ingress::register(&s, &bot, "one-main", "chat", &m).unwrap(),
        "duplicate"
    );
}
#[tokio::test]
async fn flat_event_downloads_original_resource_and_queues_exactly_one_task() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let s = fixture(d.path());
    let bin = d.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let cli = bin.join("lark-cli");
    std::fs::write(&cli,r#"#!/usr/bin/python3
import sys,json,pathlib
args=sys.argv[1:]
if args[:2]==['api','GET']:
 print(json.dumps({'data':{'items':[{'message_id':'om_test','chat_id':'oc_test','body':{'content':json.dumps({'file_key':'resource_test','file_name':'report.txt'})}}]}}))
elif args[:2]==['im','+messages-resources-download']:
 assert args[args.index('--file-key')+1]=='resource_test'
 pathlib.Path(args[args.index('--output')+1]).write_text('known attachment text')
 print('{}')
else:
 sys.exit(2)
"#).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let old_path = std::env::var_os("PATH").unwrap();
    std::env::set_var(
        "PATH",
        format!("{}:{}", bin.display(), old_path.to_string_lossy()),
    );
    let credentials = d.path().join("creds");
    std::fs::create_dir(&credentials).unwrap();
    let mut bot: Bot = s.get("bot", "one").unwrap();
    bot.channel = "lark".into();
    bot.credential_ref = Some(credentials);
    s.save("bot", "one", &bot).unwrap();
    let event = json!({"type":"im.message.receive_v1","sender_type":"user","sender_id":"local","message_id":"om_test","chat_id":"oc_test","chat_type":"p2p","message_type":"file","content":"[file]"});
    let m = channels::parse_lark(&event).unwrap();
    media::ingress::register(&s, &bot, "one-main", "chat", &m).unwrap();
    let result = media::ingress::work_once(&s).await;
    std::env::set_var("PATH", old_path);
    assert!(result.unwrap());
    let tasks = s.list::<Task>("task").unwrap();
    assert_eq!(tasks.len(), 1);
    let files = media::task_assets(&s, &tasks[0].id).unwrap();
    assert_eq!(files[0].name, "report.txt");
    assert!(media::input(&s, &tasks[0])
        .unwrap()
        .0
        .contains("known attachment text"));
    assert!(!media::ingress::work_once(&s).await.unwrap());
    assert_eq!(
        media::ingress::register(&s, &bot, "one-main", "chat", &m).unwrap(),
        "duplicate"
    );
}
