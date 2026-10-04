//! iLink text messages support a Markdown subset in current WeChat clients.
//! Keep Markdown intact; serialize common rich-text spans at this boundary.
use crate::message::Message;
fn inline_code(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    format!("{fence} {text} {fence}")
}
fn emphasis(text: &str) -> String {
    // Keep delimiters away from whitespace and syntax-heavy command arguments.
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return text.into();
    }
    if trimmed.contains(['*', '`', '\n']) {
        return inline_code(text);
    }
    format!(
        "{}**{}**{}",
        &text[..text.len() - text.trim_start().len()],
        trimmed,
        &text[text.trim_end().len()..]
    )
}
pub fn request(
    message: &Message,
    target: &crate::domain::ReplyTarget,
    key: &str,
) -> serde_json::Value {
    serde_json::json!({"msg":{"from_user_id":"","to_user_id":target.chat_id,"client_id":key,"message_type":2,"message_state":2,"context_token":target.context_token,"item_list":[{"type":1,"text_item":{"text":render(message)}}]},"base_info":{"channel_version":"0.1.0"}})
}
/// Split raw bytes at UTF-8 boundaries without parsing JSON.
pub fn requests(
    message: &Message,
    target: &crate::domain::ReplyTarget,
    key: &str,
) -> Vec<serde_json::Value> {
    const CHUNK_BYTES: usize = 3000;
    let Message::Plain(text) = message else {
        return vec![request(message, target, key)];
    };
    if text.len() <= CHUNK_BYTES {
        return vec![request(message, target, key)];
    }
    let mut remaining = text.as_str();
    let mut parts = Vec::new();
    while !remaining.is_empty() {
        let mut end = remaining.len().min(CHUNK_BYTES);
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        if end < remaining.len() {
            if let Some(newline) = remaining[..end].rfind('\n') {
                end = newline + 1;
            }
        }
        parts.push(request(
            &Message::Plain(remaining[..end].into()),
            target,
            &format!("{key}-{}", parts.len() + 1),
        ));
        remaining = &remaining[end..];
    }
    parts
}
pub fn render(message: &Message) -> String {
    match message {
        Message::File(_) | Message::Media { .. } | Message::Bundle(_) => message.plain_text(),
        // Plain payloads (including -r JSON) must remain literal in WeChat's
        // Markdown renderer. Never parse or reconstruct the payload here.
        Message::Plain(text) => {
            let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat((longest + 1).max(3));
            format!("{fence}\n{text}\n{fence}")
        }
        Message::Markdown(text) => text.clone(),
        Message::RichText(rows) => {
            let mut output = Vec::new();
            let mut command_description = false;
            for row in rows {
                let text = row.iter().map(|s| s.text.as_str()).collect::<String>();
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    output.push(String::new());
                    command_description = false;
                    continue;
                }
                if trimmed.starts_with('/') || trimmed.starts_with("- /") {
                    let command = trimmed.strip_prefix("- ").unwrap_or(trimmed);
                    if let Some((usage, description)) = command.split_once(" — ") {
                        output.push(format!("- {}\n  {}", inline_code(usage), description));
                        command_description = false;
                    } else {
                        output.push(format!("- {}", inline_code(command)));
                        command_description = true;
                    }
                    continue;
                }
                let rendered = row
                    .iter()
                    .map(|span| {
                        if span.bold {
                            emphasis(&span.text)
                        } else {
                            span.text.clone()
                        }
                    })
                    .collect::<String>();
                if command_description {
                    output.push(format!("  {rendered}"));
                    command_description = false;
                } else {
                    output.push(rendered);
                }
            }
            output.join("\n")
        }
    }
}
