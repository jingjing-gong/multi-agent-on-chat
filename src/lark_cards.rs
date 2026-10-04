//! Channel-only CardKit transport. No task state is mutated here.
use crate::{channels, domain::*};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

#[derive(Debug)]
pub struct ApiError {
    pub code: Option<i64>,
    pub method: String,
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "card {} failed (API code: {:?}; details withheld)",
            self.method, self.code
        )
    }
}
impl std::error::Error for ApiError {}
pub fn streaming_closed(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ApiError>()
        .is_some_and(|e| matches!(e.code, Some(200850 | 300309)))
}

async fn api(bot: &Bot, method: &str, path: &str, body: Value) -> Result<Value> {
    let mut cmd = channels::lark(bot)?;
    let (path, query) = path
        .split_once('?')
        .map_or((path, None), |(p, q)| (p, Some(q)));
    cmd.args(["api", method, path, "--as", "bot", "--data", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(query) = query {
        let value = query
            .strip_prefix("receive_id_type=")
            .context("invalid card query")?;
        cmd.args(["--params", &json!({"receive_id_type":value}).to_string()]);
    }
    let mut child = cmd.spawn()?;
    let mut input = child.stdin.take().context("missing card stdin")?;
    input.write_all(body.to_string().as_bytes()).await?;
    drop(input);
    let out = tokio::time::timeout(Duration::from_secs(12), child.wait_with_output())
        .await
        .map_err(|_| anyhow!("card request timed out"))??;
    decode_response(method, out.status.success(), &out.stdout, &out.stderr)
}

fn decode_response(method: &str, success: bool, stdout: &[u8], stderr: &[u8]) -> Result<Value> {
    let output = serde_json::from_slice::<Value>(stdout).ok();
    // CLI failures use stderr, successes use stdout. Only parse structured
    // envelopes; never expose arbitrary stderr or infer codes from free text.
    let error = serde_json::from_slice::<Value>(stderr).ok();
    let failure = |v: &&Value| v["ok"] == false || v["code"].as_i64().is_some_and(|n| n != 0);
    if let Some(value) = error
        .as_ref()
        .filter(failure)
        .or_else(|| output.as_ref().filter(failure))
    {
        let code = value
            .pointer("/error/code")
            .and_then(Value::as_i64)
            .or_else(|| value["code"].as_i64());
        return Err(ApiError {
            code,
            method: method.into(),
        }
        .into());
    }
    if !success {
        return Err(ApiError {
            code: None,
            method: method.into(),
        }
        .into());
    }
    output.context("invalid card response")
}

pub(crate) fn summary(text: &str) -> String {
    let plain = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = plain.chars();
    let mut result: String = chars.by_ref().take(100).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
fn card_json(text: &str) -> Value {
    let content = crate::lark_markdown::normalize(text);
    json!({"schema":"2.0","config":{"streaming_mode":true,"summary":{"content":summary(text)}},
        "body":{"elements":[{"tag":"markdown","element_id":"body","content":content}]}})
}
pub async fn create(bot: &Bot, text: &str) -> Result<String> {
    let card = card_json(text);
    let value = api(
        bot,
        "POST",
        "/open-apis/cardkit/v1/cards",
        json!({"type":"card_json","data":card.to_string()}),
    )
    .await?;
    Ok(value
        .pointer("/data/card_id")
        .and_then(Value::as_str)
        .context("missing card ID")?
        .into())
}
pub async fn send(bot: &Bot, target: &ReplyTarget, card: &str, uuid: &str) -> Result<String> {
    let content = json!({"type":"card","data":{"card_id":card}}).to_string();
    let (path, body) = if let Some(thread) = &target.thread_id {
        (
            format!("/open-apis/im/v1/messages/{thread}/reply"),
            json!({"msg_type":"interactive","content":content,"uuid":uuid}),
        )
    } else {
        let kind = if target.chat_id.starts_with("ou_") {
            "open_id"
        } else {
            "chat_id"
        };
        (
            format!("/open-apis/im/v1/messages?receive_id_type={kind}"),
            json!({"receive_id":target.chat_id,"msg_type":"interactive","content":content,"uuid":uuid}),
        )
    };
    let value = api(bot, "POST", &path, body).await?;
    Ok(value
        .pointer("/data/message_id")
        .and_then(Value::as_str)
        .context("missing card message ID")?
        .into())
}
pub async fn content(bot: &Bot, card: &str, text: &str, seq: u32, uuid: &str) -> Result<()> {
    api(
        bot,
        "PUT",
        &format!("/open-apis/cardkit/v1/cards/{card}/elements/body/content"),
        json!({"content":crate::lark_markdown::normalize(text),"sequence":seq,"uuid":uuid}),
    )
    .await?;
    Ok(())
}
pub async fn close(bot: &Bot, card: &str, seq: u32, uuid: &str) -> Result<()> {
    api(bot,"PATCH",&format!("/open-apis/cardkit/v1/cards/{card}/settings"),json!({"settings":json!({"config":{"streaming_mode":false}}).to_string(),"sequence":seq,"uuid":uuid})).await?;
    Ok(())
}
pub async fn open(bot: &Bot, card: &str, seq: u32, uuid: &str) -> Result<()> {
    api(bot,"PATCH",&format!("/open-apis/cardkit/v1/cards/{card}/settings"),json!({"settings":json!({"config":{"streaming_mode":true}}).to_string(),"sequence":seq,"uuid":uuid})).await?;
    Ok(())
}

pub async fn update_summary(bot: &Bot, card: &str, text: &str, seq: u32, uuid: &str) -> Result<()> {
    api(bot,"PATCH",&format!("/open-apis/cardkit/v1/cards/{card}/settings"),json!({"settings":json!({"config":{"summary":{"content":text}}}).to_string(),"sequence":seq,"uuid":uuid})).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn card_has_no_title_and_uses_real_preview() {
        let card = card_json("真实开头\n第二行");
        assert!(card.get("header").is_none());
        assert_eq!(card["config"]["summary"]["content"], "真实开头 第二行");
        assert_eq!(card["body"]["elements"][0]["content"], "真实开头\n第二行");
        assert_eq!(summary(&"中".repeat(101)), format!("{}…", "中".repeat(100)));
    }
    #[test]
    fn cli_stderr_errors_are_typed_without_exposing_content() {
        for code in [200850, 300309] {
            let stderr =
                json!({"ok":false,"error":{"code":code,"message":"private-token"}}).to_string();
            let error = decode_response("PUT", false, b"", stderr.as_bytes()).unwrap_err();
            assert!(streaming_closed(&error));
            assert!(!error.to_string().contains("private-token"));
            assert!(streaming_closed(
                &decode_response("PUT", false, stderr.as_bytes(), b"").unwrap_err()
            ));
        }
        assert!(decode_response(
            "POST",
            true,
            br#"{"ok":true,"data":{"card_id":"1"}}"#,
            b"warning"
        )
        .is_ok());
        let error = decode_response("PUT", false, b"", b"private-token 200850").unwrap_err();
        assert!(!streaming_closed(&error));
        assert!(!error.to_string().contains("private-token"));
        assert!(decode_response("POST", true, b"invalid", b"").is_err());
    }
}

pub(crate) fn should_fold(progress: bool, text: &str) -> bool {
    progress || text.chars().count() > 300 || text.lines().count() > 8
}
// Split only a complete standalone introductory paragraph. Code, tables,
// lists and cross-block constructs stay together inside the panel.
fn fold_sections(text: &str) -> (Option<&str>, &str) {
    let Some((intro, rest)) = text.split_once("\n\n") else {
        return (None, text);
    };
    let inline_balanced = intro.matches("**").count().is_multiple_of(2)
        && intro.matches(char::from(96)).count().is_multiple_of(2);
    let simple = intro.lines().all(|line| {
        let line = line.trim_start();
        !line.starts_with(['#', '|', '>', '-', '*', '+', '~', '<']) || line.starts_with("**")
    });
    if intro.trim().is_empty()
        || rest.trim().is_empty()
        || intro.chars().count() > 300
        || intro.contains(char::from(96))
        || intro.contains(['[', ']', '<', '>'])
        || !inline_balanced
        || !simple
        || intro.lines().any(|l| {
            l.trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
    {
        return (None, text);
    }
    (Some(intro), rest)
}
pub(crate) fn folded_card(progress: bool, text: &str) -> Value {
    let mut elements = Vec::new();
    let (body, label) = if progress {
        let (body, status) = text.rsplit_once("\n\n— ").unwrap_or((text, "已完成"));
        elements.push(json!({"tag":"markdown","content":status}));
        (body, "展开 / 折叠过程")
    } else {
        let (intro, rest) = fold_sections(text);
        if let Some(intro) = intro {
            elements
                .push(json!({"tag":"markdown","content":crate::lark_markdown::normalize(intro)}));
        }
        (rest, "展开 / 折叠内容")
    };
    elements.push(json!({"tag":"collapsible_panel","expanded":false,
        "header":{"position":"bottom","title":{"tag":"plain_text","content":label}},
        "elements":[{"tag":"markdown","element_id":"body","content":crate::lark_markdown::normalize(body)}]}));
    json!({"schema":"2.0","config":{"streaming_mode":false,"summary":{"content":summary(text)}},
        "body":{"elements":elements}})
}
pub async fn replace_card(bot: &Bot, card: &str, data: &str, seq: u32, uuid: &str) -> Result<()> {
    api(
        bot,
        "PUT",
        &format!("/open-apis/cardkit/v1/cards/{card}"),
        json!({"card":{"type":"card_json","data":data},"sequence":seq,"uuid":uuid}),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod fold_tests {
    use super::*;
    #[test]
    fn folding_preserves_full_markdown_and_only_long_answers_fold() {
        assert!(!should_fold(false, &"中".repeat(300)));
        assert!(should_fold(false, &"中".repeat(301)));
        assert!(should_fold(false, &"行\n".repeat(17)));
        assert!(should_fold(true, "进度"));
        let text = format!("**结论**\n{}", "完整正文".repeat(220));
        let card = folded_card(false, &text);
        assert_eq!(card["config"]["streaming_mode"], false);
        assert!(card.get("header").is_none());
        assert_eq!(card["body"]["elements"].as_array().unwrap().len(), 1);
        assert_eq!(card["config"]["summary"]["content"], summary(&text));
        let panel = &card["body"]["elements"][0];
        assert_eq!(panel["tag"], "collapsible_panel");
        assert_eq!(panel["expanded"], false);
        assert_eq!(
            panel["elements"][0]["content"],
            crate::lark_markdown::normalize(&text)
        );
        assert_eq!(panel["header"]["position"], "bottom");
        let paragraphs = "这是开头。\n\n**后续内容**\n\n继续。";
        let split = folded_card(false, paragraphs);
        assert_eq!(split["body"]["elements"][0]["content"], "这是开头。");
        assert_eq!(
            split["body"]["elements"][1]["elements"][0]["content"],
            "**后续内容**\n\n继续。"
        );
        let (intro, rest) = fold_sections(paragraphs);
        assert_eq!(format!("{}\n\n{rest}", intro.unwrap()), paragraphs);
        let ticks = char::from(96).to_string().repeat(3);
        let code = format!("{ticks}bash\necho hello\n\nmore code\n{ticks}");
        assert_eq!(fold_sections(&code), (None, code.as_str()));
        assert_eq!(
            fold_sections("| a | b |\n\n| c | d |"),
            (None, "| a | b |\n\n| c | d |")
        );
        let progress = folded_card(true, "正在查资料\n\n— 已完成");
        assert_eq!(progress["body"]["elements"][0]["content"], "已完成");
        assert_eq!(
            progress["body"]["elements"][1]["elements"][0]["content"],
            "正在查资料"
        );
    }
}
