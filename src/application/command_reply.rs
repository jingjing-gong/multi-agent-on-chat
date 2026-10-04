//! Decide whether an IM command already has a durable asynchronous reply owner.
use crate::{
    domain::{ReplyTarget, Task},
    store::Store,
};
use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;
#[derive(Debug, PartialEq, Eq)]
pub enum ReplyDisposition {
    Immediate,
    Deferred,
}
fn same(value: &Value, reply: &ReplyTarget) -> bool {
    serde_json::to_value(reply).is_ok_and(|target| value == &target)
}
pub fn disposition(
    store: &Store,
    command: &str,
    outcome: &Value,
    reply: &ReplyTarget,
) -> Result<ReplyDisposition> {
    use ReplyDisposition::*;
    if !outcome["error"].is_null() {
        return Ok(Immediate);
    }
    if outcome["reply_deferred"] == true {
        return Ok(Deferred);
    }
    let (command, _) = crate::presentation::split_raw(command);
    let mut words = command.split_whitespace();
    let cmd = words.next().unwrap_or_default();
    let action = words.next().unwrap_or_default();
    let id = outcome["id"].as_str().unwrap_or_default();
    let deferred = match cmd {
        "/sh" | "/retry" | "/compact" | "/ask" => {
            let task = store.get::<Task>("task", id).ok();
            let task = if cmd == "/ask" {
                task.and_then(|t| t.parent_id.and_then(|p| store.get::<Task>("task", &p).ok()))
            } else {
                task
            };
            task.is_some_and(|t| same(&serde_json::to_value(t.reply).unwrap_or(Value::Null), reply))
        }
        "/bot" if action == "add" && outcome["accepted"] == true => {
            let record = store
                .get::<Value>("bot_add", outcome["name"].as_str().unwrap_or_default())
                .ok();
            record.is_some_and(|v| same(&v["reply"], reply))
        }
        "/media" if matches!(action, "extract" | "retry-extraction") => {
            if outcome["cached"] == true
                || !matches!(outcome["state"].as_str(), Some("queued" | "running"))
            {
                false
            } else {
                let raw: Option<String> = store
                    .conn()?
                    .query_row(
                        "SELECT data FROM media_extraction_jobs WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()?;
                raw.and_then(|s| serde_json::from_str::<Value>(&s).ok())
                    .is_some_and(|v| same(&v["reply"], reply))
            }
        }
        "/media" if action == "retry" => {
            let raw: Option<String> = store
                .conn()?
                .query_row("SELECT data FROM media_inbox WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            raw.and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .is_some_and(|v| same(&v["message"]["reply"], reply))
        }
        _ => false,
    };
    Ok(if deferred { Deferred } else { Immediate })
}
