//! Shared durable reply projection. Only the transport differs by channel.
use crate::{domain::*, store::Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Desired {
    key: String,
    task: String,
    generation: u32,
    bot: String,
    reply: ReplyTarget,
    progress: bool,
    text: String,
    sealed: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Transport {
    card: Option<String>,
    message: Option<String>,
    sent: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    layout_pending: bool,
    sequence: u32,
    closed: bool,
    failed: bool,
    attempts: u32,
    next_at: i64,
    pending: Option<Operation>,
    #[serde(default)]
    allocated_sequence: u32,
    #[serde(default)]
    recovery_attempted: bool,
    #[serde(default)]
    last_error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Operation {
    text: String,
    close: bool,
    #[serde(default)]
    kind: Option<OperationKind>,
    sequence: u32,
    uuid: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OperationKind {
    Send,
    Content,
    Summary,
    Layout,
    Open,
    Close,
}
#[derive(Default)]
struct Item {
    phase: Option<String>,
    text: String,
    complete: bool,
}
pub struct Collector {
    mode: String,
    items: BTreeMap<String, Item>,
    order: Vec<String>,
    dirty: bool,
}
fn mode(store: &Store, bot: &Bot) -> String {
    crate::application::stream::mode(store, bot)
}
impl Collector {
    pub fn new(store: &Store, task: &Task) -> Result<Self> {
        let bot: Bot = store.get("bot", &task.bot_id)?;
        Ok(Self {
            mode: if task.reply.is_some() {
                mode(store, &bot)
            } else {
                "off".into()
            },
            items: BTreeMap::new(),
            order: Vec::new(),
            dirty: false,
        })
    }
    fn item(&mut self, id: &str) -> &mut Item {
        if !self.items.contains_key(id) {
            self.order.push(id.into());
        }
        self.items.entry(id.into()).or_default()
    }
    pub fn started(&mut self, id: &str, phase: Option<&str>) {
        if self.mode == "off" || id.is_empty() {
            return;
        }
        self.item(id).phase = phase.map(str::to_owned);
        self.dirty = true;
    }
    pub fn delta(&mut self, id: &str, text: &str) {
        if self.mode == "off" || id.is_empty() {
            return;
        }
        let item = self.item(id);
        if !item.complete {
            item.text.push_str(text);
            self.dirty = true;
        }
    }
    pub fn message(&mut self, id: &str, text: &str, phase: Option<&str>) {
        if self.mode == "off" || id.is_empty() {
            return;
        }
        let item = self.item(id);
        item.text = text.into();
        item.phase = phase.map(str::to_owned);
        item.complete = true;
        self.dirty = true;
    }
    pub fn flush(&mut self, store: &Store, task: &Task) -> Result<()> {
        if !self.dirty || self.mode == "off" {
            return Ok(());
        }
        let mut conn = store.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: String = tx.query_row(
            "SELECT data FROM records WHERE kind='task' AND id=?1",
            [&task.id],
            |r| r.get(0),
        )?;
        let current: Task = serde_json::from_str(&raw)?;
        if current.generation != task.generation || current.state != "running" {
            return Ok(());
        }
        for progress in [true, false] {
            let selected: Vec<_> = self
                .order
                .iter()
                .filter_map(|id| self.items.get(id).map(|item| (id, item)))
                .filter(|(_, item)| {
                    if progress {
                        item.phase.as_deref() == Some("commentary")
                    } else {
                        item.phase.as_deref() == Some("final_answer")
                            || (item.phase.is_none() && item.complete)
                    }
                })
                .collect();
            if self.mode == "progress" {
                if progress {
                    for (id, item) in selected
                        .iter()
                        .filter(|(_, item)| item.complete && !item.text.trim().is_empty())
                    {
                        let key = format!("progress:{}:{}:{}", task.id, task.generation, id);
                        tx.execute("INSERT OR IGNORE INTO deliveries(id,task_id,bot_id,target,text) VALUES(?1,?1,?2,?3,?4)",params![key,task.bot_id,serde_json::to_string(&task.reply)?,item.text])?;
                    }
                }
                continue;
            }
            let text = selected
                .iter()
                .map(|(_, item)| item.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            if text.trim().is_empty() {
                continue;
            }
            let key = format!(
                "{}:{}:{}",
                task.id,
                task.generation,
                if progress { "progress" } else { "answer" }
            );
            let desired = Desired {
                key: key.clone(),
                task: task.id.clone(),
                generation: task.generation,
                bot: task.bot_id.clone(),
                reply: task.reply.clone().unwrap(),
                progress,
                text,
                sealed: false,
            };
            tx.execute("INSERT INTO records(kind,id,data) VALUES('reply_desired',?1,?2) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data",params![key,serde_json::to_string(&desired)?])?;
        }
        tx.commit()?;
        self.dirty = false;
        Ok(())
    }
}

fn operation(state: &mut Transport, text: &str, kind: OperationKind) -> Operation {
    // Legacy pending operations used only a close boolean. Infer send from the
    // absence of message ID; persist the migrated representation before network I/O.
    if let Some(op) = state.pending.as_mut() {
        state.allocated_sequence = state
            .allocated_sequence
            .max(state.sequence)
            .max(op.sequence);
        if op.kind.is_none() {
            op.kind = Some(if state.message.is_none() {
                OperationKind::Send
            } else if op.close {
                OperationKind::Close
            } else {
                OperationKind::Content
            });
        }
        return op.clone();
    }
    state.allocated_sequence = state.allocated_sequence.max(state.sequence) + 1;
    let op = Operation {
        text: text.into(),
        close: kind == OperationKind::Close,
        kind: Some(kind),
        sequence: state.allocated_sequence,
        uuid: uuid::Uuid::new_v4().to_string(),
    };
    state.pending = Some(op.clone());
    op
}
fn may_update(store: &Store, desired: &Desired, bot: &Bot) -> Result<bool> {
    let task: Task = store.get("task", &desired.task)?;
    let current: Bot = store.get("bot", &bot.id)?;
    Ok(current.enabled
        && mode(store, &current) == "card"
        && task.generation == desired.generation
        && matches!(
            task.state.as_str(),
            "running" | "waiting" | "queued" | "succeeded"
        ))
}
async fn stop_card(
    store: &Store,
    desired: &Desired,
    state: &mut Transport,
    bot: &Bot,
) -> Result<()> {
    state.closed = false;
    if state.card.is_none() {
        state.failed = true;
        return Ok(());
    }
    if state
        .pending
        .as_ref()
        .is_some_and(|op| op.kind != Some(OperationKind::Close) && !op.close)
    {
        if let Some(op) = state.pending.take() {
            state.allocated_sequence = state.allocated_sequence.max(op.sequence);
        }
    }
    let op = operation(state, "", OperationKind::Close);
    store.save("reply_transport", &desired.key, state)?;
    let result =
        crate::lark_cards::close(bot, state.card.as_deref().unwrap(), op.sequence, &op.uuid).await;
    if let Err(error) = result {
        if !crate::lark_cards::streaming_closed(&error) {
            return Err(error);
        }
    }
    state.sequence = op.sequence;
    state.pending = None;
    state.closed = true;
    store.save("reply_transport", &desired.key, state)?;
    Ok(())
}
async fn advance(store: &Store, desired: &Desired, state: &mut Transport, bot: &Bot) -> Result<()> {
    if !may_update(store, desired, bot)? {
        return stop_card(store, desired, state, bot).await;
    }
    if state.card.is_none() {
        state.card = Some(crate::lark_cards::create(bot, &desired.text).await?);
        state.summary = crate::lark_cards::summary(&desired.text);
        store.save("reply_transport", &desired.key, state)?;
    }
    if !may_update(store, desired, bot)? {
        return stop_card(store, desired, state, bot).await;
    }
    if state.message.is_none() {
        let op = operation(state, "", OperationKind::Send);
        store.save("reply_transport", &desired.key, state)?;
        state.message = Some(
            crate::lark_cards::send(
                bot,
                &desired.reply,
                state.card.as_deref().unwrap(),
                &op.uuid,
            )
            .await?,
        );
        state.pending = None;
        store.save("reply_transport", &desired.key, state)?;
    }
    // Bounded state transitions: failed content -> open -> content -> close.
    // Unknown responses leave the exact persisted operation intact for retry.
    for _ in 0..5 {
        if !may_update(store, desired, bot)? {
            return stop_card(store, desired, state, bot).await;
        }
        let kind = if state.pending.is_some() || state.sent != desired.text {
            OperationKind::Content
        } else if state.summary != crate::lark_cards::summary(&desired.text) {
            OperationKind::Summary
        } else if desired.sealed && !state.closed {
            OperationKind::Close
        } else {
            break;
        };
        let text = if kind == OperationKind::Summary {
            crate::lark_cards::summary(&desired.text)
        } else {
            desired.text.clone()
        };
        let op = operation(state, &text, kind);
        store.save("reply_transport", &desired.key, state)?;
        let result = match op.kind.unwrap() {
            OperationKind::Summary => {
                crate::lark_cards::update_summary(
                    bot,
                    state.card.as_deref().unwrap(),
                    &op.text,
                    op.sequence,
                    &op.uuid,
                )
                .await
            }
            OperationKind::Open => {
                crate::lark_cards::open(bot, state.card.as_deref().unwrap(), op.sequence, &op.uuid)
                    .await
            }
            OperationKind::Content => {
                crate::lark_cards::content(
                    bot,
                    state.card.as_deref().unwrap(),
                    &op.text,
                    op.sequence,
                    &op.uuid,
                )
                .await
            }
            OperationKind::Close => {
                crate::lark_cards::close(bot, state.card.as_deref().unwrap(), op.sequence, &op.uuid)
                    .await
            }
            OperationKind::Send | OperationKind::Layout => {
                anyhow::bail!("invalid pending card send state")
            }
        };
        if let Err(error) = result {
            let closed = crate::lark_cards::streaming_closed(&error);
            if closed
                && op.kind == Some(OperationKind::Content)
                && !state.recovery_attempted
                && may_update(store, desired, bot)?
            {
                // The server explicitly rejected this write. Retire its sequence,
                // allocate a higher sequence and a new UUID for opening the card.
                state.last_error = Some(error.to_string());
                state.recovery_attempted = true;
                state.pending = None;
                operation(state, "", OperationKind::Open);
                store.save("reply_transport", &desired.key, state)?;
                continue;
            }
            if !(closed && op.kind == Some(OperationKind::Close) && state.sent == desired.text) {
                return Err(error);
            }
        }
        state.sequence = op.sequence;
        state.pending = None;
        match op.kind.unwrap() {
            OperationKind::Content => {
                state.sent = op.text;
                state.recovery_attempted = false;
            }
            OperationKind::Summary => state.summary = op.text,
            OperationKind::Close => {
                state.closed = true;
                state.layout_pending =
                    desired.sealed && crate::lark_cards::should_fold(desired.progress, &state.sent);
            }
            _ => {}
        }
        store.save("reply_transport", &desired.key, state)?;
        if state.closed {
            break;
        }
    }
    Ok(())
}

pub async fn deliver(store: &Store) -> Result<()> {
    // Keep each task ordered, while a slow provider does not stall other tasks.
    let mut groups: BTreeMap<String, Vec<Desired>> = BTreeMap::new();
    for desired in store.list::<Desired>("reply_desired")? {
        let state: Transport = store
            .get("reply_transport", &desired.key)
            .unwrap_or_default();
        if (!state.closed || state.layout_pending) && !state.failed && state.next_at <= now() {
            groups
                .entry(desired.task.clone())
                .or_default()
                .push(desired);
        }
    }
    let mut workers = tokio::task::JoinSet::new();
    for (_, mut group) in groups {
        group.sort_by_key(|d| (d.generation, !d.progress));
        let store = store.clone();
        workers.spawn(async move {
            for desired in group {
                deliver_one(&store, desired).await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        if workers.len() >= 4 {
            workers.join_next().await.unwrap()??;
        }
    }
    while let Some(result) = workers.join_next().await {
        result??;
    }
    Ok(())
}

async fn deliver_one(store: &Store, mut desired: Desired) -> Result<()> {
    let mut state: Transport = store
        .get("reply_transport", &desired.key)
        .unwrap_or_default();
    if (state.closed && !state.layout_pending) || state.failed || state.next_at > now() {
        return Ok(());
    }
    let task: Task = store.get("task", &desired.task)?;
    let bot: Bot = store.get("bot", &desired.bot)?;
    if state.closed && state.layout_pending {
        finish_layout(store, &desired, &mut state, &bot).await?;
        return Ok(());
    }
    if store
        .get::<serde_json::Value>("command_task_suppression", &task.id)
        .is_ok()
    {
        // An explicit command owns the final notification; close any existing card
        // without creating a late card or re-publishing partial task content.
        stop_card(store, &desired, &mut state, &bot).await?;
        state.closed = true;
        store.save("reply_transport", &desired.key, &state)?;
        return Ok(());
    }

    if !bot.enabled {
        return Ok(());
    }
    let terminal = !matches!(
        task.state.as_str(),
        "queued" | "running" | "waiting" | "cancel_requested"
    );
    if task.generation != desired.generation || terminal {
        desired.sealed = true;
        if !desired.progress && task.generation == desired.generation && task.state == "succeeded" {
            desired.text = task.result.clone().unwrap_or(desired.text);
        } else {
            desired.text.push_str(&format!(
                "\n\n— {}",
                if task.generation != desired.generation {
                    "本轮已结束"
                } else {
                    match task.state.as_str() {
                        "succeeded" => "已完成",
                        "cancelled" => "已取消",
                        _ => "已中断或失败",
                    }
                }
            ));
        }
    }
    if mode(store, &bot) != "card" || desired.text.len() > 24000 {
        let _ = stop_card(store, &desired, &mut state, &bot).await;
        state.failed = true;
    } else if let Err(error) = advance(store, &desired, &mut state, &bot).await {
        state.last_error = Some(error.to_string());
        state.attempts += 1;
        state.next_at = now() + i64::from(state.attempts) * 2;
        if state.attempts >= 3 {
            let _ = stop_card(store, &desired, &mut state, &bot).await;
            state.failed = true;
        }
    } else {
        state.attempts = 0;
        state.next_at = now() + 1;
    }
    store.save("reply_transport", &desired.key, &state)?;
    if state.closed && state.layout_pending {
        finish_layout(store, &desired, &mut state, &bot).await?;
    }
    Ok(())
}

// Cosmetic finalization must never reopen streaming or trigger text fallback.
async fn finish_layout(
    store: &Store,
    desired: &Desired,
    state: &mut Transport,
    bot: &Bot,
) -> Result<()> {
    if !bot.enabled
        || store.get::<Task>("task", &desired.task)?.generation != desired.generation
        || mode(store, bot) != "card"
        || store
            .get::<Value>("command_task_suppression", &desired.task)
            .is_ok()
    {
        state.layout_pending = false;
        state.pending = None;
        store.save("reply_transport", &desired.key, state)?;
        return Ok(());
    }
    let data = crate::lark_cards::folded_card(desired.progress, &state.sent).to_string();
    let op = operation(state, &data, OperationKind::Layout);
    store.save("reply_transport", &desired.key, state)?;
    match crate::lark_cards::replace_card(
        bot,
        state.card.as_deref().unwrap(),
        &op.text,
        op.sequence,
        &op.uuid,
    )
    .await
    {
        Ok(()) => {
            state.sequence = op.sequence;
            state.pending = None;
            state.layout_pending = false;
            state.attempts = 0;
        }
        Err(error) => {
            state.last_error = Some(format!("final layout: {error}"));
            state.attempts += 1;
            state.next_at = now() + i64::from(state.attempts) * 2;
            if state.attempts >= 3 {
                state.layout_pending = false;
                state.pending = None;
            }
        }
    }
    store.save("reply_transport", &desired.key, state)?;
    Ok(())
}

/// Only a successfully closed card containing the exact final result suppresses ordinary delivery.
/// Pending/failed card operations never mark an ordinary delivery sent.
pub fn final_delivery(store: &Store, task_id: &str, text: &str) -> Result<&'static str> {
    let Ok(task) = store.get::<Task>("task", task_id) else {
        return Ok("send");
    };
    let key = format!("{}:{}:answer", task.id, task.generation);
    let Some(raw) = store
        .conn()?
        .query_row(
            "SELECT data FROM records WHERE kind='reply_desired' AND id=?1",
            [&key],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok("send");
    };
    let _: Desired = serde_json::from_str(&raw)?;
    let bot: Bot = store.get("bot", &task.bot_id)?;
    if mode(store, &bot) != "card" || text.len() > 24000 {
        return Ok("send");
    }
    let state: Transport = store.get("reply_transport", &key).unwrap_or_default();
    if state.failed {
        Ok(
            if task.state == "succeeded" && state.message.is_some() && state.sent == text {
                "sent"
            } else {
                "send"
            },
        )
    } else if state.closed {
        Ok(if task.state == "succeeded" && state.sent == text {
            "sent"
        } else {
            "send"
        })
    } else {
        Ok("wait")
    }
}

/// Preserve the full stored result, but deliver only text not acknowledged by the card API.
/// Exact prefix matching avoids dropping revised text or guessing overlapping substrings.
pub fn fallback_text(store: &Store, task_id: &str, text: &str) -> Result<String> {
    let Ok(task) = store.get::<Task>("task", task_id) else {
        return Ok(text.into());
    };
    if task.state != "succeeded" {
        return Ok(text.into());
    }
    let key = format!("{}:{}:answer", task.id, task.generation);
    let state: Transport = store.get("reply_transport", &key).unwrap_or_default();
    if state.message.is_some() && !state.sent.is_empty() {
        if let Some(remaining) = text.strip_prefix(&state.sent) {
            return Ok(remaining.to_owned());
        }
    }
    Ok(text.into())
}

#[cfg(test)]
mod mode_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn lark_defaults_to_card_but_explicit_modes_and_other_channels_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut bot = Bot {
            id: "new-bot".into(),
            channel: "lark".into(),
            account_id: "test".into(),
            credential_ref: None,
            enabled: true,
            owner: None,
        };
        assert_eq!(mode(&store, &bot), "card");
        let path = dir.path().join("reply-streaming.json");
        for (value, expected) in [
            (json!("off"), "off"),
            (json!("progress"), "progress"),
            (json!("card"), "card"),
            (json!("invalid"), "off"),
            (Value::Null, "off"),
        ] {
            std::fs::write(&path, json!({"new-bot":value}).to_string()).unwrap();
            assert_eq!(mode(&store, &bot), expected);
        }
        std::fs::write(&path, "{}").unwrap();
        bot.channel = "wechat".into();
        assert_eq!(mode(&store, &bot), "off");
        std::fs::write(&path, json!({"new-bot":"progress"}).to_string()).unwrap();
        assert_eq!(mode(&store, &bot), "progress");
        bot.channel = "local".into();
        assert_eq!(mode(&store, &bot), "off");
    }
}
