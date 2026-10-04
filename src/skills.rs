use crate::{domain::*, engine::CodexEngine, store::Store};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;
pub fn catalog(value: Value) -> Result<Vec<Value>> {
    let mut found = Vec::new();
    for entry in value["data"]
        .as_array()
        .context("invalid skills/list response")?
    {
        if entry["errors"].as_array().is_some_and(|e| !e.is_empty()) {
            bail!("Skill scan reported errors; fix invalid SKILL.md files before selecting Skills");
        }
        for skill in entry["skills"].as_array().context("missing skills")? {
            found.push(skill.clone());
        }
    }
    found.sort_by_key(|s| s["path"].as_str().unwrap_or_default().to_owned());
    found.dedup_by(|a, b| a["path"] == b["path"]);
    Ok(found)
}
pub fn resolve<'a>(skills: &'a [Value], selector: &str) -> Result<&'a Value> {
    let matches: Vec<_> = skills
        .iter()
        .filter(|s| s["path"] == selector || s["name"] == selector)
        .collect();
    if matches.len() != 1 {
        bail!("Skill '{selector}' is missing or ambiguous; use the exact path from /skills available -r");
    }
    Ok(matches[0])
}
fn selected(skills: &[Value], selectors: &[String]) -> Result<BTreeSet<String>> {
    selectors
        .iter()
        .map(|s| {
            Ok(resolve(skills, s)?["path"]
                .as_str()
                .context("missing Skill path")?
                .to_owned())
        })
        .collect()
}
pub fn overrides(skills: &[Value], selectors: &[String]) -> Result<Value> {
    let paths = selected(skills, selectors)?;
    Ok(json!(skills.iter().map(|s|json!({"path":s["path"],"enabled":paths.contains(s["path"].as_str().unwrap_or_default())})).collect::<Vec<_>>()))
}
pub fn verify(skills: &[Value], selectors: &[String]) -> Result<()> {
    let expected = selected(skills, selectors)?;
    let actual: BTreeSet<_> = skills
        .iter()
        .filter(|s| s["enabled"] == true)
        .map(|s| s["path"].as_str().unwrap_or_default().to_owned())
        .collect();
    if actual != expected {
        bail!("Codex Skill isolation verification failed; refusing to run with unexpected Skills");
    }
    Ok(())
}
pub fn toml_overrides(value: &Value) -> Result<String> {
    let entries = value
        .as_array()
        .context("invalid skill overrides")?
        .iter()
        .map(|s| {
            Ok(format!(
                "{{path={},enabled={}}}",
                serde_json::to_string(s["path"].as_str().context("invalid Skill path")?)?,
                s["enabled"].as_bool().context("invalid Skill state")?
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("[{}]", entries.join(",")))
}
pub async fn command(store: &Store, agent: &str, text: &str) -> Result<Value> {
    let words = shlex::split(text).context("invalid quoting")?;
    let action = words.first().map(String::as_str).unwrap_or("list");
    let selector = words.get(1).map(String::as_str);
    if words.len() > 2 {
        bail!(
            "usage: /skills list|available|show|enable|disable|mode [name|path|inherit|allowlist]"
        );
    }
    let mut a: Agent = store.get("agent", agent)?;
    let mut requested_mode = None;
    if action == "mode" {
        let mode = match selector {
            Some("inherit") => SkillsMode::Inherit,
            Some("allowlist") => SkillsMode::Allowlist,
            _ => bail!("usage: /skills mode inherit|allowlist"),
        };
        requested_mode = Some(mode);
    } else if !matches!(action, "list" | "available" | "show" | "enable" | "disable") {
        bail!("usage: /skills list|available|show|enable|disable|mode");
    }
    if matches!(action, "list" | "available") && selector.is_some() {
        bail!("unexpected Skill argument");
    }
    let all = CodexEngine::skills_catalog("codex", &a.config, &a.workspace, &[]).await?;
    let revision = a.revision;
    if let Some(mode) = requested_mode {
        if mode == SkillsMode::Allowlist {
            selected(&all, &a.config.skills)?;
        }
        a = store.update_agent(agent, |a| {
            anyhow::ensure!(
                a.revision == revision,
                "Agent configuration changed; retry the command"
            );
            a.config.skills_mode = mode;
            Ok(())
        })?;
    }
    if matches!(action, "enable" | "disable") {
        let s = selector.context("missing Skill name/path")?;
        let stored = action == "disable"
            && a.config.skills_mode == SkillsMode::Allowlist
            && a.config.skills.iter().any(|v| v == s);
        let path = if stored {
            s.to_owned()
        } else {
            resolve(&all, s)?["path"]
                .as_str()
                .context("missing path")?
                .to_owned()
        };
        let mut enabled: BTreeSet<String> = if a.config.skills_mode == SkillsMode::Inherit {
            all.iter()
                .filter(|s| s["enabled"] == true)
                .map(|s| s["path"].as_str().unwrap().to_owned())
                .collect()
        } else {
            a.config
                .skills
                .iter()
                .map(|value| {
                    resolve(&all, value)
                        .ok()
                        .and_then(|v| v["path"].as_str())
                        .unwrap_or(value)
                        .to_owned()
                })
                .collect()
        };
        if action == "enable" {
            enabled.insert(path);
        } else {
            enabled.remove(&path);
        }
        a = store.update_agent(agent, |a| {
            anyhow::ensure!(
                a.revision == revision,
                "Agent configuration changed; retry the command"
            );
            a.config.skills_mode = SkillsMode::Allowlist;
            a.config.skills = enabled.into_iter().collect();
            Ok(())
        })?;
    }
    let wanted = if a.config.skills_mode == SkillsMode::Allowlist {
        Some(
            a.config
                .skills
                .iter()
                .map(|value| {
                    resolve(&all, value)
                        .ok()
                        .and_then(|v| v["path"].as_str())
                        .unwrap_or(value)
                        .to_owned()
                })
                .collect::<BTreeSet<_>>(),
        )
    } else {
        None
    };
    let mut rows = Vec::new();
    if let Some(wanted) = &wanted {
        for path in wanted {
            if !all.iter().any(|s| s["path"] == *path) {
                rows.push(json!({"name":path,"path":path,"enabled":false,"missing":true,"description":"文件缺失或名称不唯一；可使用 /skills disable 移除"}));
            }
        }
    }
    for mut skill in all.clone() {
        let enabled = wanted
            .as_ref()
            .map(|w| w.contains(skill["path"].as_str().unwrap_or_default()))
            .unwrap_or(skill["enabled"] == true);
        skill["enabled"] = json!(enabled);
        if action != "list" || enabled {
            rows.push(skill);
        }
    }
    if action == "show" {
        let skill = resolve(&rows, selector.context("missing Skill name/path")?)?;
        return Ok(json!({"agent_id":agent,"mode":a.config.skills_mode,"skills":[skill]}));
    }
    Ok(
        json!({"agent_id":agent,"mode":a.config.skills_mode,"skills":rows,"revision":a.revision,"notice":"Changes apply at the next engine boundary; old conversation history is retained."}),
    )
}
