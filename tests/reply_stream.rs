use multi_agent_on_chat::{
    domain::*,
    reply_stream::{self, Collector},
    store::Store,
};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn shared_reply_pipeline_streams_deduplicates_recovers_and_falls_back() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path()).unwrap();
    let cli = d.path().join("lark-cli");
    std::fs::write(&cli,r#"#!/usr/bin/env python3
import sys,json,pathlib
root=pathlib.Path(__file__).parent
body=json.load(sys.stdin)
path=sys.argv[3]
with (root/'calls').open('a') as f:f.write(json.dumps({'method':sys.argv[2],'path':path,'body':body})+'\n')
if 'card' in body and (root/'layout-fail').exists():
 print(json.dumps({'ok':False,'error':{'code':999}}),file=sys.stderr);sys.exit(1)
if (root/'fail').exists():
 print(json.dumps({'ok':False,'error':{'code':999}}),file=sys.stderr);sys.exit(1)
if path.endswith('/content') and (root/'expire').exists():
 code=int((root/'expire').read_text())
 if not (root/'persistent').exists(): (root/'expire').unlink()
 print(json.dumps({'ok':False,'error':{'code':code}}),file=sys.stderr);sys.exit(1)
if path.endswith('/settings') and json.loads(body['settings'])['config'].get('streaming_mode') and (root/'lose-open').exists():
 (root/'lose-open').unlink(); print('response lost');sys.exit(1)
if path=='/open-apis/cardkit/v1/cards': result={'card_id':'card_'+str(sum(1 for _ in (root/'calls').open()))}
elif '/im/v1/messages' in path: result={'message_id':'om_card'}
else: result={}
print(json.dumps({'ok':True,'data':result}))
"#).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let original = std::env::var("PATH").unwrap();
    std::env::set_var("PATH", format!("{}:{original}", d.path().display()));
    let credentials = d.path().join("creds");
    std::fs::create_dir(&credentials).unwrap();
    let bot = Bot {
        id: "test".into(),
        channel: "lark".into(),
        account_id: "cli_stream_test".into(),
        credential_ref: Some(credentials),
        owner: Some("owner".into()),
        enabled: true,
    };
    s.save("bot", "test", &bot).unwrap();
    std::fs::write(d.path().join("reply-streaming.json"), r#"{"test":"card"}"#).unwrap();
    let mut task = Task {
        id: "t_stream".into(),
        session_id: "s".into(),
        agent_id: "a".into(),
        bot_id: "test".into(),
        prompt: "hello".into(),
        state: "running".into(),
        result: None,
        parent_id: None,
        generation: 1,
        created_at: now(),
        reply: Some(ReplyTarget {
            chat_id: "ou_owner".into(),
            context_token: None,
            thread_id: Some("om_original".into()),
        }),
        kind: "agent".into(),
    };
    s.save("task", &task.id, &task).unwrap();
    let mut c = Collector::new(&s, &task).unwrap();
    c.started("reason", Some("analysis"));
    c.delta("reason", "private reasoning");
    c.started("a", Some("final_answer"));
    c.delta("a", "Hello");
    c.flush(&s, &task).unwrap();
    assert_eq!(s.list::<Value>("reply_desired").unwrap().len(), 1);
    reply_stream::deliver(&s).await.unwrap();
    let mut state: Value = s.get("reply_transport", "t_stream:1:answer").unwrap();
    assert_eq!(state["sent"], "Hello");
    assert_eq!(
        reply_stream::final_delivery(&s, &task.id, "Hello").unwrap(),
        "wait"
    );
    c.message("a", "Hello world", Some("final_answer"));
    c.flush(&s, &task).unwrap();
    // Simulate a response lost after an update, and restart with its persisted operation.
    state["pending"] =
        json!({"text":"Hello world","close":false,"sequence":2,"uuid":"fixed-update"});
    state["next_at"] = json!(0);
    s.save("reply_transport", "t_stream:1:answer", &state)
        .unwrap();
    task.state = "succeeded".into();
    task.result = Some("Hello world".into());
    s.save("task", &task.id, &task).unwrap();
    let reopened = Store::open(d.path()).unwrap();
    reply_stream::deliver(&reopened).await.unwrap();
    assert_eq!(
        reply_stream::final_delivery(&s, &task.id, "Hello world").unwrap(),
        "sent"
    );
    assert_eq!(
        reply_stream::final_delivery(&s, &task.id, "different").unwrap(),
        "send"
    );
    let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
    assert!(!calls.contains("private reasoning"));
    assert!(calls.contains("fixed-update"));
    assert!(calls.contains("om_original/reply"));
    let count = calls.lines().count();
    reply_stream::deliver(&s).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(d.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        count
    );

    // Old generations cannot overwrite the new reply projection.
    task.state = "running".into();
    task.generation = 2;
    s.save("task", &task.id, &task).unwrap();
    let mut current = Collector::new(&s, &task).unwrap();
    current.message("b", "new", Some("final_answer"));
    current.flush(&s, &task).unwrap();
    let mut stale = task.clone();
    stale.generation = 1;
    c.message("a", "stale", Some("final_answer"));
    c.flush(&s, &stale).unwrap();
    assert_eq!(
        s.get::<Value>("reply_desired", "t_stream:2:answer")
            .unwrap()["text"],
        "new"
    );
    std::fs::write(d.path().join("fail"), "").unwrap();
    task.state = "succeeded".into();
    task.result = Some("new".into());
    s.save("task", &task.id, &task).unwrap();
    for _ in 0..3 {
        reply_stream::deliver(&s).await.unwrap();
        let mut st: Value = s.get("reply_transport", "t_stream:2:answer").unwrap();
        st["next_at"] = json!(0);
        s.save("reply_transport", "t_stream:2:answer", &st).unwrap();
    }
    assert_eq!(
        reply_stream::final_delivery(&s, &task.id, "new").unwrap(),
        "send"
    );
    // A card accepted a prefix, then subsequent updates failed: send only the remainder.
    let key = "t_stream:2:answer";
    let mut partial: Value = s.get("reply_transport", key).unwrap();
    partial["message"] = json!("om_partial");
    partial["sent"] = json!("已完成第一步。\n\n");
    s.save("reply_transport", key, &partial).unwrap();
    let full = "已完成第一步。\n\n最终结果：成功。";
    assert_eq!(
        reply_stream::fallback_text(&s, &task.id, full).unwrap(),
        "最终结果：成功。"
    );
    assert_eq!(
        reply_stream::fallback_text(&s, &task.id, "修改后的完整结果").unwrap(),
        "修改后的完整结果"
    );
    // Closing a card can fail after all text was accepted; do not send the answer again.
    partial["sent"] = json!(full);
    s.save("reply_transport", key, &partial).unwrap();
    assert_eq!(
        reply_stream::final_delivery(&s, &task.id, full).unwrap(),
        "sent"
    );
    assert_eq!(reply_stream::fallback_text(&s, &task.id, full).unwrap(), "");
    // No acknowledged message means no text may be subtracted.
    partial["message"] = Value::Null;
    s.save("reply_transport", key, &partial).unwrap();
    assert_eq!(
        reply_stream::fallback_text(&s, &task.id, full).unwrap(),
        full
    );
    std::fs::remove_file(d.path().join("fail")).unwrap();
    task.generation = 3;
    task.reply.as_mut().unwrap().thread_id = None;
    task.state = "running".into();
    s.save("task", &task.id, &task).unwrap();
    let mut cancelled = Collector::new(&s, &task).unwrap();
    cancelled.started("c", Some("final_answer"));
    cancelled.delta("c", "Partial");
    cancelled.flush(&s, &task).unwrap();
    reply_stream::deliver(&s).await.unwrap();
    task.state = "cancelled".into();
    task.result = Some("cancelled".into());
    s.save("task", &task.id, &task).unwrap();
    let mut st: Value = s.get("reply_transport", "t_stream:3:answer").unwrap();
    st["next_at"] = json!(0);
    s.save("reply_transport", "t_stream:3:answer", &st).unwrap();
    reply_stream::deliver(&s).await.unwrap();
    let st: Value = s.get("reply_transport", "t_stream:3:answer").unwrap();
    assert_eq!(st["closed"], true);
    let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
    assert!(calls
        .lines()
        .any(|line| serde_json::from_str::<Value>(line).unwrap()["path"]
            == "/open-apis/im/v1/messages"));
    assert_eq!(st["sent"], "Partial"); // Cancellation closes without reopening; ordinary delivery carries cancellation.
    assert_eq!(
        reply_stream::final_delivery(&s, &task.id, "cancelled").unwrap(),
        "send"
    );

    // Explicit streaming expiry reopens the same card, with strictly increasing sequences.
    for (generation, code) in [(10, 200850), (11, 300309)] {
        task.generation = generation;
        task.state = "running".into();
        task.result = None;
        s.save("task", &task.id, &task).unwrap();
        let key = format!("t_stream:{generation}:answer");
        let mut c = Collector::new(&s, &task).unwrap();
        c.message("expiry", "恢复后的文本", Some("final_answer"));
        c.flush(&s, &task).unwrap();
        let before = std::fs::read_to_string(d.path().join("calls"))
            .unwrap()
            .lines()
            .count();
        std::fs::write(d.path().join("expire"), code.to_string()).unwrap();
        std::fs::write(d.path().join("lose-open"), "").unwrap();
        reply_stream::deliver(&s).await.unwrap();
        let mut checkpoint: Value = s.get("reply_transport", &key).unwrap();
        assert_eq!(checkpoint["pending"]["kind"], "open");
        assert_eq!(checkpoint["recovery_attempted"], true);
        let pending = checkpoint["pending"].clone();
        let card = checkpoint["card"].clone();
        let message = checkpoint["message"].clone();
        checkpoint["next_at"] = json!(0);
        s.save("reply_transport", &key, &checkpoint).unwrap();
        let restarted = Store::open(d.path()).unwrap();
        reply_stream::deliver(&restarted).await.unwrap();
        let checkpoint: Value = s.get("reply_transport", &key).unwrap();
        assert_eq!(checkpoint["sent"], "恢复后的文本");
        assert_eq!(checkpoint["card"], card);
        assert_eq!(checkpoint["message"], message);
        let calls: Vec<Value> = std::fs::read_to_string(d.path().join("calls"))
            .unwrap()
            .lines()
            .skip(before)
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            calls
                .iter()
                .filter(|v| v["path"] == "/open-apis/cardkit/v1/cards")
                .count(),
            1
        );
        let opens: Vec<_> = calls
            .iter()
            .filter(|v| {
                v["path"].as_str().unwrap().ends_with("/settings")
                    && serde_json::from_str::<Value>(v["body"]["settings"].as_str().unwrap())
                        .unwrap()["config"]["streaming_mode"]
                        == true
            })
            .collect();
        assert_eq!(opens.len(), 2);
        assert_eq!(opens[0]["body"], opens[1]["body"]);
        assert_eq!(opens[0]["body"]["uuid"], pending["uuid"]);
        let writes: Vec<_> = calls
            .iter()
            .filter(|v| v["path"].as_str().unwrap().ends_with("/content"))
            .collect();
        assert!(
            writes[0]["body"]["sequence"].as_u64().unwrap()
                < opens[0]["body"]["sequence"].as_u64().unwrap()
        );
        assert!(
            opens[0]["body"]["sequence"].as_u64().unwrap()
                < writes[1]["body"]["sequence"].as_u64().unwrap()
        );
    }
    // A provider that keeps rejecting updates must not reopen indefinitely.
    task.generation = 12;
    s.save("task", &task.id, &task).unwrap();
    let mut c = Collector::new(&s, &task).unwrap();
    c.message("repeat", "text", Some("final_answer"));
    c.flush(&s, &task).unwrap();
    let before = std::fs::read_to_string(d.path().join("calls"))
        .unwrap()
        .lines()
        .count();
    std::fs::write(d.path().join("expire"), "200850").unwrap();
    std::fs::write(d.path().join("persistent"), "").unwrap();
    for _ in 0..3 {
        reply_stream::deliver(&s).await.unwrap();
        let mut st: Value = s.get("reply_transport", "t_stream:12:answer").unwrap();
        st["next_at"] = json!(0);
        s.save("reply_transport", "t_stream:12:answer", &st)
            .unwrap();
    }
    let st: Value = s.get("reply_transport", "t_stream:12:answer").unwrap();
    assert_eq!(st["failed"], true);
    let opens = std::fs::read_to_string(d.path().join("calls"))
        .unwrap()
        .lines()
        .skip(before)
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| {
            v["body"]["settings"].as_str().is_some_and(|settings| {
                serde_json::from_str::<Value>(settings).unwrap()["config"]["streaming_mode"] == true
            })
        })
        .count();
    assert_eq!(opens, 1);
    std::fs::remove_file(d.path().join("expire")).unwrap();
    std::fs::remove_file(d.path().join("persistent")).unwrap();

    // Pending reopen after a crash must not execute after cancellation, a new generation, or off.
    for (generation, change) in [(20, "cancel"), (21, "generation"), (23, "off")] {
        std::fs::write(d.path().join("reply-streaming.json"), r#"{"test":"card"}"#).unwrap();
        task.generation = generation;
        task.state = "running".into();
        s.save("task", &task.id, &task).unwrap();
        let key = format!("t_stream:{generation}:answer");
        let mut c = Collector::new(&s, &task).unwrap();
        c.message("late", "latest", Some("final_answer"));
        c.flush(&s, &task).unwrap();
        std::fs::write(d.path().join("expire"), "300309").unwrap();
        std::fs::write(d.path().join("lose-open"), "").unwrap();
        reply_stream::deliver(&s).await.unwrap();
        let mut st: Value = s.get("reply_transport", &key).unwrap();
        assert_eq!(st["pending"]["kind"], "open");
        st["next_at"] = json!(0);
        s.save("reply_transport", &key, &st).unwrap();
        match change {
            "cancel" => task.state = "cancelled".into(),
            "generation" => task.generation += 1,
            _ => {
                std::fs::write(d.path().join("reply-streaming.json"), r#"{"test":"off"}"#).unwrap()
            }
        };
        s.save("task", &task.id, &task).unwrap();
        let before = std::fs::read_to_string(d.path().join("calls"))
            .unwrap()
            .lines()
            .count();
        reply_stream::deliver(&s).await.unwrap();
        let calls: Vec<Value> = std::fs::read_to_string(d.path().join("calls"))
            .unwrap()
            .lines()
            .skip(before)
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(!calls.is_empty());
        assert!(calls
            .iter()
            .all(|v| v["path"].as_str().unwrap().ends_with("/settings")
                && serde_json::from_str::<Value>(v["body"]["settings"].as_str().unwrap())
                    .unwrap()["config"]["streaming_mode"]
                    == false));
    }

    // Buffered task text must not create a late card after explicit cancellation.
    task.id = "cancelled_without_card".into();
    task.state = "running".into();
    task.generation = 1;
    s.save("task", &task.id, &task).unwrap();
    std::fs::write(d.path().join("reply-streaming.json"), r#"{"test":"card"}"#).unwrap();
    let mut cancelled = reply_stream::Collector::new(&s, &task).unwrap();
    cancelled.message("answer", "partial answer", Some("final_answer"));
    cancelled.flush(&s, &task).unwrap();
    s.save(
        "command_task_suppression",
        &task.id,
        &json!({"operation":"cancel"}),
    )
    .unwrap();
    task.state = "cancelled".into();
    s.save("task", &task.id, &task).unwrap();
    let before = std::fs::read_to_string(d.path().join("calls")).unwrap();
    reply_stream::deliver(&s).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(d.path().join("calls")).unwrap(),
        before
    );
    task.id = "wechat_progress".into();
    task.state = "running".into();
    s.save("task", &task.id, &task).unwrap();
    // Closing the stream precedes cosmetic folding; retry exactly the same
    // operation after restart without another message or fallback text.
    task.id = "long_fold".into();
    task.state = "running".into();
    task.generation = 1;
    let long_text = "完整正文".repeat(220);
    s.save("task", &task.id, &task).unwrap();
    let mut long = reply_stream::Collector::new(&s, &task).unwrap();
    long.message("final", &long_text, Some("final_answer"));
    long.flush(&s, &task).unwrap();
    reply_stream::deliver(&s).await.unwrap();
    let transport: Value = s.get("reply_transport", "long_fold:1:answer").unwrap();
    assert!(!transport["closed"].as_bool().unwrap());
    task.state = "succeeded".into();
    task.result = Some(long_text.clone());
    s.save("task", &task.id, &task).unwrap();
    let mut transport = transport;
    transport["next_at"] = json!(0);
    s.save("reply_transport", "long_fold:1:answer", &transport)
        .unwrap();
    std::fs::write(d.path().join("layout-fail"), "1").unwrap();
    reply_stream::deliver(&s).await.unwrap();
    let mut transport: Value = s.get("reply_transport", "long_fold:1:answer").unwrap();
    assert_eq!(transport["closed"], true);
    assert_eq!(transport["layout_pending"], true);
    assert_eq!(transport["failed"], false);
    assert_eq!(
        reply_stream::final_delivery(&s, &task.id, &long_text).unwrap(),
        "sent"
    );
    let pending = transport["pending"].clone();
    std::fs::remove_file(d.path().join("layout-fail")).unwrap();
    transport["next_at"] = json!(0);
    s.save("reply_transport", "long_fold:1:answer", &transport)
        .unwrap();
    reply_stream::deliver(&Store::open(d.path()).unwrap())
        .await
        .unwrap();
    let finished: Value = s.get("reply_transport", "long_fold:1:answer").unwrap();
    assert_eq!(finished["layout_pending"], false);
    assert_eq!(finished["sent"], long_text);
    let calls: Vec<Value> = std::fs::read_to_string(d.path().join("calls"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let layouts: Vec<&Value> = calls
        .iter()
        .filter(|v| v["body"]["uuid"] == pending["uuid"])
        .collect();
    assert_eq!(layouts.len(), 2);
    assert_eq!(layouts[0]["body"], layouts[1]["body"]);
    let card: Value =
        serde_json::from_str(layouts[1]["body"]["card"]["data"].as_str().unwrap()).unwrap();
    assert_eq!(card["body"]["elements"].as_array().unwrap().len(), 1);
    assert_eq!(card["body"]["elements"][0]["expanded"], false);
    task.id = "wechat_final_progress".into();
    task.state = "running".into();
    task.result = None;
    s.save("task", &task.id, &task).unwrap();
    // Same collector, plain progress mode: no deltas, one delivery per completed item.
    let mut wx = bot.clone();
    wx.channel = "wechat".into();
    s.save("bot", "test", &wx).unwrap();
    std::fs::write(
        d.path().join("reply-streaming.json"),
        r#"{"test":"progress"}"#,
    )
    .unwrap();
    task.state = "running".into();
    task.generation = 4;
    s.save("task", &task.id, &task).unwrap();
    let mut c = Collector::new(&s, &task).unwrap();
    c.started("p", Some("commentary"));
    c.delta("p", "working");
    c.flush(&s, &task).unwrap();
    assert!(s.deliveries().unwrap().is_empty());
    for _ in 0..2 {
        c.message("p", "working", Some("commentary"));
        c.flush(&s, &task).unwrap();
    }
    assert_eq!(s.deliveries().unwrap().len(), 1);
    std::env::set_var("PATH", original);
}
