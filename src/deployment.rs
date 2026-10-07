use crate::{domain::*, store::Store};
use anyhow::Result;
use serde_json::{json, Value};
pub fn draining(store: &Store) -> bool {
    store.get::<bool>("control", "draining").unwrap_or(false)
}
pub fn readiness(store: &Store) -> Result<Value> {
    let mut blockers = Vec::new();
    for task in store.list::<Task>("task")? {
        if [
            "queued",
            "running",
            "waiting_dependency",
            "cancel_requested",
            "interrupted",
            "needs_attention",
        ]
        .contains(&task.state.as_str())
        {
            blockers.push(json!({"kind":"task","id":task.id,"state":task.state}));
        }
    }
    let c = store.conn()?;
    for (table, terminal) in [
        ("deliveries", "'sent','failed'"),
        (
            "media_inbox",
            "'ready','failed','cancelled','delivered','ignored','submitted'",
        ),
        ("media_extraction_jobs", "'succeeded','failed','cancelled'"),
    ] {
        let exists: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |r| r.get(0),
        )?;
        if !exists {
            continue;
        }
        let mut stmt = c.prepare(&format!(
            "SELECT id,state FROM {table} WHERE state NOT IN ({terminal})"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, state) = row?;
            blockers.push(json!({"kind":table,"id":id,"state":state}));
        }
    }
    for command in store.list::<Value>("command")? {
        if command["state"]
            .as_str()
            .is_some_and(|s| ["queued", "running", "pending"].contains(&s))
        {
            blockers.push(json!({"kind":"command","id":command["id"],"state":command["state"]}));
        }
    }
    for desired in store.list::<Value>("reply_desired")? {
        if let Some(key) = desired["key"].as_str() {
            let state = store
                .get::<Value>("reply_transport", key)
                .unwrap_or(json!({}));
            if state["failed"] != true
                && (state["closed"] != true || state["layout_pending"] == true)
            {
                blockers.push(json!({"kind":"reply_desired","key":key}));
            }
        }
    }
    for stream in store.list::<Value>("reply_transport")? {
        if stream["failed"] == true {
            continue;
        }
        if !stream["pending"].is_null()
            || stream["layout_pending"] == true
            || (stream["card_id"].is_string() && stream["closed"] != true)
        {
            blockers.push(json!({"kind":"reply_transport","card_id":stream["card_id"]}));
        }
    }
    let failed_deliveries: i64 = c.query_row(
        "SELECT count(*) FROM deliveries WHERE state='failed'",
        [],
        |r| r.get(0),
    )?;
    Ok(
        json!({"draining":draining(store),"ready":draining(store)&&blockers.is_empty(),"blockers":blockers,"terminal_failed_deliveries":failed_deliveries}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drain_readiness_counts_pending_deliveries_and_requires_drain() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        assert_eq!(readiness(&s).unwrap()["ready"], false);
        s.save("control", "draining", &true).unwrap();
        assert_eq!(readiness(&s).unwrap()["ready"], true);
        s.conn().unwrap().execute("INSERT INTO deliveries(id,task_id,bot_id,target,text) VALUES('d','t','b','{}','pending')",[]).unwrap();
        assert_eq!(readiness(&s).unwrap()["ready"], false);
        s.conn()
            .unwrap()
            .execute("UPDATE deliveries SET state='sent'", [])
            .unwrap();
        assert_eq!(readiness(&s).unwrap()["ready"], true);
    }
}
