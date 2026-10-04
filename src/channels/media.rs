use super::*;
use aes::cipher::{BlockDecrypt, KeyInit};
use std::path::Path;
use tokio::io::AsyncWriteExt;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncomingAttachment {
    pub name: String,
    pub kind: String,
    pub source: Value,
}
/// Resolve the CLI's rendered media events only after durable receipt.
pub async fn hydrate(bot: &Bot, incoming: &Incoming) -> Result<Incoming> {
    let Some(id) = incoming
        .attachments
        .iter()
        .find_map(|a| a.source["hydrate_message_id"].as_str())
    else {
        return Ok(incoming.clone());
    };
    anyhow::ensure!(
        id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "invalid message identifier"
    );
    let mut cmd = lark(bot)?;
    cmd.args([
        "api",
        "GET",
        &format!("/open-apis/im/v1/messages/{id}"),
        "--as",
        "bot",
    ]);
    let result = command_output(&mut cmd).await?;
    let value: Value = serde_json::from_slice(&result.stdout)?;
    let item = value
        .pointer("/data/items/0")
        .or_else(|| value.pointer("/data/data/items/0"))
        .context("message resource unavailable")?;
    anyhow::ensure!(
        item["message_id"] == id && item["chat_id"] == incoming.chat,
        "message origin mismatch"
    );
    let raw = item
        .pointer("/body/content")
        .and_then(Value::as_str)
        .context("message content unavailable")?;
    let content: Value = serde_json::from_str(raw)?;
    let m = json!({"message_id":id,"content":raw});
    let mut hydrated = incoming.clone();
    hydrated.attachments = lark_attachments(&m);
    hydrated.text = ordered_text(&content);
    anyhow::ensure!(
        !hydrated.attachments.is_empty() || !hydrated.text.trim().is_empty(),
        "unsupported message content"
    );
    Ok(hydrated)
}
pub fn ordered_text(value: &Value) -> String {
    fn visit(v: &Value, out: &mut String, index: &mut usize) {
        if v.get("image_key").is_some() || v.get("file_key").is_some() {
            *index += 1;
            out.push_str(&format!("[附件 {}]", index));
        } else if let Some(s) = v.get("text").and_then(Value::as_str) {
            out.push_str(s);
        } else if let Some(a) = v.as_array() {
            for item in a {
                visit(item, out, index);
            }
            out.push(char::from(10));
        } else if let Some(o) = v.as_object() {
            if let Some(t) = o.get("title").and_then(Value::as_str) {
                out.push_str(t);
                out.push(char::from(10));
            }
            for (k, item) in o {
                if k != "title" {
                    visit(item, out, index);
                }
            }
        }
    }
    let mut out = String::new();
    visit(value, &mut out, &mut 0);
    out
}
pub fn lark_attachments(m: &Value) -> Vec<IncomingAttachment> {
    let content = m["content"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    let mut out = Vec::new();
    fn visit(v: &Value, m: &Value, out: &mut Vec<IncomingAttachment>) {
        if let Some(key) = v
            .get("image_key")
            .or_else(|| v.get("file_key"))
            .and_then(Value::as_str)
        {
            let kind = if v.get("image_key").is_some() {
                "image"
            } else {
                "file"
            };
            out.push(IncomingAttachment{name:v["file_name"].as_str().unwrap_or(if kind=="image"{"image.png"}else{"attachment"}).into(),kind:kind.into(),source:json!({"channel":"lark","message_id":m["message_id"],"key":key,"type":kind})});
        }
        if let Some(a) = v.as_array() {
            for x in a {
                visit(x, m, out)
            }
        }
        if let Some(o) = v.as_object() {
            for (k, x) in o {
                if k != "image_key" && k != "file_key" {
                    visit(x, m, out)
                }
            }
        }
    }
    visit(&content, m, &mut out);
    if out.is_empty() {
        visit(m, m, &mut out);
    }
    out
}
pub fn wechat_attachments(m: &Value) -> Vec<IncomingAttachment> {
    let mut out = Vec::new();
    for item in m["item_list"].as_array().into_iter().flatten() {
        for (field, kind) in [
            ("image_item", "image"),
            ("file_item", "file"),
            ("video_item", "video"),
        ] {
            if let Some(v) = item.get(field) {
                out.push(IncomingAttachment {
                    name: v["file_name"]
                        .as_str()
                        .unwrap_or(if kind == "image" {
                            "image.png"
                        } else {
                            "attachment"
                        })
                        .into(),
                    kind: kind.into(),
                    source: json!({"channel":"wechat","media":v["media"],"aeskey":v["aeskey"]}),
                });
            }
        }
    }
    out
}
fn api_error(output: &std::process::Output) -> anyhow::Error {
    for bytes in [&output.stderr, &output.stdout] {
        let text = String::from_utf8_lossy(bytes);
        if text.contains("99991672")
            || text.contains("app_scope_not_applied")
            || text.contains("missing_scopes")
        {
            return anyhow::anyhow!(
                "permission_denied: Bot lacks required media resource scope; check /bot doctor"
            );
        }
    }
    anyhow::anyhow!(
        "media channel command failed (exit {}); diagnostics withheld",
        output.status
    )
}
async fn command_output(cmd: &mut Command) -> Result<std::process::Output> {
    let result = tokio::time::timeout(Duration::from_secs(120), cmd.kill_on_drop(true).output())
        .await
        .context("media transfer timeout")??;
    if !result.status.success() {
        return Err(api_error(&result));
    }
    Ok(result)
}
pub async fn download_attachment(
    bot: &Bot,
    a: &IncomingAttachment,
    dest: &Path,
    max: u64,
) -> Result<()> {
    anyhow::ensure!(a.source["channel"] == bot.channel, "media channel mismatch");
    if bot.channel == "lark" {
        let parent = dest.parent().context("missing download directory")?;
        std::fs::create_dir_all(parent)?;
        let mut cmd = lark(bot)?;
        cmd.current_dir(parent).args([
            "im",
            "+messages-resources-download",
            "--message-id",
            a.source["message_id"]
                .as_str()
                .context("missing source message")?,
            "--file-key",
            a.source["key"].as_str().context("missing resource key")?,
            "--type",
            a.source["type"].as_str().unwrap_or("file"),
            "--output",
            dest.file_name()
                .and_then(|x| x.to_str())
                .context("bad destination")?,
            "--as",
            "bot",
        ]);
        let output = command_output(&mut cmd);
        tokio::pin!(output);
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        loop {
            tokio::select! {
                result = &mut output => { result?; break; }
                _ = tick.tick() => {
                    let size = std::fs::read_dir(parent)?.filter_map(|e| e.ok()).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum::<u64>();
                    anyhow::ensure!(size <= max, "media_size_exceeded");
                }
            }
        }
        if !dest.exists() {
            let files = std::fs::read_dir(parent)?
                .filter_map(|x| x.ok())
                .filter(|x| x.file_type().is_ok_and(|t| t.is_file()))
                .collect::<Vec<_>>();
            anyhow::ensure!(files.len() == 1, "download output missing or ambiguous");
            std::fs::rename(files[0].path(), dest)?;
        }
        anyhow::ensure!(dest.metadata()?.len() <= max, "media_size_exceeded");
        return Ok(());
    }
    anyhow::ensure!(bot.channel == "wechat", "unsupported media channel");
    let media = &a.source["media"];
    let query = media["encrypt_query_param"]
        .as_str()
        .or_else(|| media["encrypted_query_param"].as_str())
        .context("missing CDN media reference")?;
    let mut url = reqwest::Url::parse("https://novac2c.cdn.weixin.qq.com/c2c/download")?;
    url.query_pairs_mut()
        .append_pair("encrypted_query_param", query);
    let mut response = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| e.without_url())?
        .error_for_status()
        .map_err(|e| e.without_url())?;
    anyhow::ensure!(
        response.content_length().is_none_or(|s| s <= max + 16),
        "media_size_exceeded"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
        anyhow::ensure!(
            bytes.len() as u64 + chunk.len() as u64 <= max + 16,
            "media_size_exceeded"
        );
        bytes.extend_from_slice(&chunk);
    }
    let encoded = media["aes_key"]
        .as_str()
        .or_else(|| a.source["aeskey"].as_str())
        .context("missing media key")?;
    let decoded = if encoded.len() == 32 && encoded.bytes().all(|b| b.is_ascii_hexdigit()) {
        encoded.as_bytes().to_vec()
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .context("invalid media key encoding")?
    };
    let key = if decoded.len() == 16 {
        decoded
    } else {
        let hex = std::str::from_utf8(&decoded)?;
        anyhow::ensure!(hex.len() == 32, "invalid media key");
        (0..32)
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() % 16 == 0,
        "invalid encrypted media length"
    );
    let cipher =
        aes::Aes128::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid media key"))?;
    for block in bytes.as_chunks_mut::<16>().0 {
        cipher.decrypt_block(aes::Block::from_mut_slice(block));
    }
    let pad = *bytes.last().unwrap() as usize;
    anyhow::ensure!(
        pad > 0
            && pad <= 16
            && bytes[bytes.len() - pad..]
                .iter()
                .all(|b| *b as usize == pad),
        "invalid media padding"
    );
    bytes.truncate(bytes.len() - pad);
    anyhow::ensure!(bytes.len() as u64 <= max, "media_size_exceeded");
    let mut file = tokio::fs::File::create(dest).await?;
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    Ok(())
}
pub async fn prepare_media(
    bot: &Bot,
    target: &ReplyTarget,
    path: &Path,
    name: &str,
    kind: &str,
    key: &str,
) -> Result<Value> {
    if bot.channel == "local" {
        return Ok(json!({"channel":"local"}));
    }
    if bot.channel == "wechat" {
        let request =
            wechat_file::prepare_bytes(&credential(bot)?, target, &std::fs::read(path)?, name, key)
                .await?;
        return Ok(
            json!({"channel":"wechat","request":request,"presentation":"file","fallback":if kind=="image"{Some("image_as_file")}else{None}}),
        );
    }
    let mut cmd = lark(bot)?;
    let image = kind == "image";
    let field = if image { "image" } else { "file" };
    let body = if image {
        json!({"image_type":"message"})
    } else {
        json!({"file_type":"stream","file_name":name})
    };
    cmd.stdin(std::process::Stdio::from(std::fs::File::open(path)?));
    cmd.args([
        "im",
        if image { "images" } else { "files" },
        "create",
        "--data",
        &body.to_string(),
        "--file",
        "-",
        "--as",
        "bot",
    ]);
    let out = command_output(&mut cmd).await?;
    let value: Value = serde_json::from_slice(&out.stdout)?;
    let resource = if image { "image_key" } else { "file_key" };
    let key = value
        .pointer(&format!("/data/{resource}"))
        .or_else(|| value.pointer(&format!("/data/data/{resource}")))
        .or_else(|| value.get(resource))
        .and_then(Value::as_str)
        .context("upload did not return resource key")?;
    Ok(json!({"channel":"lark","kind":field,"key":key,"app_id":bot.account_id}))
}
pub async fn send_prepared_media(
    bot: &Bot,
    target: &ReplyTarget,
    prepared: &Value,
    key: &str,
) -> Result<()> {
    anyhow::ensure!(prepared["channel"] == bot.channel, "media channel mismatch");
    if bot.channel == "local" {
        return Ok(());
    }
    if bot.channel == "wechat" {
        return wx_post(&credential(bot)?, "sendmessage", &prepared["request"])
            .await
            .map(|_| ());
    }
    anyhow::ensure!(
        prepared["app_id"] == bot.account_id,
        "media upload belongs to different app"
    );
    let mut cmd = lark(bot)?;
    cmd.arg("im");
    if let Some(thread) = &target.thread_id {
        cmd.args(["+messages-reply", "--message-id", thread]);
    } else {
        cmd.args([
            "+messages-send",
            if target.chat_id.starts_with("ou_") {
                "--user-id"
            } else {
                "--chat-id"
            },
            &target.chat_id,
        ]);
    }
    cmd.args([
        if prepared["kind"] == "image" {
            "--image"
        } else {
            "--file"
        },
        prepared["key"].as_str().context("missing uploaded key")?,
        "--as",
        "bot",
        "--idempotency-key",
        key,
    ]);
    command_output(&mut cmd).await?;
    Ok(())
}
