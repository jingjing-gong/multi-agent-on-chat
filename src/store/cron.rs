//! Transactional scheduling persistence; reply policy belongs to delivery.
use super::*;
impl Store {
    pub fn fire_due(&self, at: i64) -> Result<usize> {
        let replies = self
            .list::<Bot>("bot")?
            .into_iter()
            .map(|b| Ok((b.id.clone(), crate::delivery::bot_reply(self, &b)?)))
            .collect::<Result<std::collections::HashMap<_, _>>>()?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut count = 0;
        for mut j in Self::list_c::<Job>(&tx, "job")? {
            if j.state != "enabled" {
                continue;
            }
            let Some(due) = j.next_fire else {
                continue;
            };
            if due > at {
                continue;
            }
            let mut a: Agent = Self::get_c(&tx, "agent", &j.agent_id)?;
            if Self::enabled(&tx, &a).is_err() {
                j.state = "blocked".into();
                Self::put(&tx, "job", &j.id, &j)?;
                continue;
            }
            let active:i64=tx.query_row("SELECT count(*) FROM occurrences o JOIN records r ON r.kind='task' AND r.id=o.task_id WHERE o.job_id=?1 AND json_extract(r.data,'$.state') IN ('queued','running','waiting_dependency','cancel_requested','interrupted','needs_attention')",[&j.id],|r|r.get(0))?;
            let status = if active > 0 {
                "skipped_overlap"
            } else if at - due > 3600 {
                "missed"
            } else {
                "queued"
            };
            if tx.execute(
                "INSERT OR IGNORE INTO occurrences(job_id,scheduled,status) VALUES(?1,?2,?3)",
                params![j.id, due, status],
            )? == 1
                && status == "queued"
            {
                a.config = j.config.clone();
                a.revision = j.config_revision;
                a.workspace = j.workspace.clone();
                let s = Self::session_c(&tx, &a, "cron", &j.id, true)?;
                let reply = j
                    .reply
                    .clone()
                    .or_else(|| replies.get(&j.bot_id).cloned().flatten());
                let t = Self::task_c(&tx, &a, &s, &j.prompt, None, reply)?;
                tx.execute(
                    "UPDATE occurrences SET task_id=?1 WHERE job_id=?2 AND scheduled=?3",
                    params![t.id, j.id, due],
                )?;
                count += 1;
            }
            j.next_fire = crate::schedule::advance(&j.schedule, &j.timezone, due, at)?;
            if j.next_fire.is_none() {
                j.state = "completed".into();
            }
            Self::put(&tx, "job", &j.id, &j)?;
        }
        tx.commit()?;
        Ok(count)
    }
    pub fn job_create(
        &self,
        agent: &str,
        schedule: &str,
        tz: &str,
        prompt: &str,
        reply: Option<ReplyTarget>,
    ) -> Result<Job> {
        let a: Agent = self.get("agent", agent)?;
        let next =
            crate::schedule::next(schedule, tz, now())?.context("schedule has no future firing")?;
        let j = Job {
            id: new_id("job"),
            agent_id: a.id,
            bot_id: a.bot_id,
            prompt: prompt.into(),
            schedule: schedule.into(),
            timezone: tz.into(),
            config: a.config,
            config_revision: a.revision,
            next_fire: Some(next),
            state: "enabled".into(),
            reply,
            revision: 1,
            workspace: a.workspace,
        };
        self.save("job", &j.id, &j)?;
        Ok(j)
    }
    pub fn visible_jobs(&self) -> Result<Vec<Job>> {
        Ok(self
            .list::<Job>("job")?
            .into_iter()
            .filter(|job| job.state != "deleted")
            .collect())
    }
    pub fn job_update(
        &self,
        id: &str,
        schedule: Option<&str>,
        prompt: Option<&str>,
        refresh: bool,
    ) -> Result<Job> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut j: Job = Self::get_c(&tx, "job", id)?;
        if j.state == "deleted" {
            bail!("job deleted");
        }
        if let Some(spec) = schedule {
            j.next_fire = crate::schedule::next(spec, &j.timezone, now())?;
            j.schedule = spec.into();
        }
        if let Some(prompt) = prompt {
            j.prompt = prompt.into();
        }
        if refresh {
            let a: Agent = Self::get_c(&tx, "agent", &j.agent_id)?;
            j.config = a.config;
            j.config_revision = a.revision;
            j.workspace = a.workspace;
        }
        j.revision += 1;
        Self::put(&tx, "job", id, &j)?;
        tx.commit()?;
        Ok(j)
    }
    pub fn job_action(&self, id: &str, action: &str) -> Result<Job> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut j: Job = Self::get_c(&tx, "job", id)?;
        if j.state == "deleted" {
            bail!("任务已删除，只能查看详情和历史");
        }
        match action {
            "pause" => j.state = "paused".into(),
            "delete" => j.state = "deleted".into(),
            "resume" => {
                if j.state == "deleted" {
                    bail!("deleted job");
                }
                j.next_fire = crate::schedule::next(&j.schedule, &j.timezone, now())?;
                j.state = if j.next_fire.is_some() {
                    "enabled"
                } else {
                    "completed"
                }
                .into();
            }
            _ => bail!("unknown action"),
        };
        j.revision += 1;
        Self::put(&tx, "job", id, &j)?;
        tx.commit()?;
        Ok(j)
    }
    pub fn job_history(&self, id: &str) -> Result<Vec<Value>> {
        let c = self.conn()?;
        let mut q=c.prepare("SELECT o.scheduled,o.task_id,o.status,json_extract(t.data,'$.state'),json_extract(t.data,'$.result'),d.state,d.attempts FROM occurrences o LEFT JOIN records t ON t.kind='task' AND t.id=o.task_id LEFT JOIN deliveries d ON d.task_id=o.task_id WHERE o.job_id=?1 ORDER BY o.scheduled DESC LIMIT 100")?;
        let rows=q.query_map([id],|r|Ok(json!({"scheduled_for":r.get::<_,i64>(0)?,"task_id":r.get::<_,Option<String>>(1)?,"status":r.get::<_,String>(2)?,"execution_state":r.get::<_,Option<String>>(3)?,"result":r.get::<_,Option<String>>(4)?,"delivery_state":r.get::<_,Option<String>>(5)?,"delivery_attempts":r.get::<_,Option<i64>>(6)?})))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}
