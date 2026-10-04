use multi_agent_on_chat::{
    message::{Message, Span},
    presentation,
};
#[test]
fn help_content_is_channel_independent_and_durable() {
    let catalog = multi_agent_on_chat::help::catalog();
    let message = presentation::message("/help", &catalog);
    assert!(matches!(message, Message::RichText(_)));
    let wire = serde_json::to_string(&message).unwrap();
    assert_eq!(serde_json::from_str::<Message>(&wire).unwrap(), message);
    let plain = message.plain_text();
    assert!(plain.starts_with("/help [命令名]\n"));
    assert!(plain.contains("/profile [list | 名称 | default]"));
    assert!(!plain.contains("**"));
    assert!(matches!(
        presentation::message("/help -r", &catalog),
        Message::Plain(_)
    ));
}
#[test]
fn adapter_handles_arbitrary_rich_text_without_command_knowledge() {
    let message = Message::RichText(vec![
        vec![Span {
            text: "arbitrary [x] $y$".into(),
            bold: true,
        }],
        vec![Span {
            text: "description".into(),
            bold: false,
        }],
    ]);
    let post = multi_agent_on_chat::channels::lark_post_content(&message);
    assert_eq!(post["zh_cn"]["content"][0][0]["text"], "arbitrary [x] $y$");
    assert_eq!(message.plain_text(), "arbitrary [x] $y$\ndescription");
    let source = include_str!("../src/channels.rs");
    assert!(!source.contains("crate::help") && !source.contains("crate::presentation"));
    assert!(matches!(Message::legacy("{}"), Message::Plain(_)));
    assert!(matches!(
        Message::legacy("old message"),
        Message::Markdown(_)
    ));
}
#[test]
fn command_markup_decodes_literals_without_injecting_styles() {
    let msg =
        Message::command_text("**Title**\n\n- &lt;name&gt; \\[id\\] \\*\\*literal\\*\\* &amp;lt;");
    assert_eq!(msg.plain_text(), "Title\n\n- <name> [id] **literal** &lt;");
    let Message::RichText(rows) = msg else {
        panic!()
    };
    assert!(rows[0][0].bold);
    assert!(rows[2].iter().all(|span| !span.bold));
}
