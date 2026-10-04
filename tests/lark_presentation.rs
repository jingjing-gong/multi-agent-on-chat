use multi_agent_on_chat::channels::lark_markdown;
#[test]
fn help_uses_literal_rich_text_without_markdown_math_escapes() {
    let catalog = multi_agent_on_chat::help::catalog();
    let text = multi_agent_on_chat::presentation::message("/help", &catalog);
    let post = multi_agent_on_chat::channels::lark_post_content(&text);
    let rows = post["zh_cn"]["content"].as_array().unwrap();
    let entries = catalog["commands"].as_array().unwrap();
    assert_eq!(rows.len(), entries.len() * 3 - 1);
    for (index, entry) in entries.iter().enumerate() {
        let usage = &rows[index * 3][0];
        assert_eq!(usage["tag"], "text");
        assert_eq!(usage["style"][0], "bold");
        assert_eq!(usage["text"], entry["usage"]);
        assert_eq!(rows[index * 3 + 1][0]["text"], entry["description"]);
    }
    assert!(!post.to_string().contains("\\\\["));
    let normal = multi_agent_on_chat::channels::lark_post_content(
        &multi_agent_on_chat::message::Message::Markdown("**Normal**".into()),
    );
    assert_eq!(normal["zh_cn"]["content"][0][0]["tag"], "md");
}
#[test]
fn tables_become_labelled_records_and_keep_escaped_pipes() {
    let text = "**Bots**\n\n| Bot | Note |\n| --- | :---: |\n| alpha | a\\|b |\n| beta | ready |";
    let rendered = lark_markdown(text);
    assert!(rendered.contains("- **Bot**：alpha"));
    assert!(rendered.contains("- **Note**：a\\|b"));
    assert!(rendered.contains("- **Bot**：beta"));
    assert!(!rendered.contains("| ---"));
}
#[test]
fn code_blocks_and_non_tables_remain_literal() {
    for text in [
        "**Profile**\n\n- Name：default",
        "```text\n| A |\n| --- |\n| B |\n```",
        "~~~\n| A |\n| --- |\n| B |\n~~~",
        "a | b",
    ] {
        assert_eq!(lark_markdown(text), text);
    }
}
