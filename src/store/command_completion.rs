//! Explicit cancellation commands own one durable summary, including descendants.
use super::*;
fn active(task: &Task) -> bool {
    matches!(
        task.state.as_str(),
        "queued" | "running" | "waiting_dependency" | "cancel_requested"
    )
}
impl Store {
    fn command_cancel_c(
        tx: &Connection,
        roots: Vec<String>,
        bot: &str,
        reply: &ReplyTarget,
        text: String,
    ) -> Result<()> {
        let tasks = Self::list_c::<Task>(tx, "task")?;
        let mut ids = roots;
        let mut index = 0;
        while index < ids.len() {
            let parent = ids[index].clone();
            for t in &tasks {
                if t.parent_id.as_deref() == Some(&parent) && active(t) && !ids.contains(&t.id) {
                    ids.push(t.id.clone());
                }
            }
            index += 1;
        }
        let operation = new_id("command_completion");
        for id in &ids {
            Self::put(
                tx,
                "command_task_suppression",
                id,
                &json!({"operation":operation}),
            )?;
        }
        Self::put(
            tx,
            "command_completion",
            &operation,
            &json!({"id":operation,"tasks":ids,"bot_id":bot,"reply":reply,"text":text,"state":"pending"}),
        )?;
        // Mark every descendant before finishing any, so child completion cannot
        // wake a parent back into the execution queue.
        let mut queued = Vec::new();
        for id in &ids {
            let mut t: Task = Self::get_c(tx, "task", id)?;
            if !active(&t) {
                continue;
            }
            if matches!(t.state.as_str(), "queued" | "waiting_dependency") {
                queued.push((id.clone(), t.generation));
            }
            t.state = "cancel_requested".into();
            Self::put(tx, "task", id, &t)?;
        }
        for (id, generation) in queued {
            Self::finish_c(
                tx,
                &id,
                generation,
                "cancelled",
                "Cancelled before execution",
            )?;
        }
        Ok(())
    }
    pub fn cancel_command(&self, id: &str, reply: &ReplyTarget) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let t: Task = Self::get_c(&tx, "task", id)?;
        anyhow::ensure!(
            active(&t) && t.state != "cancel_requested",
            "任务已结束或已请求取消"
        );
        Self::command_cancel_c(
            &tx,
            vec![id.into()],
            &t.bot_id,
            reply,
            format!("任务已取消：{id}"),
        )?;
        let mut result = serde_json::to_value(Self::get_c::<Task>(&tx, "task", id)?)?;
        result["reply_deferred"] = json!(true);
        tx.commit()?;
        Ok(result)
    }
    pub fn disable_agent_command(
        &self,
        id: &str,
        force: bool,
        reply: &ReplyTarget,
    ) -> Result<Value> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut agent: Agent = Self::get_c(&tx, "agent", id)?;
        let tasks: Vec<_> = Self::list_c::<Task>(&tx, "task")?
            .into_iter()
            .filter(|t| t.agent_id == id && active(t))
            .collect();
        let jobs: Vec<_> = Self::list_c::<Job>(&tx, "job")?
            .into_iter()
            .filter(|j| j.agent_id == id && j.state == "enabled")
            .collect();
        anyhow::ensure!(
            force || (tasks.is_empty() && jobs.is_empty()),
            "active tasks or jobs; use --force"
        );
        agent.enabled = false;
        agent.revision += 1;
        Self::put(&tx, "agent", id, &agent)?;
        let paused = jobs.len();
        for mut job in jobs {
            job.state = "paused".into();
            Self::put(&tx, "job", &job.id, &job)?;
        }
        let mut result = serde_json::to_value(agent.clone())?;
        if !tasks.is_empty() {
            Self::command_cancel_c(&tx, tasks.iter().map(|t| t.id.clone()).collect(), &agent.bot_id, reply, format!("Agent 已停用：{id}；已取消 {} 个活动任务及其关联任务，暂停 {paused} 个定时任务。", tasks.len()))?;
            result["reply_deferred"] = json!(true);
        }
        tx.commit()?;
        Ok(result)
    }
    pub fn flush_command_completions(&self) -> Result<()> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for mut op in Self::list_c::<Value>(&tx, "command_completion")? {
            if op["state"] != "pending" {
                continue;
            }
            let ids = op["tasks"].as_array().context("command completion tasks")?;
            let mut ready = true;
            for id in ids {
                if active(&Self::get_c::<Task>(
                    &tx,
                    "task",
                    id.as_str().context("task id")?,
                )?) {
                    ready = false;
                    break;
                }
            }
            if !ready {
                continue;
            }
            let id = op["id"]
                .as_str()
                .context("command completion id")?
                .to_owned();
            tx.execute("INSERT OR IGNORE INTO deliveries(id,task_id,bot_id,target,text) VALUES(?1,?1,?2,?3,?4)",params![id,op["bot_id"].as_str(),op["reply"].to_string(),op["text"].as_str()])?;
            op["state"] = json!("completed");
            Self::put(&tx, "command_completion", &id, &op)?;
        }
        tx.commit()?;
        Ok(())
    }
}
