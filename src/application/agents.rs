use super::*;
use serde_json::{json, Value};
use std::path::Path;
pub fn changes(before: &Agent, after: &Agent) -> Value {
    let mut fields = Vec::new();
    for (name, old, new) in [
        (
            "system_prompt",
            json!(before.config.system_prompt),
            json!(after.config.system_prompt),
        ),
        (
            "model",
            json!(before.config.model),
            json!(after.config.model),
        ),
        (
            "effort",
            json!(before.config.effort),
            json!(after.config.effort),
        ),
        (
            "profile",
            json!(before.config.profile),
            json!(after.config.profile),
        ),
    ] {
        if old != new {
            fields.push(json!({"field":name,"before":old,"after":new}));
        }
    }
    json!(fields)
}

#[derive(Default)]
pub struct ConfigPatch {
    pub system: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub profile: Option<String>,
}
pub fn configure(store: &Store, scope: &Scope, id: &str, patch: ConfigPatch) -> Result<Agent> {
    scope.agent(store, id)?;
    if let Some(profile) = &patch.profile {
        crate::engine::validate_profile(profile)?;
    }
    store.update_agent(id, |a| {
        if let Some(v) = patch.system {
            a.config.system_prompt = v;
        }
        if let Some(v) = patch.model {
            a.config.model = (v != "default").then_some(v);
        }
        if let Some(v) = patch.effort {
            a.config.effort = (v != "default").then_some(v);
        }
        if let Some(v) = patch.profile {
            a.config.profile = (v != "default").then_some(v);
        }
        Ok(())
    })
}
pub fn create(
    store: &Store,
    scope: &Scope,
    id: &str,
    bot: &str,
    template: &str,
    workspace: &Path,
) -> Result<Agent> {
    scope.bot(bot)?;
    store.create_agent(id, bot, template, workspace)
}
pub fn delete(store: &Store, scope: &Scope, id: &str, force: bool) -> Result<Agent> {
    scope.agent(store, id)?;
    store.delete_agent(id, force)
}
pub fn enable(store: &Store, scope: &Scope, id: &str) -> Result<Agent> {
    scope.agent(store, id)?;
    store.update_agent(id, |a| {
        a.enabled = true;
        Ok(())
    })
}
pub fn list(store: &Store, scope: &Scope, global: bool) -> Result<Vec<Agent>> {
    Ok(store
        .list::<Agent>("agent")?
        .into_iter()
        .filter(|a| global || scope.bot(&a.bot_id).is_ok())
        .collect())
}
