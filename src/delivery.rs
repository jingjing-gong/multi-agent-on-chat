//! Durable delivery planning and checkpoints, independent of task execution.
use crate::{channels, domain::*, message::Message, store::Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Part {
    Message(Message),
    WechatRequest(serde_json::Value),
    RawFile(String),
    Attachment(std::path::PathBuf),
}
pub fn plan(bot: &Bot, target: &ReplyTarget, message: &Message, key: &str) -> Vec<Part> {
    if let Message::Bundle(parts) = message {
        return parts
            .iter()
            .enumerate()
            .flat_map(|(i, p)| plan(bot, target, p, &format!("{key}:{i}")))
            .collect();
    }
    if matches!(message, Message::Media { .. }) {
        if bot.channel == "wechat" && matches!(message, Message::Media {kind,..} if kind == "image")
        {
            return vec![
                Part::Message(Message::Plain("图片将以文件附件发送。".into())),
                Part::Message(message.clone()),
            ];
        }
        return vec![Part::Message(message.clone())];
    }
    if bot.channel != "wechat" {
        return vec![Part::Message(message.clone())];
    }
    if let Message::File(path) = message {
        return vec![Part::Attachment(path.clone())];
    }
    if let Message::Plain(text) = message {
        if text.len() > 3000 {
            return vec![Part::RawFile(text.clone())];
        }
    }
    channels::wechat_format::requests(message, target, key)
        .into_iter()
        .map(Part::WechatRequest)
        .collect()
}
pub async fn send(
    store: &Store,
    bot: &Bot,
    target: &ReplyTarget,
    message: &Message,
    key: &str,
) -> Result<()> {
    fn snapshot(store: &Store, bot: &Bot, message: &Message, key: &str) -> Result<Message> {
        match message {
            Message::File(path) => {
                let asset =
                    crate::media::import(store, &bot.id, "legacy-delivery", path, "legacy", None)?;
                crate::media::add_ref(store, &asset.id, "delivery", key)?;
                Ok(crate::media::message(&asset))
            }
            Message::Bundle(parts) => Ok(Message::Bundle(
                parts
                    .iter()
                    .map(|p| snapshot(store, bot, p, key))
                    .collect::<Result<Vec<_>>>()?,
            )),
            _ => Ok(message.clone()),
        }
    }
    let parts = match store.get::<Vec<Part>>("delivery_plan", key) {
        Ok(parts) => parts,
        Err(_) => {
            let parts = plan(bot, target, &snapshot(store, bot, message, key)?, key);
            store.save("delivery_plan", key, &parts)?;
            parts
        }
    };
    for (index, part) in parts.iter().enumerate() {
        let part_key = format!("{key}:part:{index}");
        if store
            .get::<bool>("delivery_part_sent", &part_key)
            .unwrap_or(false)
        {
            continue;
        }
        match part {
            Part::Message(Message::Media {
                asset_id,
                expires_at,
                ..
            }) => {
                anyhow::ensure!(expires_at.is_none_or(|e| e > now()), "media_expired");
                let asset = crate::media::get(store, asset_id)?;
                crate::media::check(&asset, &bot.id, None)?;
                let prepared = match store.get::<serde_json::Value>("delivery_upload", &part_key) {
                    Ok(v) => v,
                    Err(_) => {
                        crate::media::transfer(
                            store,
                            &part_key,
                            Some(asset_id),
                            "upload",
                            "running",
                            serde_json::json!({"started":now()}),
                        )?;
                        let v = channels::prepare_media(
                            bot,
                            target,
                            &asset.path,
                            &asset.name,
                            &asset.kind,
                            &part_key,
                        )
                        .await?;
                        store.save("delivery_upload", &part_key, &v)?;
                        crate::media::transfer(
                            store,
                            &part_key,
                            Some(asset_id),
                            "upload",
                            "succeeded",
                            serde_json::json!({"finished":now()}),
                        )?;
                        v
                    }
                };
                anyhow::ensure!(!store.get::<bool>("media_send_started", &part_key).unwrap_or(false), "delivery_unknown: previous send may have reached the channel; inspect before resending");
                store.save("media_send_started", &part_key, &true)?;
                if let Err(error) =
                    channels::send_prepared_media(bot, target, &prepared, &part_key).await
                {
                    let message = error.to_string();
                    if message.contains("permission_denied") || message.contains("unsupported") {
                        store.save("media_send_started", &part_key, &false)?;
                        return Err(error);
                    }
                    anyhow::bail!("delivery_unknown: channel response did not confirm send; inspect before resending");
                }
            }
            Part::Message(m) => channels::send_message(bot, target, m, &part_key).await?,
            Part::WechatRequest(request) => channels::wechat_send_request(bot, request).await?,
            Part::Attachment(path) => {
                let request = match store.get::<serde_json::Value>("delivery_upload", &part_key) {
                    Ok(request) => request,
                    Err(_) => {
                        let request =
                            channels::wechat_prepare_attachment(bot, target, path, &part_key)
                                .await?;
                        store.save("delivery_upload", &part_key, &request)?;
                        request
                    }
                };
                channels::wechat_send_request(bot, &request).await?;
            }
            Part::RawFile(text) => {
                let request = match store.get::<serde_json::Value>("delivery_upload", &part_key) {
                    Ok(request) => request,
                    Err(_) => {
                        let request = channels::wechat_prepare_file(bot, target, text, key).await?;
                        store.save("delivery_upload", &part_key, &request)?;
                        request
                    }
                };
                channels::wechat_send_request(bot, &request).await?;
            }
        }
        store.save("delivery_part_sent", &part_key, &true)?;
    }
    Ok(())
}
pub fn bot_reply(store: &Store, bot: &Bot) -> Result<Option<ReplyTarget>> {
    let Some(owner) = &bot.owner else {
        return Ok(None);
    };
    if bot.channel == "lark" {
        return Ok(Some(ReplyTarget {
            chat_id: owner.clone(),
            context_token: None,
            thread_id: None,
        }));
    }
    let raw: Option<String>=store.conn()?.query_row("SELECT target FROM deliveries WHERE bot_id=?1 AND json_extract(target,'$.chat_id')=?2 ORDER BY rowid DESC LIMIT 1",params![bot.id,owner],|r|r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
