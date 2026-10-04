use multi_agent_on_chat::{channels::wechat_format, domain::ReplyTarget, presentation};
use serde_json::json;
#[test]
fn long_raw_skills_are_split_without_losing_json_bytes() {
    use multi_agent_on_chat::message::Message;
    let target = ReplyTarget {
        chat_id: "owner".into(),
        context_token: Some("ctx".into()),
        thread_id: None,
    };
    let value = json!({"skills": (0..47).map(|n| json!({"name":format!("skill-{n}"),"description":"中文描述 **literal**".repeat(90)})).collect::<Vec<_>>()});
    let message = presentation::message("/skills -r", &value);
    let Message::Plain(raw) = &message else {
        panic!()
    };
    let parts = wechat_format::requests(&message, &target, "stable");
    assert!(parts.len() > 1);
    assert_eq!(parts, wechat_format::requests(&message, &target, "stable"));
    let mut joined = String::new();
    for (i, part) in parts.iter().enumerate() {
        let text = part["msg"]["item_list"][0]["text_item"]["text"]
            .as_str()
            .unwrap();
        assert!(text.len() <= 3010);
        joined.push_str(
            text.strip_prefix("```\n")
                .unwrap()
                .strip_suffix("\n```")
                .unwrap(),
        );
        assert_eq!(part["msg"]["client_id"], format!("stable-{}", i + 1));
        assert_eq!(part["msg"]["context_token"], "ctx");
    }
    assert_eq!(&joined, raw);
}
#[test]
fn raw_payload_is_literal_in_wechat_request_without_json_parsing() {
    use multi_agent_on_chat::message::Message;
    let target = ReplyTarget {
        chat_id: "owner".into(),
        context_token: Some("context".into()),
        thread_id: None,
    };
    let value = json!({"text":"**literal** <name> &lt; $$ ```", "enabled":false});
    let message = presentation::message("/skills list -r", &value);
    let Message::Plain(raw) = &message else {
        panic!("raw command must stay plain")
    };
    let request = wechat_format::request(&message, &target, "raw");
    let text = request["msg"]["item_list"][0]["text_item"]["text"]
        .as_str()
        .unwrap();
    assert_eq!(text, format!("````\n{raw}\n````"));
    // Non-JSON plain payloads are also preserved, without any parse attempt.
    let invalid = "{not json: **literal**}";
    assert_eq!(
        wechat_format::render(&Message::Plain(invalid.into())),
        format!("```\n{invalid}\n```")
    );
}
#[test]
fn rich_command_formatting_reaches_the_outbound_request() {
    let target = ReplyTarget {
        chat_id: "owner".into(),
        context_token: Some("context".into()),
        thread_id: None,
    };
    let message = presentation::message(
        "/template help",
        &json!({"commands":["/template list — 列出模板","/template show <name@version> — 查看配置"]}),
    );
    let request = wechat_format::request(&message, &target, "idempotency");
    let text = request["msg"]["item_list"][0]["text_item"]["text"]
        .as_str()
        .unwrap();
    assert!(text.starts_with("**模板命令**"), "{text}");
    assert!(text.contains("- ` /template show <name@version> `\n  查看配置"));
    assert!(!text.contains("&lt;"));
    assert_eq!(request["msg"]["context_token"], "context");
    assert_eq!(request["msg"]["client_id"], "idempotency");
    let help = presentation::message("/help", &multi_agent_on_chat::help::catalog());
    assert!(wechat_format::render(&help).starts_with("- ` /help [命令名] `\n  "));
}
