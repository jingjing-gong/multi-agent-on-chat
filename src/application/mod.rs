//! Shared application operations. Adapters supply identity, never business rules.
pub mod agents;
pub mod command_reply;
pub mod commands;
pub mod cron;
pub mod errors;
pub mod execution;
pub mod output;
pub mod tasks;
use crate::{domain::*, store::Store};
use anyhow::Result;

#[derive(Clone, Debug)]
pub enum Scope {
    Local,
    Bot(String),
}
impl Scope {
    pub fn bot(&self, id: &str) -> Result<()> {
        if matches!(self, Self::Bot(own) if own != id) {
            return Err(errors::Error::new(errors::Code::Forbidden, "目标属于其他 Bot").into());
        }
        Ok(())
    }
    pub fn agent(&self, store: &Store, id: &str) -> Result<Agent> {
        let agent: Agent = store.get("agent", id)?;
        self.bot(&agent.bot_id)?;
        Ok(agent)
    }
}

pub mod media;

pub mod stream;
