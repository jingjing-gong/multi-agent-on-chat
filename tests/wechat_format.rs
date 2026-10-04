use multi_agent_on_chat::{
    channels::wechat_format::render,
    message::{Message, Span},
};
#[test]
fn preserves_native_markdown_instead_of_stripping_it() {
    let markdown="# Heading\n\n**粗体** and *italic*\n\n- list\n\n| A | B |\n| --- | --- |\n| a | b |\n\n```rust\nlet x = 1;\n```\n\n`code` [link](https://example.com)";
    assert_eq!(render(&Message::Markdown(markdown.into())), markdown);
}
#[test]
fn serializes_rich_text_as_native_bold_and_keeps_raw_unchanged() {
    let msg = Message::RichText(vec![
        vec![Span {
            text: "/profile [list | name]".into(),
            bold: true,
        }],
        vec![Span {
            text: "说明".into(),
            bold: false,
        }],
    ]);
    let before = serde_json::to_string(&msg).unwrap();
    assert_eq!(render(&msg), "- ` /profile [list | name] `\n  说明");
    assert_eq!(serde_json::to_string(&msg).unwrap(), before);
    let raw = " {\"text\":\"**literal**\"}\n";
    assert_eq!(
        render(&Message::Plain(raw.into())),
        format!("```\n{raw}\n```")
    );
}
#[test]
fn bold_whitespace_and_complex_usage_do_not_break_markdown() {
    let msg = Message::RichText(vec![
        vec![Span {
            text: " 标题 ".into(),
            bold: true,
        }],
        vec![Span {
            text: "/sh echo `hello`".into(),
            bold: true,
        }],
        vec![Span {
            text: "运行说明".into(),
            bold: false,
        }],
    ]);
    let text = render(&msg);
    assert!(text.starts_with(" **标题** ") && !text.contains("** 标题 **"));
    assert!(text.contains("- `` /sh echo `hello` ``\n  运行说明"));
    let help =
        multi_agent_on_chat::presentation::message("/help", &multi_agent_on_chat::help::catalog());
    let rendered = render(&help);
    assert!(rendered.contains("- ` /profile [list | 名称 | default] `\n  查看"));
    assert!(!rendered.contains("**/") && !rendered.contains("\\["));
}
