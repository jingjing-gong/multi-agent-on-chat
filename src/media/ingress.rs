use super::*;
use crate::{application::Scope, channels::Incoming};
use serde_json::{json, Value};

pub fn register(
    store: &Store,
    bot: &Bot,
    agent: &str,
    scope: &str,
    message: &Incoming,
) -> Result<String> {
    let id = new_id("media");
    let mut c = store.conn()?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if tx.execute(
        "INSERT OR IGNORE INTO receipts VALUES(?1,?2)",
        params![bot.id, message.event_id],
    )? == 0
    {
        return Ok("duplicate".into());
    }
    tx.execute("INSERT INTO media_inbox(id,bot_id,event_id,scope,state,data,created) VALUES(?1,?2,?3,?4,'received',?5,?6)",params![id,bot.id,message.event_id,scope,json!({"agent":agent,"message":message,"assets":[]}).to_string(),now()])?;
    tx.commit()?;
    Ok(id)
}
pub fn retry(store: &Store, scope: &Scope, id: &str) -> Result<Value> {
    retry_with_reply(store, scope, id, None)
}
pub fn retry_with_reply(
    store: &Store,
    scope: &Scope,
    id: &str,
    reply: Option<ReplyTarget>,
) -> Result<Value> {
    let mut conn = store.conn()?;
    let c = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let (bot, state): (String, String) = c
        .query_row(
            "SELECT bot_id,state FROM media_inbox WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .context("media preparation not found")?;
    scope.bot(&bot)?;
    anyhow::ensure!(
        matches!(state.as_str(), "blocked" | "failed"),
        "media input is not retryable"
    );
    if let Some(reply) = reply {
        let raw: String = c.query_row("SELECT data FROM media_inbox WHERE id=?1", [id], |r| {
            r.get(0)
        })?;
        let mut data: Value = serde_json::from_str(&raw)?;
        data["message"]["reply"] = serde_json::to_value(reply)?;
        c.execute(
            "UPDATE media_inbox SET data=?2 WHERE id=?1",
            params![id, data.to_string()],
        )?;
    }
    c.execute(
        "UPDATE media_inbox SET state='received',attempts=0,next_at=0,lease_until=0 WHERE id=?1",
        [id],
    )?;
    c.commit()?;
    Ok(json!({"id":id,"state":"received"}))
}
pub fn pending(store: &Store, bot: &str, scope: &str) -> Result<Vec<Value>> {
    let c = store.conn()?;
    let mut q=c.prepare("SELECT id,state,attempts,data FROM media_inbox WHERE bot_id=?1 AND scope=?2 ORDER BY created DESC LIMIT 30")?;
    let rows=q.query_map(params![bot,scope],|r|{let data:String=r.get(3)?;let v:Value=serde_json::from_str(&data).unwrap_or_default();Ok(json!({"id":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"attempts":r.get::<_,i64>(2)?,"error":v["error"],"task_id":v["task_id"]}))})?.collect::<std::result::Result<Vec<_>,_>>()?;
    Ok(rows)
}
pub async fn work_once(store: &Store) -> Result<bool> {
    let claim = {
        let mut c = store.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row:Option<(String,String,String,String,i64,i64)>=tx.query_row("SELECT id,bot_id,scope,data,generation,attempts FROM media_inbox WHERE ((state='received' AND next_at<=?1) OR (state='preparing' AND lease_until<?1)) AND NOT EXISTS(SELECT 1 FROM media_inbox earlier WHERE earlier.bot_id=media_inbox.bot_id AND earlier.scope=media_inbox.scope AND earlier.rowid<media_inbox.rowid AND earlier.state IN ('received','preparing')) AND (SELECT count(*) FROM media_inbox active WHERE active.bot_id=media_inbox.bot_id AND active.state='preparing' AND active.lease_until>=?1)<2 ORDER BY rowid LIMIT 1",[now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        if let Some((id, bot, scope, data, generation, attempts)) = row {
            tx.execute("UPDATE media_inbox SET state='preparing',generation=generation+1,attempts=attempts+1,lease_until=?1 WHERE id=?2",params![now()+1500,id])?;
            tx.commit()?;
            Some((id, bot, scope, data, generation + 1, attempts + 1))
        } else {
            None
        }
    };
    let Some((id, bot_id, scope, raw, generation, attempts)) = claim else {
        return Ok(false);
    };
    let mut data: Value = serde_json::from_str(&raw)?;
    let message: Incoming = serde_json::from_value(data["message"].clone())?;
    let result = prepare(store, &id, &bot_id, &scope, &message, &mut data, generation).await;
    if let Err(e) = result {
        let error = e.to_string();
        let temporary = error.contains("timeout")
            || error.contains("connect")
            || error.contains("429")
            || error.contains("502")
            || error.contains("503");
        let state = if temporary && attempts < 5 {
            "received"
        } else {
            "blocked"
        };
        data["error"] = json!(error.chars().take(300).collect::<String>());
        let updated=store.conn()?.execute("UPDATE media_inbox SET state=?1,data=?2,next_at=?3,lease_until=0 WHERE id=?4 AND generation=?5 AND state='preparing'",params![state,data.to_string(),now()+30*attempts,id,generation])?;
        if state == "blocked" && updated > 0 {
            notice(
                store,
                &bot_id,
                &message.reply,
                &format!("media-error:{id}:{generation}"),
                &format!(
                    "附件准备失败，未提交模型。{}
使用 /media retry {} 重试，或重新发送附件。",
                    data["error"].as_str().unwrap_or("unknown"),
                    id
                ),
            )?;
        }
    }
    Ok(true)
}
fn notice(store: &Store, bot: &str, target: &ReplyTarget, key: &str, text: &str) -> Result<()> {
    store.conn()?.execute(
        "INSERT OR IGNORE INTO deliveries(id,task_id,bot_id,target,text) VALUES(?1,?2,?3,?4,?5)",
        params![new_id("d"), key, bot, serde_json::to_string(target)?, text],
    )?;
    Ok(())
}
async fn prepare(
    store: &Store,
    id: &str,
    bot_id: &str,
    scope: &str,
    message: &Incoming,
    data: &mut Value,
    generation: i64,
) -> Result<()> {
    let bot: Bot = store.get("bot", bot_id)?;
    anyhow::ensure!(
        bot.enabled && bot.owner.as_deref() == Some(message.sender.as_str()),
        "media owner revoked"
    );
    let limits = limits(store)?;
    let hydrated = crate::channels::media::hydrate(&bot, message).await?;
    let message = &hydrated;
    data["message"] = serde_json::to_value(message)?;
    anyhow::ensure!(
        message.attachments.len() <= limits.attachments,
        "too many attachments"
    );
    let mut ids: Vec<String> = serde_json::from_value(data["assets"].clone())?;
    for (index, attachment) in message.attachments.iter().enumerate().skip(ids.len()) {
        let dir = store
            .root
            .join("media/tmp")
            .join(format!("{id}-{generation}-{index}"));
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let destination = dir.join("attachment");
        let transfer_id = format!("{id}:{index}");
        transfer(
            store,
            &transfer_id,
            None,
            "download",
            "running",
            json!({"generation":generation,"started":now()}),
        )?;
        let result = async {
            crate::channels::download_attachment(&bot, attachment, &destination, limits.file_bytes)
                .await?;
            let mut asset = import(store, bot_id, scope, &destination, "inbound", None)?;
            anyhow::ensure!(
                attachment.kind != "image" || asset.kind == "image",
                "media_type_mismatch: image payload is not a supported image"
            );
            asset.name = clean_name(&attachment.name);
            store.conn()?.execute(
                "UPDATE media_assets SET data=?1 WHERE id=?2",
                params![serde_json::to_string(&asset)?, asset.id],
            )?;
            Ok::<_, anyhow::Error>(asset)
        }
        .await;
        let _ = std::fs::remove_dir_all(&dir);
        if result.is_err() {
            transfer(
                store,
                &transfer_id,
                None,
                "download",
                "failed",
                json!({"generation":generation,"finished":now()}),
            )?;
        }
        let asset = result?;
        transfer(
            store,
            &transfer_id,
            Some(&asset.id),
            "download",
            "succeeded",
            json!({"generation":generation,"finished":now(),"size":asset.size}),
        )?;
        add_ref(store, &asset.id, "inbox", id)?;
        ids.push(asset.id);
        data["assets"] = json!(ids);
        anyhow::ensure!(store.conn()?.execute("UPDATE media_inbox SET data=?1 WHERE id=?2 AND generation=?3 AND state='preparing'",params![data.to_string(),id,generation])?>0,"media input cancelled or lease lost");
    }
    let agent = data["agent"].as_str().context("missing media agent")?;
    store.submit_media(
        agent,
        scope,
        &message.text,
        Some(message.reply.clone()),
        &ids,
        Some((id, generation)),
    )?;
    Ok(())
}
