use super::*;
use crate::media::{self as assets, Asset};
use anyhow::Context;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
pub fn import_asset(
    store: &Store,
    scope: &Scope,
    path: &Path,
    bot: &str,
    conversation: &str,
) -> Result<Asset> {
    scope.bot(bot)?;
    assets::import(store, bot, conversation, path, "attachment", None)
}
pub fn list(store: &Store, scope: &Scope, bot: Option<&str>) -> Result<Value> {
    let bot = match scope {
        Scope::Bot(b) => {
            if let Some(id) = bot {
                scope.bot(id)?;
            }
            Some(b.as_str())
        }
        Scope::Local => bot,
    };
    Ok(
        json!({"assets":assets::list(store,bot,None)?.iter().map(assets::public).collect::<Vec<_>>() }),
    )
}
pub fn show(store: &Store, scope: &Scope, id: &str) -> Result<Value> {
    let a = assets::get(store, id)?;
    scope.bot(&a.bot_id)?;
    Ok(assets::public(&a))
}
pub fn export(store: &Store, scope: &Scope, id: &str, path: &Path) -> Result<Value> {
    let a = assets::get(store, id)?;
    scope.bot(&a.bot_id)?;
    assets::check(&a, &a.bot_id, None)?;
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::io::copy(&mut std::fs::File::open(&a.path)?, &mut out)?;
    out.sync_all()?;
    Ok(json!({"asset_id":id,"output":path}))
}
pub fn send(store: &Store, scope: &Scope, id: &str, bot: &str, chat: &str) -> Result<Value> {
    scope.bot(bot)?;
    anyhow::ensure!(
        matches!(scope, Scope::Local),
        "cross-chat media send requires local administrator"
    );
    let b: Bot = store.get("bot", bot)?;
    let a = assets::get(store, id)?;
    assets::check(&a, bot, None)?;
    let target = if b.channel == "wechat" {
        let r = crate::delivery::bot_reply(store, &b)?.context("no WeChat reply route")?;
        anyhow::ensure!(r.chat_id == chat, "WeChat target has no reply context");
        r
    } else {
        ReplyTarget {
            chat_id: chat.into(),
            thread_id: None,
            context_token: None,
        }
    };
    let delivery = assets::enqueue(store, bot, &target, &a, &new_id("media-send"))?;
    Ok(json!({"asset_id":id,"delivery_id":delivery,"state":"queued"}))
}
pub fn retry(store: &Store, scope: &Scope, id: &str) -> Result<Value> {
    assets::ingress::retry(store, scope, id)
}
pub fn submit(
    store: &Store,
    scope: &Scope,
    agent: &str,
    conversation: &str,
    prompt: &str,
    paths: &[PathBuf],
) -> Result<Value> {
    let a = scope.agent(store, agent)?;
    anyhow::ensure!(
        paths.len() <= assets::limits(store)?.attachments,
        "too many attachments"
    );
    if paths.is_empty() {
        return Ok(serde_json::to_value(store.submit(
            agent,
            conversation,
            prompt,
            None,
            None,
        )?)?);
    }
    let files = paths
        .iter()
        .map(|p| assets::import(store, &a.bot_id, conversation, p, "attachment", None))
        .collect::<Result<Vec<_>>>()?;
    let ids = files.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
    Ok(serde_json::to_value(store.submit_media(
        agent,
        conversation,
        prompt,
        None,
        &ids,
        None,
    )?)?)
}
pub fn gc(store: &Store, scope: &Scope, dry_run: bool) -> Result<Value> {
    anyhow::ensure!(
        matches!(scope, Scope::Local),
        "media GC requires local administrator"
    );
    let candidates = assets::list(store, None, None)?
        .into_iter()
        .filter(|a| matches!(a.state.as_str(), "available" | "expired"))
        .collect::<Vec<_>>();
    let mut removed = Vec::new();
    for mut a in candidates {
        let mut c = store.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let refs:i64=tx.query_row("SELECT count(*) FROM media_refs r WHERE asset_id=?1 AND (r.kind='pin' OR (r.kind='delivery' AND EXISTS(SELECT 1 FROM deliveries d WHERE d.id=r.ref_id AND d.state IN ('pending','unknown'))) OR (r.kind IN ('task_input','task_artifact') AND EXISTS(SELECT 1 FROM records t WHERE t.kind='task' AND t.id=r.ref_id AND json_extract(t.data,'$.state') IN ('queued','running','waiting_dependency','cancel_requested','interrupted'))) OR (r.kind='extraction' AND EXISTS(SELECT 1 FROM media_extraction_jobs j WHERE j.id=r.ref_id AND j.state IN ('queued','running','blocked'))) OR (r.kind='extraction_result' AND EXISTS(SELECT 1 FROM media_extraction_jobs j JOIN media_assets source ON source.id=j.asset_id WHERE j.id=r.ref_id AND json_extract(source.data,'$.state')='available')) OR (r.kind='inbox' AND EXISTS(SELECT 1 FROM media_inbox i WHERE i.id=r.ref_id AND i.state IN ('received','preparing','blocked'))))",[&a.id],|r|r.get(0))?;
        if refs > 0 && a.purpose != "auth_qr" {
            if !dry_run {
                tx.execute(
                    "DELETE FROM records WHERE kind='media_unreferenced' AND id=?1",
                    [&a.id],
                )?;
            }
            tx.commit()?;
            continue;
        }
        let since:Option<i64>=tx.query_row("SELECT CAST(data AS INTEGER) FROM records WHERE kind='media_unreferenced' AND id=?1",[&a.id],|r|r.get(0)).optional()?;
        let since = since.unwrap_or_else(now);
        if !dry_run {
            tx.execute(
                "INSERT OR IGNORE INTO records VALUES('media_unreferenced',?1,?2)",
                rusqlite::params![a.id, since.to_string()],
            )?;
        }
        let due = a.state == "expired"
            || if a.purpose == "auth_qr" {
                a.expires_at.is_some_and(|e| e + 3600 <= now())
            } else {
                since + 7 * 86400 <= now()
            };
        if !due {
            tx.commit()?;
            continue;
        }
        removed.push(a.id.clone());
        if !dry_run {
            a.state = "expired".into();
            tx.execute(
                "UPDATE media_assets SET data=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_string(&a)?, a.id],
            )?;
        }
        tx.commit()?;
        if !dry_run {
            // Hold the writer lock against import while unlinking. Expiry was already committed.
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let others:i64=tx.query_row("SELECT count(*) FROM media_assets WHERE id!=?1 AND json_extract(data,'$.path')=?2 AND json_extract(data,'$.state')='available'",rusqlite::params![a.id,a.path.to_string_lossy()],|r|r.get(0))?;
            if others == 0 {
                match std::fs::remove_file(&a.path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            tx.commit()?;
        }
    }
    Ok(json!({"dry_run":dry_run,"assets":removed}))
}
pub fn publish(store: &Store, task: &Task, path: &Path, caption: Option<&str>) -> Result<Value> {
    use crate::message::Message;
    use rusqlite::{params, OptionalExtension};
    use sha2::{Digest, Sha256};

    anyhow::ensure!(
        caption.is_none_or(|s| s.len() <= 16_384),
        "caption exceeds 16 KiB"
    );
    let session: Session = store.get("session", &task.session_id)?;
    let root = std::fs::canonicalize(&session.workspace)?;
    let path = std::fs::canonicalize(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })?;
    anyhow::ensure!(
        path.starts_with(root),
        "publish path must be inside task workspace; use explicit local import for external paths"
    );
    let current: Task = store.get("task", &task.id)?;
    anyhow::ensure!(
        current.generation == task.generation && current.state == "running",
        "task is no longer running"
    );
    let a = assets::import(store, &task.bot_id, &session.scope, &path, "artifact", None)?;
    // Import produces an immutable snapshot. Only a still-running generation may
    // attach it and enqueue publication; cancellation and publishing serialize on
    // this transaction. An import interrupted before this commit is GC eligible.
    let mut conn = store.conn()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let data: String = tx.query_row(
        "SELECT data FROM records WHERE kind='task' AND id=?1",
        [&task.id],
        |r| r.get(0),
    )?;
    let current: Task = serde_json::from_str(&data)?;
    anyhow::ensure!(
        current.generation == task.generation && current.state == "running",
        "task is no longer running"
    );
    let digest = Sha256::digest(serde_json::to_vec(&(&a.sha256, &a.name, caption))?);
    let key = format!("artifact:{}:{}:{digest:x}", task.id, task.generation);
    if let Some(data) = tx
        .query_row(
            "SELECT data FROM records WHERE kind='media_publication' AND id=?1",
            [&key],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        // Identical tool retries reuse the original asset and delivery receipt.
        tx.execute("DELETE FROM media_assets WHERE id=?1", [&a.id])?;
        tx.commit()?;
        return Ok(serde_json::from_str(&data)?);
    }
    tx.execute(
        "INSERT INTO media_refs VALUES(?1,'task_artifact',?2,?3)",
        params![a.id, task.id, now()],
    )?;
    let delivery = if let Some(target) = &current.reply {
        let id = new_id("d");
        let payload = if let Some(text) = caption.filter(|s| !s.is_empty()) {
            Message::Bundle(vec![Message::Plain(text.into()), assets::message(&a)])
        } else {
            assets::message(&a)
        };
        tx.execute(
            "INSERT INTO deliveries(id,task_id,bot_id,target,text) VALUES(?1,?2,?3,?4,?5)",
            params![id, key, a.bot_id, serde_json::to_string(target)?, a.name],
        )?;
        tx.execute(
            "INSERT INTO records(kind,id,data) VALUES('delivery_payload',?1,?2)",
            params![id, serde_json::to_string(&payload)?],
        )?;
        tx.execute(
            "INSERT INTO media_refs VALUES(?1,'delivery',?2,?3)",
            params![a.id, id, now()],
        )?;
        Some(id)
    } else {
        None
    };
    let result = json!({"asset":assets::public(&a),"caption":caption,"delivery_id":delivery});
    tx.execute(
        "INSERT INTO records(kind,id,data) VALUES('media_publication',?1,?2)",
        params![key, serde_json::to_string(&result)?],
    )?;
    tx.commit()?;
    Ok(result)
}
