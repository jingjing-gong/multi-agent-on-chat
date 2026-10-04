use super::*;
use rusqlite::params;
use serde_json::{json, Value};
pub fn details(store: &Store, scope: &Scope, id: &str) -> Result<Value> {
    let task: Task = store.get("task", id)?;
    scope.bot(&task.bot_id)?;
    let c = store.conn()?;
    let mut q = c.prepare(
        "SELECT kind,data,created FROM events WHERE task_id=?1 ORDER BY seq DESC LIMIT 20",
    )?;
    let events = q.query_map([id], |r| Ok(json!({"kind":r.get::<_,String>(0)?,"data":r.get::<_,String>(1)?,"created":r.get::<_,i64>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let mut q = c.prepare("SELECT state,attempts FROM deliveries WHERE task_id=?1 OR id IN (SELECT d.ref_id FROM media_refs d JOIN media_refs a ON a.asset_id=d.asset_id WHERE a.kind='task_artifact' AND a.ref_id=?1 AND d.kind='delivery')")?;
    let deliveries = q
        .query_map(params![id], |r| {
            Ok(json!({"state":r.get::<_,String>(0)?,"attempts":r.get::<_,i64>(1)?}))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = serde_json::to_value(task)?;
    result["attachments"] = json!(crate::media::task_assets(store, id)?
        .iter()
        .map(crate::media::public)
        .collect::<Vec<_>>());
    let mut q=c.prepare("SELECT a.data FROM media_assets a JOIN media_refs r ON a.id=r.asset_id WHERE r.kind='task_artifact' AND r.ref_id=?1")?;
    let artifacts = q
        .query_map([id], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    result["artifacts"] = json!(artifacts
        .iter()
        .map(|s| serde_json::from_str::<crate::media::Asset>(s).map(|a| crate::media::public(&a)))
        .collect::<std::result::Result<Vec<_>, _>>()?);
    result["events"] = json!(events);
    result["deliveries"] = json!(deliveries);
    Ok(result)
}
