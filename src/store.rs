use crate::domain::*;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}
impl Store {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(root.as_ref())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.as_ref(), std::fs::Permissions::from_mode(0o700))?;
        }
        let s = Self {
            root: root.as_ref().to_path_buf(),
        };
        let c = s.conn()?;
        let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 3 {
            bail!("database schema newer than this executable");
        }
        c.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;
  CREATE TABLE IF NOT EXISTS records(kind TEXT NOT NULL,id TEXT NOT NULL,data TEXT NOT NULL,PRIMARY KEY(kind,id));
  CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY AUTOINCREMENT,task_id TEXT NOT NULL,kind TEXT NOT NULL,data TEXT NOT NULL,created INTEGER NOT NULL);
  CREATE TABLE IF NOT EXISTS inputs(id TEXT PRIMARY KEY,task_id TEXT NOT NULL,text TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending');
  CREATE TABLE IF NOT EXISTS occurrences(job_id TEXT NOT NULL,scheduled INTEGER NOT NULL,task_id TEXT,status TEXT NOT NULL,PRIMARY KEY(job_id,scheduled));
  CREATE TABLE IF NOT EXISTS deliveries(id TEXT PRIMARY KEY,task_id TEXT UNIQUE NOT NULL,bot_id TEXT NOT NULL,target TEXT NOT NULL,text TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,next_at INTEGER NOT NULL DEFAULT 0);
  CREATE TABLE IF NOT EXISTS receipts(bot_id TEXT NOT NULL,event_id TEXT NOT NULL,PRIMARY KEY(bot_id,event_id));
  CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
")?;
        crate::media::migrate(&c)?;
        crate::media::extractions::migrate(&c)?;
        c.pragma_update(None, "user_version", 3)?;
        c.execute_batch("COMMIT")?;
        Ok(s)
    }
    pub fn conn(&self) -> Result<Connection> {
        let c = Connection::open(self.root.join("runtime.sqlite3"))?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(c)
    }
    fn put<T: Serialize>(c: &Connection, kind: &str, id: &str, v: &T) -> Result<()> {
        c.execute("INSERT INTO records VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET data=excluded.data",params![kind,id,serde_json::to_string(v)?])?;
        Ok(())
    }
    fn get_c<T: DeserializeOwned>(c: &Connection, kind: &str, id: &str) -> Result<T> {
        let s: String = c
            .query_row(
                "SELECT data FROM records WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .with_context(|| format!("{kind} {id} not found"))?;
        Ok(serde_json::from_str(&s)?)
    }
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<T> {
        Self::get_c(&self.conn()?, kind, id)
    }
    pub fn save<T: Serialize>(&self, kind: &str, id: &str, v: &T) -> Result<()> {
        Self::put(&self.conn()?, kind, id, v)
    }
    fn list_c<T: DeserializeOwned>(c: &Connection, kind: &str) -> Result<Vec<T>> {
        let mut q = c.prepare("SELECT data FROM records WHERE kind=?1 ORDER BY rowid")?;
        let rows = q.query_map([kind], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        Self::list_c(&self.conn()?, kind)
    }
    pub fn register_template(&self, t: &Template) -> Result<()> {
        t.validate()?;
        let key = format!("{}@{}", t.id, t.version);
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Ok(old) = Self::get_c::<Template>(&tx, "template", &key) {
            if serde_json::to_value(old)? != serde_json::to_value(t)? {
                bail!("template version already exists with different content");
            }
        }
        Self::put(&tx, "template", &key, t)?;
        tx.commit()?;
        Ok(())
    }
    pub fn delete_template(&self, key: &str) -> Result<Template> {
        let (name, version) = key
            .rsplit_once('@')
            .context("usage: /template delete <name@version>")?;
        identifier(name)?;
        let version: u32 = version.parse().context("invalid template version")?;
        if version == 0 {
            bail!("template version must be positive");
        }
        if key == "default@1" {
            bail!("default@1 is required for new Bots and cannot be deleted");
        }
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let template: Template = Self::get_c(&tx, "template", key)?;
        tx.execute("DELETE FROM records WHERE kind='template' AND id=?1", [key])?;
        tx.commit()?;
        Ok(template)
    }
    pub fn template_from_agent(
        &self,
        agent_id: &str,
        id: &str,
        version: u32,
        description: Option<&str>,
    ) -> Result<Template> {
        identifier(id)?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let agent: Agent = Self::get_c(&tx, "agent", agent_id)?;
        let mut template = agent.config.clone();
        template.id = id.into();
        template.version = version;
        if let Some(text) = description {
            template.description = text.into();
        }
        template.validate()?;
        let key = format!("{id}@{version}");
        Self::put(&tx, "template", &key, &template)?;
        tx.commit()?;
        Ok(template)
    }
    pub fn set_bot_enabled(&self, id: &str, enabled: bool) -> Result<Bot> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut bot: Bot = Self::get_c(&tx, "bot", id)?;
        if enabled && bot.owner.is_none() && bot.channel != "lark" {
            bail!("set an owner before enabling");
        }
        bot.enabled = enabled && bot.owner.is_some();
        Self::put(&tx, "owner_binding_paused", id, &!enabled)?;
        Self::put(&tx, "bot", id, &bot)?;
        tx.commit()?;
        Ok(bot)
    }
    fn ensure_default_template(c: &Connection) -> Result<Template> {
        if let Ok(t) = Self::get_c::<Template>(c, "template", "default@1") {
            return Ok(t);
        }
        let template = Template {
            id: "default".into(),
            version: 1,
            description: "Use the local Codex default configuration".into(),
            system_prompt: String::new(),
            profile: None,
            model: None,
            effort: None,
            skills: vec![],
            skills_mode: Default::default(),
        };
        Self::put(c, "template", "default@1", &template)?;
        Ok(template)
    }
    fn ensure_bot_agent(
        &self,
        c: &Connection,
        bot: &Bot,
        template: &Template,
    ) -> Result<Option<Agent>> {
        if Self::list_c::<Agent>(c, "agent")?
            .iter()
            .any(|a| a.bot_id == bot.id)
        {
            return Ok(None);
        }
        identifier(&bot.id)?;
        let workspace = self.root.join("workspaces").join(&bot.id);
        std::fs::create_dir_all(&workspace)?;
        let preferred = format!("{}-main", bot.id);
        let id = if preferred.len() <= 100 && Self::get_c::<Agent>(c, "agent", &preferred).is_err()
        {
            preferred
        } else {
            new_id("agent")
        };
        let agent = Agent {
            id,
            bot_id: bot.id.clone(),
            workspace: workspace.canonicalize()?,
            revision: 1,
            config: template.clone(),
            enabled: true,
        };
        Self::put(c, "agent", &agent.id, &agent)?;
        Ok(Some(agent))
    }
    /// Idempotent setup; preserve all existing Agents, including disabled ones.
    pub fn initialize_defaults(&self) -> Result<Vec<Agent>> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let template = Self::ensure_default_template(&tx)?;
        let mut created = vec![];
        for bot in Self::list_c::<Bot>(&tx, "bot")? {
            if let Some(a) = self.ensure_bot_agent(&tx, &bot, &template)? {
                created.push(a);
            }
        }
        tx.commit()?;
        Ok(created)
    }
    pub fn add_bot_with_defaults(&self, bot: &Bot) -> Result<Agent> {
        identifier(&bot.id)?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if Self::list_c::<Bot>(&tx, "bot")?
            .iter()
            .any(|b| b.id == bot.id || b.channel == bot.channel && b.account_id == bot.account_id)
        {
            bail!("bot or channel account already registered");
        }
        let template = Self::ensure_default_template(&tx)?;
        Self::put(&tx, "bot", &bot.id, bot)?;
        let agent = self
            .ensure_bot_agent(&tx, bot, &template)?
            .context("unexpected existing agent for new bot")?;
        tx.commit()?;
        Ok(agent)
    }
    pub fn add_bot(&self, b: &Bot) -> Result<()> {
        identifier(&b.id)?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if Self::list_c::<Bot>(&tx, "bot")?
            .iter()
            .any(|x| x.id == b.id || x.channel == b.channel && x.account_id == b.account_id)
        {
            bail!("bot or channel account already registered");
        }
        Self::put(&tx, "bot", &b.id, b)?;
        tx.commit()?;
        Ok(())
    }
    pub fn awaiting_owner(&self, bot: &Bot) -> Result<bool> {
        Ok(bot.channel == "lark"
            && bot.owner.is_none()
            && !self
                .get::<bool>("owner_binding_paused", &bot.id)
                .unwrap_or(false))
    }
    /// Bind exactly one owner and enqueue its acknowledgement in one transaction.
    pub fn bind_owner_from_message(
        &self,
        id: &str,
        message: &crate::channels::Incoming,
    ) -> Result<bool> {
        if !message.is_private || !message.sender.starts_with("ou_") {
            return Ok(false);
        }
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut bot: Bot = Self::get_c(&tx, "bot", id)?;
        if bot.channel != "lark"
            || bot.owner.is_some()
            || Self::get_c::<bool>(&tx, "owner_binding_paused", id).unwrap_or(false)
        {
            return Ok(false);
        }
        if tx.execute(
            "INSERT OR IGNORE INTO receipts(bot_id,event_id) VALUES(?1,?2)",
            params![id, message.event_id],
        )? == 0
        {
            return Ok(false);
        }
        bot.owner = Some(message.sender.clone());
        bot.enabled = true;
        Self::put(&tx, "bot", id, &bot)?;
        Self::put(
            &tx,
            "onboarding_notification",
            id,
            &json!({"status":"queued","recipient":message.sender,"reason":"owner_bound"}),
        )?;
        tx.execute("INSERT INTO deliveries(id,task_id,bot_id,target,text) VALUES(?1,?2,?3,?4,?5)",params![new_id("d"),format!("owner-binding:{id}"),id,serde_json::to_string(&message.reply)?,"owner 设置成功，你已成为此 Bot 的使用者。这条消息仅用于绑定；请再发一条消息开始对话。"])?;
        tx.commit()?;
        Ok(true)
    }
    pub fn create_agent(
        &self,
        id: &str,
        bot: &str,
        template: &str,
        workspace: &Path,
    ) -> Result<Agent> {
        identifier(id)?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let b: Bot = Self::get_c(&tx, "bot", bot)?;
        if !b.enabled {
            bail!("bot disabled");
        }
        if Self::get_c::<Agent>(&tx, "agent", id).is_ok() {
            bail!("agent exists");
        }
        if tx
            .query_row(
                "SELECT 1 FROM records WHERE kind='archived_template' AND id=?1",
                [template.split('@').next().unwrap_or(template)],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            bail!("template archived");
        }
        let config: Template = Self::get_c(&tx, "template", template)?;
        let workspace = workspace.canonicalize().context("workspace must exist")?;
        if !workspace.is_dir() {
            bail!("workspace is not a directory");
        }
        let a = Agent {
            id: id.into(),
            bot_id: bot.into(),
            workspace,
            revision: 1,
            config,
            enabled: true,
        };
        Self::put(&tx, "agent", id, &a)?;
        tx.commit()?;
        Ok(a)
    }
    pub fn delete_agent(&self, id: &str, force: bool) -> Result<Agent> {
        let active: Vec<_> = self
            .list::<Task>("task")?
            .into_iter()
            .filter(|t| {
                t.agent_id == id
                    && matches!(
                        t.state.as_str(),
                        "queued" | "running" | "waiting_dependency" | "cancel_requested"
                    )
            })
            .collect();
        let jobs = self.list::<Job>("job")?;
        if !force
            && (!active.is_empty()
                || jobs
                    .iter()
                    .any(|j| j.agent_id == id && j.state == "enabled"))
        {
            bail!("active tasks or jobs; use --force");
        }
        let a = self.update_agent(id, |a| {
            a.enabled = false;
            Ok(())
        })?;
        for t in active {
            if t.state != "cancel_requested" {
                self.cancel(&t.id)?;
            }
        }
        for j in jobs {
            if j.agent_id == id && j.state == "enabled" {
                self.job_action(&j.id, "pause")?;
            }
        }
        Ok(a)
    }
    pub fn update_agent(
        &self,
        id: &str,
        change: impl FnOnce(&mut Agent) -> Result<()>,
    ) -> Result<Agent> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut a: Agent = Self::get_c(&tx, "agent", id)?;
        change(&mut a)?;
        a.config.validate()?;
        a.revision += 1;
        Self::put(&tx, "agent", id, &a)?;
        tx.commit()?;
        Ok(a)
    }
    fn session_c(c: &Connection, a: &Agent, kind: &str, scope: &str, new: bool) -> Result<Session> {
        if !new {
            if let Some(s) = Self::list_c::<Session>(c, "session")?
                .into_iter()
                .find(|s| {
                    s.agent_id == a.id && s.kind == kind && s.scope == scope && s.state == "open"
                })
            {
                return Ok(s);
            }
        }
        let s = Session {
            id: new_id("s"),
            agent_id: a.id.clone(),
            kind: kind.into(),
            scope: scope.into(),
            config: a.config.clone(),
            config_revision: a.revision,
            thread_id: None,
            last_used: now(),
            state: "open".into(),
            workspace: a.workspace.clone(),
        };
        Self::put(c, "session", &s.id, &s)?;
        Ok(s)
    }
    fn task_c(
        c: &Connection,
        a: &Agent,
        s: &Session,
        prompt: &str,
        parent: Option<String>,
        reply: Option<ReplyTarget>,
    ) -> Result<Task> {
        let t = Task {
            id: new_id("t"),
            session_id: s.id.clone(),
            agent_id: a.id.clone(),
            bot_id: a.bot_id.clone(),
            prompt: prompt.into(),
            state: "queued".into(),
            result: None,
            parent_id: parent,
            generation: 0,
            created_at: now(),
            reply,
            kind: "agent".into(),
        };
        Self::put(c, "task", &t.id, &t)?;
        Ok(t)
    }
    pub fn submit(
        &self,
        agent: &str,
        scope: &str,
        prompt: &str,
        reply: Option<ReplyTarget>,
        event: Option<&str>,
    ) -> Result<Task> {
        if prompt.trim().is_empty() {
            bail!("empty input");
        }
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let a: Agent = Self::get_c(&tx, "agent", agent)?;
        Self::enabled(&tx, &a)?;
        if let Some(e) = event {
            if tx.execute(
                "INSERT OR IGNORE INTO receipts VALUES(?1,?2)",
                params![a.bot_id, e],
            )? == 0
            {
                bail!("duplicate inbound event");
            }
        }
        let mut s = Self::session_c(&tx, &a, "user", scope, false)?;
        let active = Self::list_c::<Task>(&tx, "task")?.into_iter().find(|t| {
            t.session_id == s.id
                && matches!(
                    t.state.as_str(),
                    "queued" | "running" | "waiting_dependency"
                )
        });
        let t = if let Some(mut t) = active {
            if t.state == "waiting_dependency" {
                t.state = "queued".into();
                Self::put(&tx, "task", &t.id, &t)?;
            }
            tx.execute(
                "INSERT INTO inputs(id,task_id,text) VALUES(?1,?2,?3)",
                params![new_id("i"), t.id, prompt],
            )?;
            t
        } else {
            Self::task_c(&tx, &a, &s, prompt, None, reply)?
        };
        s.last_used = now();
        Self::put(&tx, "session", &s.id, &s)?;
        tx.commit()?;
        Ok(t)
    }
    fn enabled(c: &Connection, a: &Agent) -> Result<()> {
        let b: Bot = Self::get_c(c, "bot", &a.bot_id)?;
        if !a.enabled || !b.enabled {
            bail!("agent or bot disabled");
        }
        Ok(())
    }
    /// Queue immutable media input as a separate turn; never downgrade attachments to text steering.
    pub fn submit_media(
        &self,
        agent: &str,
        scope: &str,
        prompt: &str,
        reply: Option<ReplyTarget>,
        assets: &[String],
        inbox: Option<(&str, i64)>,
    ) -> Result<Task> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let a: Agent = Self::get_c(&tx, "agent", agent)?;
        Self::enabled(&tx, &a)?;
        if let Some((id, generation)) = inbox {
            let valid:i64=tx.query_row("SELECT count(*) FROM media_inbox WHERE id=?1 AND state='preparing' AND generation=?2",params![id,generation],|r|r.get(0))?;
            anyhow::ensure!(valid == 1, "media input lease lost");
        }
        for id in assets {
            let raw: String =
                tx.query_row("SELECT data FROM media_assets WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
            let asset: crate::media::Asset = serde_json::from_str(&raw)?;
            crate::media::check(&asset, &a.bot_id, Some(scope))?;
            anyhow::ensure!(
                asset.purpose != "auth_qr",
                "authentication QR cannot be model input"
            );
        }
        let session = Self::session_c(&tx, &a, "user", scope, false)?;
        let text = if prompt.trim().is_empty() {
            "请查看随消息发送的附件。"
        } else {
            prompt
        };
        let t = Self::task_c(&tx, &a, &session, text, None, reply)?;
        for id in assets {
            tx.execute(
                "INSERT OR IGNORE INTO media_refs VALUES(?1,'task_input',?2,?3)",
                params![id, t.id, now()],
            )?;
        }
        if let Some((id, generation)) = inbox {
            tx.execute("UPDATE media_inbox SET state='submitted',data=json_set(data,'$.task_id',?1),lease_until=0 WHERE id=?2 AND generation=?3",params![t.id,id,generation])?;
        }
        tx.commit()?;
        Ok(t)
    }
    pub fn compact(&self, agent: &str, scope: &str) -> Result<Task> {
        self.compact_with_reply(agent, scope, None)
    }
    pub fn compact_with_reply(
        &self,
        agent: &str,
        scope: &str,
        reply: Option<ReplyTarget>,
    ) -> Result<Task> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let a: Agent = Self::get_c(&tx, "agent", agent)?;
        Self::enabled(&tx, &a)?;
        let session = Self::session_c(&tx, &a, "user", scope, false)?;
        if session.thread_id.is_none() {
            bail!("session has no engine context to compact");
        }
        if Self::list_c::<Task>(&tx, "task")?.iter().any(|t| {
            t.session_id == session.id
                && matches!(
                    t.state.as_str(),
                    "queued" | "running" | "waiting_dependency" | "cancel_requested"
                )
        }) {
            bail!("session is busy");
        }
        let mut task = Self::task_c(
            &tx,
            &a,
            &session,
            "Compact current conversation context",
            None,
            reply,
        )?;
        task.kind = "compact".into();
        Self::put(&tx, "task", &task.id, &task)?;
        tx.commit()?;
        Ok(task)
    }
    pub fn submit_shell(
        &self,
        agent: &str,
        scope: &str,
        command: &str,
        reply: Option<ReplyTarget>,
    ) -> Result<Task> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let a: Agent = Self::get_c(&tx, "agent", agent)?;
        Self::enabled(&tx, &a)?;
        let s = Self::session_c(&tx, &a, "shell", scope, true)?;
        let mut t = Self::task_c(&tx, &a, &s, command, None, reply)?;
        t.kind = "shell".into();
        Self::put(&tx, "task", &t.id, &t)?;
        tx.commit()?;
        Ok(t)
    }
    pub fn events(&self, task: Option<&str>, after: i64) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT seq,task_id,kind,data,created FROM events WHERE seq>?1 AND (?2 IS NULL OR task_id=?2) ORDER BY seq LIMIT 500")?;
        let rows=q.query_map(params![after,task],|r|Ok(json!({"seq":r.get::<_,i64>(0)?,"task_id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"data":r.get::<_,String>(3)?,"created_at":r.get::<_,i64>(4)?})))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn deliveries(&self) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT id,task_id,bot_id,state,attempts,next_at FROM deliveries ORDER BY rowid DESC LIMIT 100")?;
        let rows=q.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"task_id":r.get::<_,String>(1)?,"bot_id":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?,"attempts":r.get::<_,i64>(4)?,"next_attempt_at":r.get::<_,i64>(5)?})))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn end_turn(&self, t: &Task, state: &str, result: &str) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut current: Task = Self::get_c(&tx, "task", &t.id)?;
        if current.generation != t.generation
            || !matches!(current.state.as_str(), "running" | "cancel_requested")
        {
            return Ok(());
        }
        if current.state == "cancel_requested" {
            Self::finish_c(&tx, &t.id, t.generation, "cancelled", result)?;
            tx.commit()?;
            return Ok(());
        }
        let pending: i64 = tx.query_row(
            "SELECT count(*) FROM inputs WHERE task_id=?1 AND state='pending'",
            [&t.id],
            |r| r.get(0),
        )?;
        let child = Self::list_c::<Task>(&tx, "task")?.iter().any(|x| {
            x.parent_id.as_deref() == Some(&t.id)
                && matches!(
                    x.state.as_str(),
                    "queued" | "running" | "waiting_dependency"
                )
        });
        if state == "succeeded" && (child || pending > 0) {
            current.result = Some(result.into());
            current.state = if child && pending == 0 {
                "waiting_dependency"
            } else {
                "queued"
            }
            .into();
            Self::put(&tx, "task", &t.id, &current)?;
            tx.commit()?;
        } else {
            Self::finish_c(&tx, &t.id, t.generation, state, result)?;
            tx.commit()?;
        }
        Ok(())
    }
    pub fn query(
        &self,
        source_task: &str,
        target: &str,
        prompt: &str,
        collaboration: Option<&str>,
        key: &str,
    ) -> Result<Task> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let task = Self::query_c(&tx, source_task, target, prompt, collaboration, key)?;
        tx.commit()?;
        Ok(task)
    }
    pub fn ask(
        &self,
        agent: &str,
        scope: &str,
        target: &str,
        prompt: &str,
        reply: Option<ReplyTarget>,
    ) -> Result<Task> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let a: Agent = Self::get_c(&tx, "agent", agent)?;
        Self::enabled(&tx, &a)?;
        let s = Self::session_c(&tx, &a, "user", scope, false)?;
        let mut source = Self::task_c(
            &tx,
            &a,
            &s,
            "Summarize the requested agent result for the user.",
            None,
            reply,
        )?;
        source.state = "waiting_dependency".into();
        Self::put(&tx, "task", &source.id, &source)?;
        let child = Self::query_c(&tx, &source.id, target, prompt, None, &new_id("q"))?;
        tx.commit()?;
        Ok(child)
    }
    fn query_c(
        tx: &Connection,
        source_task: &str,
        target: &str,
        prompt: &str,
        collaboration: Option<&str>,
        key: &str,
    ) -> Result<Task> {
        let source: Task = Self::get_c(tx, "task", source_task)?;
        let source_agent: Agent = Self::get_c(tx, "agent", &source.agent_id)?;
        Self::enabled(tx, &source_agent)?;
        if !matches!(
            source.state.as_str(),
            "running" | "waiting_dependency" | "queued"
        ) {
            bail!("source task is not active");
        }
        let key = format!("{source_task}:{key}");
        if let Ok(id) = Self::get_c::<String>(tx, "query_key", &key) {
            return Self::get_c(tx, "task", &id);
        }
        let a: Agent = Self::get_c(tx, "agent", target)?;
        Self::enabled(tx, &a)?;
        let scope = format!("{}:{}", source.agent_id, source.session_id);
        let mut depth = 0;
        let mut ancestor = source.clone();
        while let Some(p) = ancestor.parent_id {
            depth += 1;
            if depth >= 4 {
                bail!("maximum collaboration depth reached");
            }
            ancestor = Self::get_c(tx, "task", &p)?;
        }
        if Self::list_c::<Task>(tx, "task")?
            .iter()
            .filter(|t| t.parent_id.as_deref() == Some(source_task))
            .count()
            >= 32
        {
            bail!("task collaboration budget exhausted");
        }
        let mut s = if let Some(id) = collaboration {
            let s: Session = Self::get_c(tx, "session", id)?;
            if s.kind != "collaboration"
                || s.agent_id != target
                || s.scope != scope
                || s.state != "open"
            {
                bail!("collaboration not accessible or expired");
            }
            s
        } else {
            Self::session_c(tx, &a, "collaboration", &scope, true)?
        };
        s.last_used = now();
        Self::put(tx, "session", &s.id, &s)?;
        let t = Self::task_c(tx, &a, &s, prompt, Some(source_task.into()), None)?;
        Self::put(tx, "query_key", &key, &t.id)?;
        Ok(t)
    }
    pub fn claim(&self, limit: usize) -> Result<Option<Task>> {
        self.claim_lane(limit, None)
    }
    pub fn claim_agent(&self, limit: usize) -> Result<Option<Task>> {
        self.claim_lane(limit, Some(false))
    }
    pub fn claim_shell(&self, limit: usize) -> Result<Option<Task>> {
        self.claim_lane(limit, Some(true))
    }
    fn claim_lane(&self, limit: usize, shell: Option<bool>) -> Result<Option<Task>> {
        if !(1..=20).contains(&limit) {
            bail!("worker limit must be 1..20");
        }
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut tasks = {
            let mut q = tx.prepare("SELECT data FROM records WHERE kind='task' ORDER BY rowid")?;
            let rows = q.query_map([], |r| r.get::<_, String>(0))?;
            rows.map(|raw| Ok(serde_json::from_str::<Task>(&raw?)?))
                .collect::<Result<Vec<_>>>()?
        };
        let active: Vec<_> = tasks
            .iter()
            .filter(|t| shell.is_none_or(|is_shell| (t.kind == "shell") == is_shell))
            .filter(|t| matches!(t.state.as_str(), "running" | "cancel_requested"))
            .collect();
        if active.len() >= limit {
            return Ok(None);
        }
        let busy: Vec<String> = active.iter().map(|t| t.session_id.clone()).collect();
        // Serialize workspaces within each lane. Explicit /sh commands may run
        // alongside Agent tools, but two shell commands do not share a workspace.
        let mut busy_workspaces = Vec::new();
        for task in &active {
            let s: Session = Self::get_c(&tx, "session", &task.session_id)?;
            busy_workspaces.push(s.workspace);
        }
        tasks.sort_by_key(|t| t.created_at);
        for mut t in tasks {
            if shell.is_some_and(|is_shell| (t.kind == "shell") != is_shell) {
                continue;
            }
            if t.state != "queued" || busy.contains(&t.session_id) {
                continue;
            }
            let a: Agent = Self::get_c(&tx, "agent", &t.agent_id)?;
            let mut session: Session = Self::get_c(&tx, "session", &t.session_id)?;
            if session.kind == "user" && session.config_revision != a.revision {
                session.workspace = a.workspace.clone();
            }
            if busy_workspaces.contains(&session.workspace) {
                continue;
            }
            if Self::enabled(&tx, &a).is_err() {
                continue;
            }
            Self::put(&tx, "session", &session.id, &session)?;
            t.state = "running".into();
            t.generation += 1;
            Self::put(&tx, "task", &t.id, &t)?;
            tx.commit()?;
            return Ok(Some(t));
        }
        Ok(None)
    }
    pub fn finish(&self, id: &str, generation: u32, state: &str, result: &str) -> Result<bool> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let done = Self::finish_c(&tx, id, generation, state, result)?;
        tx.commit()?;
        Ok(done)
    }
    fn finish_c(
        tx: &Connection,
        id: &str,
        generation: u32,
        state: &str,
        result: &str,
    ) -> Result<bool> {
        let mut t: Task = Self::get_c(tx, "task", id)?;
        if t.generation != generation || !matches!(t.state.as_str(), "running" | "cancel_requested")
        {
            return Ok(false);
        }
        t.state = if t.state == "cancel_requested" {
            "cancelled".into()
        } else {
            state.into()
        };
        t.result = Some(result.into());
        Self::put(tx, "task", id, &t)?;
        let mut s: Session = Self::get_c(tx, "session", &t.session_id)?;
        s.last_used = now();
        Self::put(tx, "session", &s.id, &s)?;
        let suppressed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM records WHERE kind='command_task_suppression' AND id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if let Some(reply) = t.reply.as_ref().filter(|_| !suppressed) {
            tx.execute("INSERT OR IGNORE INTO deliveries(id,task_id,bot_id,target,text) VALUES(?1,?2,?3,?4,?5)",params![new_id("d"),id,t.bot_id,serde_json::to_string(reply)?,result])?;
        }
        if let Some(parent) = &t.parent_id {
            tx.execute(
                "INSERT INTO inputs(id,task_id,text) VALUES(?1,?2,?3)",
                params![
                    format!("response:{id}"),
                    parent,
                    format!(
                        "[Agent response; request={id}; collaboration={}; state={}]\n{result}",
                        t.session_id, t.state
                    )
                ],
            )?;
            let mut p: Task = Self::get_c(tx, "task", parent)?;
            if p.state == "waiting_dependency" {
                p.state = "queued".into();
                Self::put(tx, "task", parent, &p)?;
            }
        }
        Ok(true)
    }
    pub fn set_thread(&self, session: &str, thread: &str) -> Result<()> {
        let mut s: Session = self.get("session", session)?;
        s.thread_id = Some(thread.into());
        s.last_used = now();
        self.save("session", session, &s)
    }
    pub fn event(&self, task: &str, kind: &str, data: &Value) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO events(task_id,kind,data,created) VALUES(?1,?2,?3,?4)",
            params![task, kind, data.to_string(), now()],
        )?;
        Ok(())
    }
    pub fn inputs(&self, task: &str) -> Result<Vec<(String, String)>> {
        let c = self.conn()?;
        let mut q = c.prepare(
            "SELECT id,text FROM inputs WHERE task_id=?1 AND state='pending' ORDER BY rowid",
        )?;
        let rows = q.query_map([task], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn input_applied(&self, id: &str) -> Result<()> {
        self.conn()?
            .execute("UPDATE inputs SET state='applied' WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn cancel(&self, id: &str) -> Result<Task> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let tasks = Self::list_c::<Task>(&tx, "task")?;
        let root: Task = Self::get_c(&tx, "task", id)?;
        if !matches!(
            root.state.as_str(),
            "running" | "queued" | "waiting_dependency"
        ) {
            bail!("task is already terminal or cancellation requested");
        }
        let mut ids = vec![id.to_owned()];
        let mut index = 0;
        while index < ids.len() {
            let parent = ids[index].clone();
            for t in &tasks {
                if t.parent_id.as_deref() == Some(&parent) && !ids.contains(&t.id) {
                    ids.push(t.id.clone());
                }
            }
            index += 1;
        }
        for id in ids {
            let mut t: Task = Self::get_c(&tx, "task", &id)?;
            match t.state.as_str() {
                "running" => {
                    t.state = "cancel_requested".into();
                    Self::put(&tx, "task", &id, &t)?;
                }
                "queued" | "waiting_dependency" => {
                    t.state = "cancel_requested".into();
                    Self::put(&tx, "task", &id, &t)?;
                    Self::finish_c(
                        &tx,
                        &id,
                        t.generation,
                        "cancelled",
                        "Cancelled before execution",
                    )?;
                }
                _ => {}
            }
        }
        let result = Self::get_c(&tx, "task", id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn retry(&self, id: &str) -> Result<Task> {
        self.retry_with_reply(id, None)
    }
    pub fn retry_with_reply(&self, id: &str, reply: Option<ReplyTarget>) -> Result<Task> {
        let t: Task = self.get("task", id)?;
        if t.state != "failed" {
            bail!("only failed tasks can retry; interrupted tasks require outcome reconciliation");
        }
        let a: Agent = self.get("agent", &t.agent_id)?;
        let s: Session = self.get("session", &t.session_id)?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut next = Self::task_c(&tx, &a, &s, &t.prompt, t.parent_id, reply.or(t.reply))?;
        next.kind = t.kind;
        Self::put(&tx, "task", &next.id, &next)?;
        tx.execute("INSERT OR IGNORE INTO media_refs SELECT asset_id,kind,?1,?2 FROM media_refs WHERE kind='task_input' AND ref_id=?3",params![next.id,now(),id])?;
        tx.commit()?;
        Ok(next)
    }
    pub fn recover(&self) -> Result<usize> {
        let mut n = 0;
        for t in self.list::<Task>("task")? {
            if matches!(t.state.as_str(), "running" | "cancel_requested")
                && self.finish(
                    &t.id,
                    t.generation,
                    "interrupted",
                    "Service stopped during execution; reconcile external effects before retry",
                )?
            {
                n += 1;
            }
        }
        Ok(n)
    }
    pub fn clear(&self, agent: &str, scope: &str) -> Result<()> {
        let a: Agent = self.get("agent", agent)?;
        crate::media::extractions::cancel_scope(self, &a.bot_id, scope)?;
        self.conn()?.execute("UPDATE media_inbox SET state='cancelled',generation=generation+1,lease_until=0 WHERE scope=?1 AND json_extract(data,'$.agent')=?2 AND state IN ('received','preparing','blocked','failed')", params![scope,agent])?;
        for mut s in self.list::<Session>("session")? {
            if s.agent_id == agent && s.kind == "user" && s.scope == scope {
                s.state = "closed".into();
                self.save("session", &s.id, &s)?;
            }
        }
        Ok(())
    }
}

mod cron;

mod command_completion;
