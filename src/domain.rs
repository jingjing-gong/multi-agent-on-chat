use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub skills_mode: SkillsMode,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillsMode {
    #[default]
    Inherit,
    Allowlist,
}
fn one() -> u32 {
    1
}
impl Template {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.id)?;
        if self.version == 0 {
            bail!("template version must be positive");
        }
        if self.system_prompt.len() > 100_000 {
            bail!("system prompt too large");
        }
        for s in &self.skills {
            if s.trim().is_empty() || s.len() > 4096 {
                bail!("invalid Skill selector");
            }
        }
        Ok(())
    }
}
pub fn identifier(s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > 100
        || !s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_- .".contains(&c) && c != b' ')
    {
        bail!("invalid identifier: use letters, digits, dot, underscore or hyphen");
    }
    if s == "." || s == ".." {
        bail!("invalid identifier");
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bot {
    pub id: String,
    pub channel: String,
    pub account_id: String,
    pub credential_ref: Option<PathBuf>,
    pub enabled: bool,
    pub owner: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub bot_id: String,
    pub workspace: PathBuf,
    pub revision: u32,
    pub config: Template,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub kind: String,
    pub scope: String,
    pub config: Template,
    pub config_revision: u32,
    pub thread_id: Option<String>,
    pub last_used: i64,
    pub state: String,
    #[serde(default)]
    pub workspace: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub bot_id: String,
    pub prompt: String,
    pub state: String,
    pub result: Option<String>,
    pub parent_id: Option<String>,
    pub generation: u32,
    pub created_at: i64,
    pub reply: Option<ReplyTarget>,
    #[serde(default = "default_task_kind")]
    pub kind: String,
}
fn default_task_kind() -> String {
    "agent".into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplyTarget {
    pub chat_id: String,
    pub context_token: Option<String>,
    pub thread_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub agent_id: String,
    pub bot_id: String,
    pub prompt: String,
    pub schedule: String,
    pub timezone: String,
    pub config: Template,
    pub config_revision: u32,
    pub next_fire: Option<i64>,
    pub state: String,
    pub reply: Option<ReplyTarget>,
    pub revision: u32,
    #[serde(default)]
    pub workspace: PathBuf,
}
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
