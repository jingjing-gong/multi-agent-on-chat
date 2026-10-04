use crate::domain::{Bot, ReplyTarget};
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;
pub mod media;
pub mod wechat_file;
pub mod wechat_format;
pub use media::{download_attachment, prepare_media, send_prepared_media};

#[derive(Serialize, Deserialize)]
pub struct WechatCredential {
    pub bot_token: String,
    pub account_id: String,
    pub base_url: String,
    pub owner: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Incoming {
    pub event_id: String,
    pub is_private: bool,
    pub sender: String,
    pub chat: String,
    pub text: String,
    #[serde(default)]
    pub untranscribed_voice: usize,
    pub reply: ReplyTarget,
    #[serde(default)]
    pub attachments: Vec<media::IncomingAttachment>,
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("missing parent")?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(40))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn success(v: &Value) -> Result<()> {
    for field in ["ret", "errcode", "code"] {
        if v[field].as_i64().unwrap_or(0) != 0 {
            bail!("channel API returned {field}={}", v[field]);
        }
    }
    Ok(())
}
pub fn validate_wechat_url(url: &str) -> Result<()> {
    let u = reqwest::Url::parse(url)?;
    if u.scheme() != "https"
        || !u
            .host_str()
            .is_some_and(|h| h == "ilinkai.weixin.qq.com" || h.ends_with(".weixin.qq.com"))
    {
        bail!("untrusted WeChat API origin");
    }
    Ok(())
}
pub async fn wechat_login() -> Result<WechatCredential> {
    wechat_login_with_link(|_| Ok(())).await
}
pub async fn wechat_login_with_link(
    mut on_link: impl FnMut(&str) -> Result<()>,
) -> Result<WechatCredential> {
    let c = client()?;
    let qr: Value = c
        .get("https://ilinkai.weixin.qq.com/ilink/bot/get_bot_qrcode")
        .query(&[("bot_type", "3")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    success(&qr)?;
    let code = qr["qrcode"].as_str().context("missing QR identifier")?;
    let content = qr["qrcode_img_content"]
        .as_str()
        .context("missing QR link")?;
    on_link(content)?;
    eprintln!("Scan with WeChat: {content}");
    let image = qrcode::QrCode::new(content)?
        .render::<qrcode::render::unicode::Dense1x2>()
        .build();
    eprintln!("{image}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    while tokio::time::Instant::now() < deadline {
        let v: Value = c
            .get("https://ilinkai.weixin.qq.com/ilink/bot/get_qrcode_status")
            .query(&[("qrcode", code)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        success(&v)?;
        match v["status"].as_str().unwrap_or("") {
            "confirmed" => {
                let base = v["baseurl"]
                    .as_str()
                    .unwrap_or("https://ilinkai.weixin.qq.com");
                validate_wechat_url(base)?;
                return Ok(WechatCredential {
                    bot_token: v["bot_token"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .context("missing token")?
                        .into(),
                    account_id: v["ilink_bot_id"].as_str().context("missing bot id")?.into(),
                    base_url: base.into(),
                    owner: v["ilink_user_id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .context("missing owner id")?
                        .into(),
                });
            }
            "expired" => bail!("QR expired; run the add command again"),
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
    bail!("QR login timed out")
}
fn credential(bot: &Bot) -> Result<WechatCredential> {
    let p = bot
        .credential_ref
        .as_ref()
        .context("missing credential reference")?;
    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
}
async fn wx_post(cred: &WechatCredential, path: &str, value: &Value) -> Result<Value> {
    validate_wechat_url(&cred.base_url)?;
    let v: Value = client()?
        .post(format!(
            "{}/ilink/bot/{path}",
            cred.base_url.trim_end_matches('/')
        ))
        .header("AuthorizationType", "ilink_bot_token")
        .bearer_auth(&cred.bot_token)
        .header(
            "X-WECHAT-UIN",
            base64::engine::general_purpose::STANDARD
                .encode((uuid::Uuid::new_v4().as_u128() as u32).to_string()),
        )
        .json(value)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    success(&v)?;
    Ok(v)
}
pub async fn wechat_poll(bot: &Bot, cursor: &str) -> Result<(String, Vec<Incoming>)> {
    let v = wx_post(
        &credential(bot)?,
        "getupdates",
        &json!({"get_updates_buf":cursor,"base_info":{"channel_version":"0.1.0"}}),
    )
    .await?;
    Ok(parse_wechat_updates(&v, cursor))
}
pub(crate) fn parse_wechat_updates(v: &Value, cursor: &str) -> (String, Vec<Incoming>) {
    let mut out = vec![];
    for m in v["msgs"].as_array().into_iter().flatten() {
        if m["message_type"].as_i64() != Some(1) {
            continue;
        }
        if m.get("message_state")
            .is_some_and(|state| !state.is_null() && state.as_i64() != Some(2))
        {
            continue;
        }
        let sender = m["from_user_id"].as_str().unwrap_or("");
        if sender.is_empty() {
            continue;
        }
        let mut media_index = 0;
        let text = m["item_list"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| {
                if let Some(text) = i["text_item"]["text"].as_str() {
                    Some(text.to_owned())
                } else if let Some(voice) = i.get("voice_item") {
                    voice["text"]
                        .as_str()
                        .filter(|text| !text.trim().is_empty())
                        .map(str::to_owned)
                } else if ["image_item", "file_item", "video_item"]
                    .iter()
                    .any(|k| i.get(k).is_some())
                {
                    media_index += 1;
                    Some(format!("[附件 {media_index}]"))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let untranscribed_voice = m["item_list"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| {
                item.get("voice_item").is_some()
                    && item["voice_item"]["text"]
                        .as_str()
                        .is_none_or(|text| text.trim().is_empty())
            })
            .count();
        if text.trim().is_empty()
            && media::wechat_attachments(m).is_empty()
            && untranscribed_voice == 0
        {
            continue;
        }
        let id = m["message_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| m["message_id"].to_string());
        if id == "null" {
            continue;
        }
        out.push(Incoming {
            untranscribed_voice,
            attachments: media::wechat_attachments(m),
            event_id: id,
            is_private: true,
            sender: sender.into(),
            chat: sender.into(),
            text,
            reply: ReplyTarget {
                chat_id: sender.into(),
                context_token: m["context_token"].as_str().map(str::to_owned),
                thread_id: None,
            },
        });
    }
    (v["get_updates_buf"].as_str().unwrap_or(cursor).into(), out)
}
pub fn lark_markdown(text: &str) -> String {
    crate::lark_markdown::normalize(&crate::message::flatten_tables(text))
}
pub async fn send(bot: &Bot, target: &ReplyTarget, text: &str, key: &str) -> Result<()> {
    send_message(bot, target, &crate::message::Message::legacy(text), key).await
}
pub fn lark_post_content(message: &crate::message::Message) -> Value {
    use crate::message::Message;
    let rows = match message {
        Message::RichText(rows) => rows.iter().map(|row| {
            if row.is_empty() { return json!([{"tag":"text","text":" "}]); }
            json!(row.iter().map(|span| json!({"tag":"text","text":span.text,"style":if span.bold {vec!["bold"]}else{vec![]}})).collect::<Vec<_>>())
        }).collect::<Vec<_>>(),
        Message::File(_) | Message::Media{..} | Message::Bundle(_) => vec![json!([])],
        Message::Markdown(text) => vec![json!([{"tag":"md","text":lark_markdown(text)}])],
        Message::Plain(text) => vec![json!([{"tag":"text","text":text}])],
    };
    json!({"zh_cn":{"content":rows}})
}
pub async fn send_message(
    bot: &Bot,
    target: &ReplyTarget,
    message: &crate::message::Message,
    key: &str,
) -> Result<()> {
    if let crate::message::Message::Media {
        path,
        name,
        kind,
        expires_at,
        ..
    } = message
    {
        anyhow::ensure!(
            expires_at.is_none_or(|e| e > crate::domain::now()),
            "media_expired"
        );
        let prepared = prepare_media(bot, target, path, name, kind, key).await?;
        return send_prepared_media(bot, target, &prepared, key).await;
    }
    if let crate::message::Message::Bundle(parts) = message {
        for (i, part) in parts.iter().enumerate() {
            Box::pin(send_message(bot, target, part, &format!("{key}:{i}"))).await?;
        }
        return Ok(());
    }
    let text = message.plain_text();
    match bot.channel.as_str() {
        "local" => Ok(()),
        "wechat" => {
            let cred = credential(bot)?;
            if let crate::message::Message::File(path) = message {
                let request = wechat_file::prepare_bytes(
                    &cred,
                    target,
                    &std::fs::read(path)?,
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .context("invalid attachment name")?,
                    key,
                )
                .await?;
                return wx_post(&cred, "sendmessage", &request).await.map(|_| ());
            }
            if let crate::message::Message::Plain(raw) = message {
                if raw.len() > 3000 {
                    return wechat_file::send(&cred, target, raw, key).await;
                }
            }
            for part in wechat_format::requests(message, target, key) {
                wx_post(&cred, "sendmessage", &part).await?;
            }
            Ok(())
        }
        "lark" => {
            let mut cmd = lark(bot)?;
            cmd.args(["im"]);
            if let Some(thread) = &target.thread_id {
                cmd.args(["+messages-reply", "--message-id", thread]);
            } else {
                let target_flag = if target.chat_id.starts_with("ou_") {
                    "--user-id"
                } else {
                    "--chat-id"
                };
                cmd.args(["+messages-send", target_flag, &target.chat_id]);
            }
            // Keep raw JSON literal; use an explicit post to avoid CLI image
            // auto-downloads while enabling Lark's Markdown renderer.
            if let crate::message::Message::File(path) = message {
                cmd.current_dir(path.parent().context("missing attachment directory")?);
                cmd.args([
                    "--image",
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .context("invalid attachment name")?,
                ]);
            } else if matches!(message, crate::message::Message::Plain(_)) {
                cmd.args(["--text", &text]);
            } else {
                let content = lark_post_content(message).to_string();
                cmd.args(["--msg-type", "post", "--content", &content]);
            }
            cmd.args(["--as", "bot", "--idempotency-key", key]);
            let out = tokio::time::timeout(Duration::from_secs(40), cmd.output()).await??;
            if !out.status.success() {
                bail!("Lark send failed (diagnostics withheld to protect credentials)");
            }
            let v: Value =
                serde_json::from_slice(&out.stdout).context("invalid Lark send response")?;
            success(&v)
        }
        _ => bail!("unknown channel"),
    }
}
pub async fn wechat_send_request(bot: &Bot, request: &Value) -> Result<()> {
    wx_post(&credential(bot)?, "sendmessage", request).await?;
    Ok(())
}
pub async fn wechat_send_file(
    bot: &Bot,
    target: &ReplyTarget,
    text: &str,
    key: &str,
) -> Result<()> {
    wechat_file::send(&credential(bot)?, target, text, key).await
}
pub async fn wechat_prepare_attachment(
    bot: &Bot,
    target: &ReplyTarget,
    path: &Path,
    key: &str,
) -> Result<Value> {
    wechat_file::prepare_bytes(
        &credential(bot)?,
        target,
        &std::fs::read(path)?,
        path.file_name()
            .and_then(|p| p.to_str())
            .context("invalid attachment name")?,
        key,
    )
    .await
}
pub async fn wechat_prepare_file(
    bot: &Bot,
    target: &ReplyTarget,
    text: &str,
    key: &str,
) -> Result<Value> {
    wechat_file::prepare(&credential(bot)?, target, text, key).await
}
pub fn lark(bot: &Bot) -> Result<Command> {
    let mut cmd = Command::new("lark-cli");
    let credentials = bot
        .credential_ref
        .as_ref()
        .context("missing lark config directory")?;
    let runtime = crate::lark_runtime::config_path(credentials, &bot.account_id)?;
    cmd.env("LARKSUITE_CLI_CONFIG_DIR", runtime)
        .kill_on_drop(true);
    Ok(cmd)
}
pub async fn lark_login(dir: &Path) -> Result<String> {
    lark_login_with_app(dir, None, "feishu").await
}
/// Keep old QR attachments readable by queued deliveries; never reuse credential state.
pub fn lark_registration_directory(path: &Path) -> Result<std::path::PathBuf> {
    if !path.try_exists()? {
        return Ok(path.to_path_buf());
    }
    anyhow::ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_dir(),
        "credential path is not a directory"
    );
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let safe = (kind.is_dir() && (name == "cache" || name == "logs"))
            || (kind.is_file() && name == "login-qr.png");
        anyhow::ensure!(
            safe,
            "existing credential configuration found; refusing to overwrite it"
        );
    }
    let parent = path.parent().context("missing credentials root")?;
    loop {
        // Keep the lark-cli Unix socket path within macOS limits.
        let retry = parent.join(&uuid::Uuid::new_v4().simple().to_string()[..8]);
        match std::fs::create_dir(&retry) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&retry, std::fs::Permissions::from_mode(0o700))?;
                }
                return Ok(retry);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

fn lark_init_args(app_id: Option<&str>, brand: &str) -> Result<Vec<String>> {
    if !matches!(brand, "feishu" | "lark") {
        bail!("brand must be feishu or lark");
    }
    let mut args = vec!["config".into(), "init".into()];
    if let Some(id) = app_id {
        if !id.starts_with("cli_")
            || id.len() <= 4
            || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            bail!("invalid Lark App ID");
        }
        args.extend([
            "--app-id".into(),
            id.into(),
            "--app-secret-stdin".into(),
            "--brand".into(),
            brand.into(),
        ]);
    } else {
        args.push("--new".into());
    }
    Ok(args)
}
pub async fn lark_login_with_app(dir: &Path, app_id: Option<&str>, brand: &str) -> Result<String> {
    lark_login_with_secret(dir, app_id, brand, None).await
}
pub async fn lark_login_with_secret(
    dir: &Path,
    app_id: Option<&str>,
    brand: &str,
    secret: Option<&str>,
) -> Result<String> {
    if let Some(secret) = secret {
        if app_id.is_none() || secret.trim().is_empty() || secret.contains(['\n', '\r']) {
            bail!("App Secret requires an App ID and must be nonempty single-line text");
        }
    }
    lark_login_internal(dir, app_id, brand, secret, None).await
}
pub async fn lark_login_with_link(
    dir: &Path,
    app_id: Option<&str>,
    brand: &str,
    secret: Option<&str>,
    notify: impl Fn(&str, &Path) -> Result<()> + Send + Sync,
) -> Result<String> {
    lark_login_internal(dir, app_id, brand, secret, Some(&notify)).await
}
type LoginNotice<'a> = dyn Fn(&str, &Path) -> Result<()> + Send + Sync + 'a;
fn lark_registration_url(line: &str) -> Option<&str> {
    let text = line.trim();
    let url = reqwest::Url::parse(text).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some_and(|h| {
            h == "feishu.cn"
                || h.ends_with(".feishu.cn")
                || h == "larksuite.com"
                || h.ends_with(".larksuite.com")
        }))
    .then_some(text)
}
async fn lark_login_internal(
    dir: &Path,
    app_id: Option<&str>,
    brand: &str,
    secret: Option<&str>,
    notify: Option<&LoginNotice<'_>>,
) -> Result<String> {
    if let Some(secret) = secret {
        anyhow::ensure!(
            app_id.is_some()
                && !secret.trim().is_empty()
                && !secret.contains([char::from(10), char::from(13)]),
            "invalid app secret"
        );
    }
    let args = lark_init_args(app_id, brand)?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut cmd = Command::new("lark-cli");
    cmd.env("LARKSUITE_CLI_CONFIG_DIR", dir)
        .args(args)
        .stdin(if secret.is_some() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stdout(if secret.is_some() || notify.is_some() {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .stderr(if notify.is_some() && app_id.is_none() {
            Stdio::piped()
        } else if secret.is_some() {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    if let Some(secret) = secret {
        use tokio::io::AsyncWriteExt;
        let mut input = child
            .stdin
            .take()
            .context("missing credential input pipe")?;
        input.write_all(secret.as_bytes()).await?;
        input.write_all(b"\n").await?;
        input.shutdown().await?;
    }
    let status = tokio::time::timeout(Duration::from_secs(300), async {
        if let Some(output) = child.stderr.take() {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(output).lines();
            let mut sent = false;
            while let Some(line) = lines.next_line().await? {
                if !sent {
                    if let Some(url) = lark_registration_url(&line) {
                        let image = dir.join("login-qr.png");
                        let qr = Command::new("lark-cli")
                            .current_dir(dir)
                            .args(["auth", "qrcode", url, "--output", "login-qr.png"])
                            .kill_on_drop(true)
                            .output()
                            .await?;
                        anyhow::ensure!(qr.status.success(), "QR image generation failed");
                        notify.context("missing login notification")?(url, &image)?;
                        sent = true;
                    }
                }
            }
        }
        child.wait().await.map_err(anyhow::Error::from)
    })
    .await??;
    if !status.success() {
        bail!("Lark onboarding failed");
    }
    let output = Command::new("lark-cli")
        .env("LARKSUITE_CLI_CONFIG_DIR", dir)
        .args(["auth", "status", "--json", "--verify"])
        .kill_on_drop(true)
        .output()
        .await?;
    if !output.status.success() {
        bail!("Lark identity verification failed");
    }
    let v: Value = serde_json::from_slice(&output.stdout)?;
    success(&v)?;
    let selected = Command::new("lark-cli")
        .env("LARKSUITE_CLI_CONFIG_DIR", dir)
        .args(["whoami", "--as", "bot"])
        .kill_on_drop(true)
        .output()
        .await?;
    if !selected.status.success() {
        bail!("explicit Lark bot selection failed");
    }
    let account = verified_lark_identity(&v, &serde_json::from_slice(&selected.stdout)?)?;
    if app_id.is_some_and(|expected| expected != account) {
        bail!("verified app differs from requested App ID");
    }
    Ok(account)
}
fn identity_field<'a>(v: &'a Value, a: &str, b: &str) -> Result<&'a str> {
    let x = v
        .get(a)
        .or_else(|| v.get(b))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .context("missing identity field")?;
    if v.get(a).is_some() && v.get(b).is_some() && v[a] != v[b] {
        bail!("conflicting identity fields");
    }
    Ok(x)
}
fn verified_lark_identity(status: &Value, selected: &Value) -> Result<String> {
    let wrapped = status.get("data").is_some();
    let payload = if wrapped {
        if status["ok"] != true || !matches!(status["identity"].as_str(), Some("bot" | "user")) {
            bail!("invalid identity envelope");
        }
        if status.get("appId").is_some()
            || status.get("app_id").is_some()
            || status.get("identities").is_some()
        {
            bail!("mixed identity envelope");
        }
        &status["data"]
    } else {
        if status["verified"] != true
            || !matches!(status["identity"].as_str(), Some("bot" | "user"))
        {
            bail!("unverified identity response");
        }
        status
    };
    success(payload)?;
    let app = identity_field(payload, "appId", "app_id")?;
    let brand = payload["brand"].as_str().context("missing brand")?;
    let bot = &payload["identities"]["bot"];
    if bot["status"] != "ready" || bot["available"] != true || bot["verified"] != true {
        bail!("bot is not verified and ready");
    }
    let open = identity_field(bot, "openId", "open_id")?;
    if !app.starts_with("cli_") || !open.starts_with("ou_") || !matches!(brand, "feishu" | "lark") {
        bail!("invalid bot identity");
    }
    if selected["identity"] != "bot"
        || selected["brand"] != brand
        || identity_field(selected, "appId", "app_id")? != app
    {
        bail!("explicit bot identity differs from verified app");
    }
    Ok(app.into())
}
fn lark_post_text(value: &Value) -> Option<String> {
    let post = if value.get("content").is_some() {
        value
    } else {
        value.get("zh_cn").or_else(|| value.get("en_us"))?
    };
    let mut lines = Vec::new();
    if let Some(title) = post["title"].as_str().filter(|s| !s.is_empty()) {
        lines.push(title.to_owned());
    }
    for row in post["content"].as_array()? {
        let mut text = String::new();
        for item in row.as_array()? {
            match item["tag"].as_str()? {
                "text" | "a" | "md" => text.push_str(item["text"].as_str()?),
                "code_block" => text.push_str(item["text"].as_str()?),
                "at" => text.push_str(item["user_name"].as_str().unwrap_or("@user")),
                "img" => text.push_str("[图片]"),
                _ => text.push_str("[附件]"),
            }
        }
        lines.push(text);
    }
    Some(lines.join(
        "
",
    ))
}

pub fn parse_lark(v: &Value) -> Option<Incoming> {
    let event = v.get("event").unwrap_or(v);
    let flat = event.get("message").is_none();
    if flat
        && event["type"]
            .as_str()
            .is_some_and(|t| t != "im.message.receive_v1")
    {
        return None;
    }
    let m = if flat { event } else { &event["message"] };
    let sender_type = if flat {
        &event["sender_type"]
    } else {
        &event["sender"]["sender_type"]
    };
    if sender_type.as_str() != Some("user")
        || !matches!(
            m["message_type"].as_str(),
            Some("text" | "post" | "image" | "file" | "audio" | "media")
        )
    {
        return None;
    }
    let sender = if flat {
        event["sender_id"].as_str()?
    } else {
        event["sender"]["sender_id"]["open_id"].as_str()?
    };
    let chat = m["chat_id"].as_str()?;
    let id = m["message_id"].as_str()?;
    if sender.is_empty() || chat.is_empty() || id.is_empty() {
        return None;
    }
    let raw = m["content"].as_str()?;
    let mut text = if !matches!(m["message_type"].as_str(), Some("text" | "post")) {
        String::new()
    } else if m["message_type"] == "post" {
        match serde_json::from_str::<Value>(raw) {
            Ok(value) => lark_post_text(&value)?,
            Err(_) if flat => raw.to_owned(), // CLI has already flattened the rich text.
            Err(_) => return None,
        }
    } else if flat {
        raw.to_owned()
    } else {
        let content: Value = serde_json::from_str(raw).ok()?;
        content["text"].as_str()?.to_owned()
    };
    let mut attachments = media::lark_attachments(m);
    if flat && m["message_type"] != "text" && attachments.is_empty() {
        attachments.push(media::IncomingAttachment {
            name: "message".into(),
            kind: "pending".into(),
            source: json!({"hydrate_message_id":id}),
        });
    }
    if !flat && !attachments.is_empty() {
        text = media::ordered_text(&serde_json::from_str::<Value>(raw).ok()?);
    }
    if text.trim().is_empty() && attachments.is_empty() {
        return None;
    }
    Some(Incoming {
        untranscribed_voice: 0,
        attachments,
        event_id: id.into(),
        is_private: m["chat_type"].as_str() == Some("p2p"),
        sender: sender.into(),
        chat: chat.into(),
        text,
        reply: ReplyTarget {
            chat_id: chat.into(),
            context_token: None,
            thread_id: Some(id.into()),
        },
    })
}
#[cfg(test)]
mod tests {
    #[test]
    fn rich_text_commands_survive_flat_and_native_events() {
        let command = "/bot add lark --name om-assistant";
        let mut flat = serde_json::json!({"sender_type":"user","sender_id":"ou_owner","message_type":"post","chat_id":"oc_chat","chat_type":"p2p","message_id":"om_msg","content":command});
        assert_eq!(super::parse_lark(&flat).unwrap().text, command);
        let post = serde_json::json!({"zh_cn":{"title":"","content":[[{"tag":"text","text":"/bot add lark ","style":["bold"]},{"tag":"text","text":"--name om-assistant"}]]}});
        flat["content"] = serde_json::json!(post.to_string());
        assert_eq!(super::parse_lark(&flat).unwrap().text, command);
        let native = serde_json::json!({"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_owner"}},"message":flat}});
        assert_eq!(super::parse_lark(&native).unwrap().text, command);
        flat["sender_type"] = serde_json::json!("app");
        assert!(super::parse_lark(&flat).is_none());
    }

    #[test]
    fn unfinished_registration_can_retry_without_moving_queued_qr() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bot");
        assert_eq!(super::lark_registration_directory(&path).unwrap(), path);
        std::fs::create_dir_all(path.join("logs")).unwrap();
        std::fs::write(path.join("login-qr.png"), b"old QR").unwrap();
        let retry = super::lark_registration_directory(&path).unwrap();
        assert_ne!(retry, path);
        assert_eq!(retry.file_name().unwrap().len(), 8);
        assert_eq!(retry.parent(), path.parent());
        assert_eq!(std::fs::read(path.join("login-qr.png")).unwrap(), b"old QR");
        std::fs::write(path.join("config.json"), b"existing credentials").unwrap();
        assert!(super::lark_registration_directory(&path).is_err());
    }

    #[test]
    fn registration_url_preserves_opaque_query_and_rejects_other_output() {
        let url = "https://accounts.feishu.cn/register?token=a%2Bb&next=%2Fapp";
        assert_eq!(
            super::lark_registration_url(&format!("  {url}  ")),
            Some(url)
        );
        for line in [
            "secret=abc",
            "https://evil.example/login",
            "https://feishu.cn.evil.example/login",
            "http://feishu.cn/login",
        ] {
            assert_eq!(super::lark_registration_url(line), None);
        }
    }

    use super::*;
    #[test]
    fn existing_lark_app_uses_stdin_and_qr_path_stays_available() {
        assert_eq!(
            lark_init_args(None, "feishu").unwrap(),
            vec!["config", "init", "--new"]
        );
        assert_eq!(
            lark_init_args(Some("cli_example"), "lark").unwrap(),
            vec![
                "config",
                "init",
                "--app-id",
                "cli_example",
                "--app-secret-stdin",
                "--brand",
                "lark"
            ]
        );
        assert!(lark_init_args(Some("bad"), "feishu").is_err());
        assert!(lark_init_args(Some("cli_example"), "other").is_err());
    }
    #[test]
    fn lark_flat_and_native_events_match_and_bots_are_ignored() {
        let flat = json!({"type":"im.message.receive_v1","sender_type":"user","sender_id":"ou_owner","message_id":"om_1","chat_id":"oc_1","chat_type":"p2p","message_type":"text","content":"hello"});
        let native = json!({"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_owner"}},"message":{"message_id":"om_1","chat_id":"oc_1","message_type":"text","content":"{\"text\":\"hello\"}"}}});
        let a = parse_lark(&flat).unwrap();
        assert!(a.is_private);
        let mut group = flat.clone();
        group["chat_type"] = json!("group");
        assert!(!parse_lark(&group).unwrap().is_private);
        group.as_object_mut().unwrap().remove("chat_type");
        assert!(!parse_lark(&group).unwrap().is_private);
        let b = parse_lark(&native).unwrap();
        assert_eq!(a.text, b.text);
        assert_eq!(a.sender, b.sender);
        assert_eq!(a.reply.thread_id.as_deref(), Some("om_1"));
        let mut bot = flat.clone();
        bot["sender_type"] = json!("bot");
        assert!(parse_lark(&bot).is_none());
        let mut literal = flat;
        literal["content"] = json!("{\"text\":\"keep literal\"}");
        assert_eq!(
            parse_lark(&literal).unwrap().text,
            literal["content"].as_str().unwrap()
        );
    }
    #[test]
    fn wechat_keeps_cursor_context_and_numeric_id_ignores_bot() {
        let mut user = json!({"message_type":1,"message_id":42,"from_user_id":"owner","context_token":"opaque","item_list":[{"type":1,"text_item":{"text":"hi"}}]});
        let mut bot = user.clone();
        bot["message_type"] = json!(2);
        let (cursor, m) = parse_wechat_updates(
            &json!({"get_updates_buf":"next","msgs":[user.clone(),bot]}),
            "old",
        );
        assert_eq!(cursor, "next");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].event_id, "42");
        assert_eq!(m[0].reply.context_token.as_deref(), Some("opaque"));
        user["from_user_id"] = json!("");
        assert!(parse_wechat_updates(&json!({"msgs":[user]}), "old")
            .1
            .is_empty());
    }
    #[test]
    fn wechat_voice_text_preserves_order_and_skips_voice_downloads() {
        let mut event = json!({"message_type":1,"message_state":2,"message_id":42,"from_user_id":"owner","item_list":[
            {"text_item":{"text":"先"}},
            {"voice_item":{"text":"检查项目","media":{}}},
            {"image_item":{"media":{}}},
            {"voice_item":{"text":"然后总结"}},
            {"voice_item":{"text":"  "}},
            {"text_item":{"text":"谢谢"}}
        ]});
        let messages = parse_wechat_updates(&json!({"msgs":[event.clone()]}), "").1;
        let message = &messages[0];
        assert_eq!(message.text, "先\n检查项目\n[附件 1]\n然后总结\n谢谢");
        assert_eq!(message.untranscribed_voice, 1);
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(message.attachments[0].kind, "image");
        for state in [0, 1] {
            event["message_state"] = json!(state);
            assert!(parse_wechat_updates(&json!({"msgs":[event.clone()]}), "")
                .1
                .is_empty());
        }
        let legacy: Incoming = serde_json::from_value(json!({
            "event_id":"old","is_private":true,"sender":"owner","chat":"chat","text":"hello",
            "reply":{"chat_id":"chat","thread_id":null,"context_token":null}
        }))
        .unwrap();
        assert_eq!(legacy.untranscribed_voice, 0);
    }
    #[test]
    fn lark_identity_requires_verified_bot_and_matching_selection() {
        let mut status = json!({"identity":"user","verified":true,"appId":"cli_test","brand":"feishu","identities":{"bot":{"status":"ready","available":true,"verified":true,"openId":"ou_bot"}}});
        let mut selected = json!({"identity":"bot","appId":"cli_test","brand":"feishu"});
        assert_eq!(
            verified_lark_identity(&status, &selected).unwrap(),
            "cli_test"
        );
        selected["appId"] = json!("cli_other");
        assert!(verified_lark_identity(&status, &selected).is_err());
        selected["appId"] = json!("cli_test");
        status["identities"]["bot"]["verified"] = json!(false);
        assert!(verified_lark_identity(&status, &selected).is_err());
        assert!(verified_lark_identity(&json!({"app_id":"cli_test"}), &selected).is_err());
    }
    #[test]
    fn rejects_credential_redirect() {
        assert!(validate_wechat_url("http://ilinkai.weixin.qq.com").is_err());
        assert!(validate_wechat_url("https://evil.com").is_err());
    }
    #[test]
    fn ignores_bot_events() {
        assert!(parse_lark(&json!({"event":{"sender":{"sender_type":"app"}}})).is_none());
    }
    #[test]
    fn parses_user_text() {
        let x=parse_lark(&json!({"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_1"}},"message":{"message_type":"text","chat_id":"oc_1","message_id":"om_1","content":"{\"text\":\"hello\"}"}}})).unwrap();
        assert_eq!(x.text, "hello");
    }
}
