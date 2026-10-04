//! Durable, scoped parser jobs. No model worker waits for a parser process.
use super::{
    extractors::{self, Options},
    *,
};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub asset_id: String,
    pub bot_id: String,
    pub scope: String,
    pub parser: String,
    pub options: Options,
    pub state: String,
    pub attempts: i64,
    pub generation: i64,
    pub created: i64,
    pub finished: Option<i64>,
    pub result_asset_id: Option<String>,
    pub error: Option<Value>,
    pub reply: Option<ReplyTarget>,
    pub task_id: Option<String>,
}
pub fn migrate(c: &rusqlite::Connection) -> Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS media_extraction_jobs(id TEXT PRIMARY KEY,asset_id TEXT NOT NULL,bot_id TEXT NOT NULL,scope TEXT NOT NULL,cache_key TEXT NOT NULL,state TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,generation INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0,lease_until INTEGER NOT NULL DEFAULT 0,data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS extraction_queue ON media_extraction_jobs(state,next_at);
CREATE INDEX IF NOT EXISTS extraction_cache ON media_extraction_jobs(cache_key,state);
CREATE TRIGGER IF NOT EXISTS media_ref_resets_retention AFTER INSERT ON media_refs BEGIN DELETE FROM records WHERE kind='media_unreferenced' AND id=NEW.asset_id; END;")?;
    Ok(())
}
fn get_job(store: &Store, id: &str) -> Result<Job> {
    let raw: String = store
        .conn()?
        .query_row(
            "SELECT data FROM media_extraction_jobs WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .context("extraction_not_found")?;
    Ok(serde_json::from_str(&raw)?)
}
fn authorized(store: &Store, id: &str, bot: Option<&str>, scope: Option<&str>) -> Result<Job> {
    let job = get_job(store, id)?;
    anyhow::ensure!(
        bot.is_none_or(|b| b == job.bot_id) && scope.is_none_or(|s| s == job.scope),
        "media_forbidden: extraction belongs to another conversation"
    );
    let asset = get(store, &job.asset_id)?;
    check(&asset, &job.bot_id, Some(&job.scope))?;
    anyhow::ensure!(
        asset.purpose != "auth_qr",
        "media_forbidden: authentication QR cannot be parsed"
    );
    Ok(job)
}
fn public_job(j: &Job) -> Value {
    json!({"schema_version":1,"id":j.id,"source_asset_id":j.asset_id,"parser":j.parser,"options":j.options,"state":j.state,"attempts":j.attempts,"created":j.created,"finished":j.finished,"error":j.error,"result_available":j.result_asset_id.is_some()})
}
pub fn status(store: &Store, id: &str, bot: Option<&str>, scope: Option<&str>) -> Result<Value> {
    Ok(public_job(&authorized(store, id, bot, scope)?))
}

pub async fn request(
    store: &Store,
    asset_id: &str,
    bot: Option<&str>,
    scope: Option<&str>,
    options: Options,
    reply: Option<ReplyTarget>,
    task_id: Option<&str>,
) -> Result<Value> {
    let asset = get(store, asset_id)?;
    check(&asset, bot.unwrap_or(&asset.bot_id), scope)?;
    anyhow::ensure!(
        asset.purpose != "auth_qr",
        "media_forbidden: authentication QR cannot be parsed"
    );
    anyhow::ensure!(
        options.start_page >= 1
            && options
                .end_page
                .is_none_or(|e| e >= options.start_page && e - options.start_page < 50),
        "invalid_page_range: at most 50 pages per request"
    );
    let parser = extractors::identity(&asset).await?;
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!([
            1,
            asset.bot_id,
            asset.scope,
            asset.sha256,
            parser,
            options
        ]))?)
    );
    let mut c = store.conn()?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(id) = task_id {
        let raw: String = tx.query_row(
            "SELECT data FROM records WHERE kind='task' AND id=?1",
            [id],
            |r| r.get(0),
        )?;
        let task: Task = serde_json::from_str(&raw)?;
        anyhow::ensure!(
            task.bot_id == asset.bot_id && task.state == "running",
            "task no longer running"
        );
        let raw: String = tx.query_row(
            "SELECT data FROM records WHERE kind='session' AND id=?1",
            [&task.session_id],
            |r| r.get(0),
        )?;
        let session: Session = serde_json::from_str(&raw)?;
        anyhow::ensure!(
            session.scope == asset.scope && session.state != "closed",
            "session no longer active"
        );
    }
    let cached:Option<String>=tx.query_row("SELECT data FROM media_extraction_jobs WHERE cache_key=?1 AND state IN ('queued','running','succeeded') ORDER BY rowid DESC LIMIT 1",[&key],|r|r.get(0)).optional()?;
    if let Some(raw) = cached {
        let j: Job = serde_json::from_str(&raw)?;
        // Reuse only the same source grant; a matching hash never grants a different file's access.
        if j.asset_id == asset_id
            && check(&get(store, &j.asset_id)?, &asset.bot_id, Some(&asset.scope)).is_ok()
            && j.result_asset_id.as_ref().is_none_or(|id| {
                get(store, id)
                    .and_then(|a| check(&a, &asset.bot_id, Some(&asset.scope)))
                    .is_ok()
            })
        {
            let mut result = public_job(&j);
            result["cached"] = json!(true);
            return Ok(result);
        }
    }
    let count:i64=tx.query_row("SELECT count(*) FROM media_extraction_jobs WHERE state IN ('queued','running') AND bot_id=?1",[&asset.bot_id],|r|r.get(0))?;
    let total: i64 = tx.query_row(
        "SELECT count(*) FROM media_extraction_jobs WHERE state IN ('queued','running')",
        [],
        |r| r.get(0),
    )?;
    anyhow::ensure!(count < 32 && total < 128, "extraction_queue_full");
    let j = Job {
        id: new_id("extract"),
        asset_id: asset_id.into(),
        bot_id: asset.bot_id,
        scope: asset.scope,
        parser,
        options,
        state: "queued".into(),
        attempts: 0,
        generation: 0,
        created: now(),
        finished: None,
        result_asset_id: None,
        error: None,
        reply,
        task_id: task_id.map(str::to_owned),
    };
    tx.execute("INSERT INTO media_extraction_jobs(id,asset_id,bot_id,scope,cache_key,state,data) VALUES(?1,?2,?3,?4,?5,'queued',?6)",params![j.id,j.asset_id,j.bot_id,j.scope,key,serde_json::to_string(&j)?])?;
    tx.execute(
        "INSERT INTO media_refs VALUES(?1,'extraction',?2,?3)",
        params![j.asset_id, j.id, now()],
    )?;
    tx.commit()?;
    Ok(public_job(&j))
}
pub fn result(
    store: &Store,
    id: &str,
    bot: Option<&str>,
    scope: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    anyhow::ensure!(
        (1..=16000).contains(&limit),
        "result limit must be 1..16000 characters"
    );
    let job = authorized(store, id, bot, scope)?;
    anyhow::ensure!(
        job.state == "succeeded",
        "extraction_not_ready: query status for details"
    );
    let asset = get(
        store,
        job.result_asset_id
            .as_deref()
            .context("extraction_result_missing")?,
    )?;
    check(&asset, &job.bot_id, Some(&job.scope))?;
    let mut bytes = Vec::new();
    std::fs::File::open(&asset.path)?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "extraction_result_exceeds_budget"
    );
    let data: Value = serde_json::from_slice(&bytes)?;
    let mut skipped = 0;
    let mut used = 0;
    let mut segments = vec![];
    let mut total = 0;
    for segment in data["segments"]
        .as_array()
        .context("invalid extraction result")?
    {
        let text = segment["text"].as_str().unwrap_or("");
        let n = text.chars().count();
        total += n;
        if skipped + n <= offset {
            skipped += n;
            continue;
        }
        let start = offset.saturating_sub(skipped);
        let length = (n - start).min(limit - used);
        if length > 0 {
            let mut part = segment.clone();
            part["text"] = json!(text.chars().skip(start).take(length).collect::<String>());
            part["segment_offset"] = json!(start);
            segments.push(part);
            used += length;
        }
        skipped += n;
    }
    let mut out = public_job(&job);
    out["segments"] = json!(segments);
    out["offset"] = json!(offset);
    out["next_offset"] = if offset + used < total {
        json!(offset + used)
    } else {
        Value::Null
    };
    out["total_characters"] = json!(total);
    for key in ["coverage", "warnings", "truncated", "schema_version"] {
        if let Some(v) = data.get(key) {
            out[key] = v.clone();
        }
    }
    out["content_trust"] = json!("user_attachment_data");
    Ok(out)
}
pub fn cancel(store: &Store, id: &str, bot: Option<&str>, scope: Option<&str>) -> Result<Value> {
    let mut j = authorized(store, id, bot, scope)?;
    anyhow::ensure!(
        matches!(
            j.state.as_str(),
            "queued" | "running" | "blocked" | "failed"
        ),
        "extraction is terminal"
    );
    let previous = j.generation;
    j.generation += 1;
    j.state = "cancelled".into();
    j.finished = Some(now());
    let changed=store.conn()?.execute("UPDATE media_extraction_jobs SET state='cancelled',generation=?2,lease_until=0,data=?3 WHERE id=?1 AND generation=?4 AND state IN ('queued','running','blocked','failed')",params![id,j.generation,serde_json::to_string(&j)?,previous])?;
    anyhow::ensure!(changed == 1, "extraction_state_changed");
    Ok(public_job(&j))
}
pub fn retry(store: &Store, id: &str, bot: Option<&str>, scope: Option<&str>) -> Result<Value> {
    retry_with_reply(store, id, bot, scope, None)
}
pub fn retry_with_reply(
    store: &Store,
    id: &str,
    bot: Option<&str>,
    scope: Option<&str>,
    reply: Option<ReplyTarget>,
) -> Result<Value> {
    let mut j = authorized(store, id, bot, scope)?;
    anyhow::ensure!(
        matches!(j.state.as_str(), "blocked" | "failed"),
        "extraction is not retryable"
    );
    let previous = j.generation;
    j.generation += 1;
    j.state = "queued".into();
    if reply.is_some() {
        j.reply = reply;
    }
    j.error = None;
    j.finished = None;
    let changed=store.conn()?.execute("UPDATE media_extraction_jobs SET state='queued',generation=?2,lease_until=0,next_at=0,data=?3 WHERE id=?1 AND generation=?4 AND state IN ('blocked','failed')",params![id,j.generation,serde_json::to_string(&j)?,previous])?;
    anyhow::ensure!(changed == 1, "extraction_state_changed");
    Ok(public_job(&j))
}
pub fn recover(store: &Store) -> Result<()> {
    let c = store.conn()?;
    let mut q = c.prepare("SELECT data FROM media_extraction_jobs WHERE state='running'")?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for raw in rows {
        let mut j: Job = serde_json::from_str(&raw)?;
        j.state = "queued".into();
        j.generation += 1;
        c.execute("UPDATE media_extraction_jobs SET state='queued',generation=?2,lease_until=0,next_at=0,data=?3 WHERE id=?1 AND state='running'",params![j.id,j.generation,serde_json::to_string(&j)?])?;
    }
    Ok(())
}
pub fn cancel_scope(store: &Store, bot: &str, scope: &str) -> Result<()> {
    let c = store.conn()?;
    let mut q=c.prepare("SELECT id FROM media_extraction_jobs WHERE bot_id=?1 AND scope=?2 AND state IN ('queued','running','blocked')")?;
    let ids = q
        .query_map(params![bot, scope], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        cancel(store, &id, Some(bot), Some(scope))?;
    }
    Ok(())
}
fn is_cancelled(store: &Store, j: &Job) -> Result<bool> {
    let current = get_job(store, &j.id)?;
    if current.state != "running" || current.generation != j.generation {
        return Ok(true);
    }
    if let Some(id) = &j.task_id {
        let task: Task = store.get("task", id)?;
        let session: Session = store.get("session", &task.session_id)?;
        if matches!(task.state.as_str(), "cancelled" | "cancel_requested")
            || session.state == "closed"
        {
            let _ = cancel(store, &j.id, Some(&j.bot_id), Some(&j.scope));
            return Ok(true);
        }
    }
    let bot: Bot = store.get("bot", &j.bot_id)?;
    if !bot.enabled {
        let _ = cancel(store, &j.id, Some(&j.bot_id), Some(&j.scope));
        return Ok(true);
    }
    Ok(false)
}
pub async fn work_once(store: &Store) -> Result<bool> {
    let mut job = {
        let mut c = store.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw:Option<String>=tx.query_row("SELECT data FROM media_extraction_jobs j WHERE ((state='queued' AND next_at<=?1) OR (state='running' AND lease_until<?1)) AND NOT EXISTS(SELECT 1 FROM media_extraction_jobs a WHERE a.bot_id=j.bot_id AND a.state='running' AND a.lease_until>=?1) ORDER BY rowid LIMIT 1",[now()],|r|r.get(0)).optional()?;
        let Some(raw) = raw else { return Ok(false) };
        let mut j: Job = serde_json::from_str(&raw)?;
        j.state = "running".into();
        j.generation += 1;
        j.attempts += 1;
        tx.execute("UPDATE media_extraction_jobs SET state='running',generation=?2,attempts=?3,lease_until=?4,data=?5 WHERE id=?1",params![j.id,j.generation,j.attempts,now()+180,serde_json::to_string(&j)?])?;
        tx.commit()?;
        j
    };
    let run = async {
        let asset = get(store, &job.asset_id)?;
        check(&asset, &job.bot_id, Some(&job.scope))?;
        anyhow::ensure!(
            asset.purpose != "auth_qr",
            "authentication QR cannot be parsed"
        );
        let parser = extractors::identity(&asset).await?;
        anyhow::ensure!(
            parser == job.parser,
            "parser_version_changed: request a new extraction"
        );
        let parsed = extractors::extract(&asset, &job.options).await?;
        Ok::<Value, anyhow::Error>(serde_json::to_value(parsed)?)
    };
    tokio::pin!(run);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    let parsed = loop {
        tokio::select! {result=&mut run=>break result,_=tick.tick()=>{if is_cancelled(store,&job)?{return Ok(true);}}}
    };
    let parsed = match parsed {
        Ok(value) => Ok(value),
        Err(error) => {
            let detail=error.downcast_ref::<extractors::MediaError>().map(|e|serde_json::to_value(e).unwrap_or(Value::Null)).unwrap_or(json!({"code":"extraction_failed","message":"source unavailable or extraction failed","retryable":false}));
            Err(detail)
        }
    };
    let mut parsed = parsed;
    let result_asset = if let Ok(value) = &parsed {
        let persist = (|| -> Result<Asset> {
            let directory = store.root.join("media/tmp");
            std::fs::create_dir_all(&directory)?;
            let path = directory.join(format!("{}-{}.json", job.id, job.generation));
            let bytes = serde_json::to_vec(value)?;
            anyhow::ensure!(
                bytes.len() <= 2 * 1024 * 1024,
                "extraction_result_exceeds_budget"
            );
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            let imported = import(
                store,
                &job.bot_id,
                &job.scope,
                &path,
                "extraction_result",
                None,
            );
            let _ = std::fs::remove_file(path);
            imported
        })();
        match persist {
            Ok(a) => Some(a),
            Err(_) => {
                parsed = Err(
                    json!({"code":"result_storage_failed","phase":"persist","message":"cannot store extraction result; check media quota and disk","retryable":false}),
                );
                None
            }
        }
    } else {
        None
    };
    if is_cancelled(store, &job)? {
        return Ok(true);
    }
    let mut c = store.conn()?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let valid:i64=tx.query_row("SELECT count(*) FROM media_extraction_jobs WHERE id=?1 AND state='running' AND generation=?2",params![job.id,job.generation],|r|r.get(0))?;
    if valid == 0 {
        return Ok(true);
    }
    if let Some(task_id) = &job.task_id {
        let raw: String = tx.query_row(
            "SELECT data FROM records WHERE kind='task' AND id=?1",
            [task_id],
            |r| r.get(0),
        )?;
        let task: Task = serde_json::from_str(&raw)?;
        let raw: String = tx.query_row(
            "SELECT data FROM records WHERE kind='session' AND id=?1",
            [&task.session_id],
            |r| r.get(0),
        )?;
        let session: Session = serde_json::from_str(&raw)?;
        if matches!(task.state.as_str(), "cancelled" | "cancel_requested")
            || session.state == "closed"
        {
            job.state = "cancelled".into();
            job.generation += 1;
            job.finished = Some(now());
            tx.execute("UPDATE media_extraction_jobs SET state='cancelled',generation=?2,lease_until=0,data=?3 WHERE id=?1",params![job.id,job.generation,serde_json::to_string(&job)?])?;
            tx.commit()?;
            return Ok(true);
        }
    }
    job.finished = Some(now());
    job.error = parsed.err();
    job.state = if result_asset.is_some() {
        "succeeded"
    } else if job
        .error
        .as_ref()
        .is_some_and(|e| e["code"] == "dependency_missing")
    {
        "blocked"
    } else {
        "failed"
    }
    .into();
    if let Some(asset) = result_asset {
        job.result_asset_id = Some(asset.id.clone());
        tx.execute(
            "INSERT OR IGNORE INTO media_refs VALUES(?1,'extraction_result',?2,?3)",
            params![asset.id, job.id, now()],
        )?;
    }
    tx.execute(
        "UPDATE media_extraction_jobs SET state=?2,lease_until=0,data=?3 WHERE id=?1",
        params![job.id, job.state, serde_json::to_string(&job)?],
    )?;
    if let Some(target) = &job.reply {
        let text = if job.state == "succeeded" {
            format!(
                "附件解析完成：{}。使用 /media result {} 查看带页码的内容。",
                job.id, job.id
            )
        } else {
            format!(
                "附件解析未完成：{}（{}）。使用 /media status {} 查看原因。",
                job.id,
                job.error
                    .as_ref()
                    .and_then(|e| e["code"].as_str())
                    .unwrap_or("extraction_failed"),
                job.id
            )
        };
        tx.execute("INSERT OR IGNORE INTO deliveries(id,task_id,bot_id,target,text) VALUES(?1,?2,?3,?4,?5)",params![new_id("d"),format!("extraction:{}:{}",job.id,job.generation),job.bot_id,serde_json::to_string(target)?,text])?;
    }
    tx.commit()?;
    Ok(true)
}
