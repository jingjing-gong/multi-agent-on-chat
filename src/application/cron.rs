use super::*;
use anyhow::{bail, Context};
use rusqlite::params;
use serde_json::{json, Value};
pub fn propose(
    store: &Store,
    scope: &Scope,
    agent: &str,
    schedule: &str,
    timezone: &str,
    prompt: &str,
    reply: Option<ReplyTarget>,
) -> Result<Value> {
    let a = scope.agent(store, agent)?;
    anyhow::ensure!(!prompt.trim().is_empty(), "定时任务指令不能为空");
    let next = crate::schedule::next(schedule, timezone, now())?.context("no future firing")?;
    let id = new_id("draft");
    let draft = json!({"id":id,"agent":a,"schedule":schedule,"timezone":timezone,"prompt":prompt,"next_fire":next,"expires_at":now()+900,"status":"pending","reply":reply});
    store.save("draft", &id, &draft)?;
    Ok(json!({"created":false,"draft":draft,"reply":"confirm / 确认 in a later message"}))
}
pub fn action(store: &Store, scope: &Scope, id: &str, action: &str) -> Result<Job> {
    let job: Job = store.get("job", id)?;
    scope.bot(&job.bot_id)?;
    store.job_action(id, action)
}
pub fn cancel_draft(store: &Store, id: &str) -> Result<Value> {
    let mut c = store.conn()?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let raw: String = tx.query_row(
        "SELECT data FROM records WHERE kind='draft' AND id=?1",
        [id],
        |r| r.get(0),
    )?;
    let mut d: Value = serde_json::from_str(&raw)?;
    if d["status"] != "pending" {
        bail!("draft is not pending");
    }
    d["status"] = json!("cancelled");
    tx.execute(
        "UPDATE records SET data=?1 WHERE kind='draft' AND id=?2",
        params![d.to_string(), id],
    )?;
    tx.commit()?;
    Ok(json!({"created":false,"cancelled":id}))
}

pub fn confirm_draft(store: &Store, id: &str) -> Result<Value> {
    let mut c = store.conn()?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let raw: String = tx.query_row(
        "SELECT data FROM records WHERE kind='draft' AND id=?1",
        [id],
        |r| r.get(0),
    )?;
    let mut d: Value = serde_json::from_str(&raw)?;
    if d["status"] == "confirmed" {
        return Ok(json!({"created":false,"job_id":d["job_id"],"already_confirmed":true}));
    }
    if d["status"] != "pending" || d["expires_at"].as_i64().unwrap_or(0) <= now() {
        bail!("draft expired or not pending");
    }
    let a: Agent = serde_json::from_value(d["agent"].clone())?;
    let current: Agent = store.get("agent", &a.id)?;
    let b: Bot = store.get("bot", &a.bot_id)?;
    if !current.enabled || !b.enabled || current.bot_id != a.bot_id {
        bail!("draft target unavailable");
    }
    let spec = d["schedule"].as_str().context("invalid schedule")?;
    let tz = d["timezone"].as_str().context("invalid timezone")?;
    let next = crate::schedule::next(spec, tz, now())?.context("no future firing")?;
    let j = Job {
        id: format!("job_{id}"),
        agent_id: a.id,
        bot_id: a.bot_id,
        prompt: d["prompt"].as_str().unwrap_or_default().into(),
        schedule: spec.into(),
        timezone: tz.into(),
        config: a.config,
        config_revision: a.revision,
        next_fire: Some(next),
        state: "enabled".into(),
        reply: d
            .get("reply")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?,
        revision: 1,
        workspace: a.workspace,
    };
    tx.execute(
        "INSERT INTO records VALUES('job',?1,?2)",
        params![j.id, serde_json::to_string(&j)?],
    )?;
    d["status"] = json!("confirmed");
    d["job_id"] = json!(j.id);
    tx.execute(
        "UPDATE records SET data=?1 WHERE kind='draft' AND id=?2",
        params![d.to_string(), id],
    )?;
    tx.commit()?;
    Ok(json!({"created":true,"job":j}))
}
