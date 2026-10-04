//! Bot-scoped reply mode configuration shared by commands and delivery.
use crate::{domain::Bot, store::Store};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde_json::{json, Map, Value};
use std::io::Write;
fn read(store: &Store) -> Result<Map<String, Value>> {
    let path = store.root.join("reply-streaming.json");
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)?
            .as_object()
            .cloned()
            .context("回复模式配置必须是 JSON 对象；请先修复文件"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e.into()),
    }
}
pub fn default_mode(bot: &Bot) -> &'static str {
    if bot.channel == "lark" {
        "card"
    } else {
        "off"
    }
}
fn resolve(bot: &Bot, configured: Option<&Value>) -> &'static str {
    match configured {
        None => default_mode(bot),
        Some(Value::String(s))
            if s == "progress" && matches!(bot.channel.as_str(), "lark" | "wechat") =>
        {
            "progress"
        }
        Some(Value::String(s)) if s == "card" && bot.channel == "lark" => "card",
        _ => "off",
    }
}
pub fn mode(store: &Store, bot: &Bot) -> String {
    // Malformed configuration fails closed; command writes never replace it.
    read(store)
        .map(|c| resolve(bot, c.get(&bot.id)).to_owned())
        .unwrap_or_else(|_| "off".into())
}
pub fn command(store: &Store, bot: &Bot, action: &str) -> Result<Value> {
    if !matches!(action, "" | "off" | "progress" | "card" | "default") {
        bail!("用法：/stream [card | progress | off | default]");
    }
    if action == "card" && bot.channel != "lark" {
        bail!("card 流式卡片仅支持飞书；微信可使用 progress 或 off");
    }
    if action == "progress" && !matches!(bot.channel.as_str(), "lark" | "wechat") {
        bail!("当前渠道不支持 progress");
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(store.root.join("reply-streaming.lock"))?;
    lock.lock_exclusive()?;
    let mut config = read(store)?;
    if !action.is_empty() {
        if action == "default" {
            config.remove(&bot.id);
        } else {
            config.insert(bot.id.clone(), json!(action));
        }
        let path = store.root.join("reply-streaming.json");
        let tmp = store.root.join(format!(
            ".reply-streaming-{}.tmp",
            crate::domain::new_id("write")
        ));
        let result = (|| -> Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            file.write_all(&serde_json::to_vec_pretty(&config)?)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result?;
    }
    let configured = config.get(&bot.id);
    Ok(
        json!({"bot_id":bot.id,"channel":bot.channel,"mode":resolve(bot,configured),"configured":configured,
        "source":if configured.is_some(){"显式配置"}else{"渠道默认"},"default":default_mode(bot),
        "notice":if action.is_empty(){"仅作用于当前 Bot；/stream card|progress|off|default 切换"}else{"设置已保存，无需重启；新任务使用此模式。切换 off/progress 会关闭现有流式卡片，最终结果按投递规则发送。"}}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bot(id: &str, channel: &str) -> Bot {
        Bot {
            id: id.into(),
            channel: channel.into(),
            account_id: id.into(),
            credential_ref: None,
            owner: None,
            enabled: true,
        }
    }
    #[test]
    fn concurrent_scoped_writes_defaults_and_invalid_file() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(d.path()).unwrap();
        std::thread::scope(|threads| {
            for i in 0..12 {
                let s = store.clone();
                threads.spawn(move || {
                    command(&s, &bot(&format!("b{i}"), "lark"), "progress").unwrap()
                });
            }
        });
        assert_eq!(read(&store).unwrap().len(), 12);
        assert_eq!(
            command(&store, &bot("b0", "lark"), "default").unwrap()["mode"],
            "card"
        );
        assert_eq!(mode(&store, &bot("b1", "lark")), "progress");
        assert!(command(&store, &bot("wx", "wechat"), "card").is_err());
        assert_eq!(mode(&store, &bot("wx", "wechat")), "off");
        let path = d.path().join("reply-streaming.json");
        std::fs::write(&path, "{broken").unwrap();
        assert!(command(&store, &bot("b0", "lark"), "card").is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{broken");
    }
}
