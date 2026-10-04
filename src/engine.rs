//! Codex app-server adapter. No storage or channel dependencies live here.
//! Wire shapes are checked against `codex app-server generate-json-schema`.
use crate::domain::Template;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, oneshot, Mutex},
};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>>;

#[derive(Clone)]
pub struct EngineControl {
    input: Arc<Mutex<ChildStdin>>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}
impl EngineControl {
    async fn write(&self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        let mut input = self.input.lock().await;
        input.write_all(&bytes).await?;
        input.flush().await?;
        Ok(())
    }
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if let Err(error) = self
            .write(json!({"id":id,"method":method,"params":params}))
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        let result = tokio::time::timeout(Duration::from_secs(60), rx).await;
        self.pending.lock().await.remove(&id);
        match result {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => bail!("Codex {method}: {error}"),
            Ok(Err(_)) => bail!("Codex connection closed while awaiting {method}"),
            Err(_) => bail!("Codex {method} response timed out; execution state may be unknown"),
        }
    }
    pub async fn steer(&self, thread_id: &str, turn_id: &str, text: &str) -> Result<()> {
        self.request(
            "turn/steer",
            json!({"threadId":thread_id,"expectedTurnId":turn_id,"input":text_input(text)}),
        )
        .await?;
        Ok(())
    }
    pub async fn respond_tool(&self, id: Value, result: Value, success: bool) -> Result<()> {
        let text = result
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| result.to_string());
        self.write(json!({"id":id,"result":{"contentItems":[{"type":"inputText","text":text}],"success":success}})).await
    }
    pub async fn interrupt(&self, thread_id: &str, turn_id: &str) -> Result<()> {
        self.request(
            "turn/interrupt",
            json!({"threadId":thread_id,"turnId":turn_id}),
        )
        .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    Message {
        id: String,
        text: String,
        phase: Option<String>,
    },
    Delta {
        id: String,
        text: String,
    },
    MessageStarted {
        id: String,
        phase: Option<String>,
    },
    Completed {
        status: String,
        error: Option<String>,
    },
    ToolCall {
        id: Value,
        name: String,
        arguments: Value,
    },
    Other(Value),
}

pub struct CodexEngine {
    child: Child,
    control: EngineControl,
    events: mpsc::UnboundedReceiver<Value>,
    reader: tokio::task::JoinHandle<()>,
    pub thread_id: String,
    pub turn_id: Option<String>,
    default_effort: Option<String>,
}
impl CodexEngine {
    /// Query the active profile's provider without starting a conversation.
    pub async fn model_catalog(binary: &str, config: &Template, cwd: &Path) -> Result<Value> {
        let profile = load_profile(config.profile.as_deref())?;
        let engine = Self::connect_inner(binary, config, cwd, None, &[], None, true).await?;
        let result = async {
            let effective = engine
                .control
                .request("config/read", json!({"cwd":cwd,"includeLayers":false}))
                .await?;
            if let Some(provider) = catalog_provider(&effective["config"], profile.as_ref())? {
                return crate::model_catalog::provider_models(&provider).await;
            }
            let mut models = Vec::new();
            let mut cursor: Option<String> = None;
            let mut cursors = std::collections::HashSet::new();
            loop {
                let page = engine
                    .control
                    .request(
                        "model/list",
                        json!({"includeHidden":false,"limit":100,"cursor":cursor}),
                    )
                    .await?;
                models.extend(
                    page["data"]
                        .as_array()
                        .context("model/list returned no data array")?
                        .iter()
                        .cloned(),
                );
                cursor = page["nextCursor"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                match &cursor {
                    None => break,
                    Some(next) if !cursors.insert(next.clone()) => {
                        bail!("model/list repeated a pagination cursor")
                    }
                    _ => {}
                }
            }
            Ok(json!({"data":models,"nextCursor":null}))
        }
        .await;
        let _ = engine.shutdown().await;
        result
    }

    /// Discover tools without starting a model turn or operating the desktop.
    pub async fn computer_use_status(cwd: &Path) -> Result<Value> {
        let config: Template = serde_json::from_value(json!({"id":"doctor"}))?;
        let engine = Self::connect(&config, cwd, None).await?;
        let result = async {
            let mut cursor: Option<String> = None;
            loop {
                let page = engine
                    .control
                    .request("mcpServerStatus/list", json!({"limit":100,"cursor":cursor}))
                    .await?;
                if let Some(server) = page["data"]
                    .as_array()
                    .and_then(|servers| servers.iter().find(|s| s["name"] == "cua_repl"))
                {
                    return Ok(computer_use_report(server));
                }
                cursor = page["nextCursor"].as_str().map(str::to_owned);
                if cursor.is_none() {
                    return Ok(computer_use_report(&Value::Null));
                }
            }
        }
        .await;
        let _ = engine.shutdown().await;
        result
    }
    /// One isolated app-server process per active worker. Dropping it kills the
    /// process; durable thread ids allow later turns to resume their context.
    pub async fn connect(config: &Template, cwd: &Path, thread_id: Option<&str>) -> Result<Self> {
        Self::connect_binary("codex", config, cwd, thread_id).await
    }
    pub async fn connect_binary(
        binary: &str,
        config: &Template,
        cwd: &Path,
        thread_id: Option<&str>,
    ) -> Result<Self> {
        Self::connect_scoped(binary, config, cwd, thread_id, &[]).await
    }
    pub async fn connect_scoped(
        binary: &str,
        config: &Template,
        cwd: &Path,
        thread_id: Option<&str>,
        environment: &[(String, String)],
    ) -> Result<Self> {
        let overrides = if config.skills_mode == crate::domain::SkillsMode::Allowlist {
            let available = Self::skills_catalog(binary, config, cwd, environment).await?;
            Some(crate::skills::overrides(&available, &config.skills)?)
        } else {
            None
        };
        Self::connect_inner(
            binary,
            config,
            cwd,
            thread_id,
            environment,
            overrides.as_ref(),
            false,
        )
        .await
    }
    pub async fn skills_catalog(
        binary: &str,
        config: &Template,
        cwd: &Path,
        environment: &[(String, String)],
    ) -> Result<Vec<Value>> {
        let engine =
            Self::connect_inner(binary, config, cwd, None, environment, None, true).await?;
        let result = engine
            .control
            .request("skills/list", json!({"cwds":[cwd],"forceReload":true}))
            .await;
        let _ = engine.shutdown().await;
        crate::skills::catalog(result?)
    }
    #[allow(clippy::too_many_arguments)]
    async fn connect_inner(
        binary: &str,
        config: &Template,
        cwd: &Path,
        thread_id: Option<&str>,
        environment: &[(String, String)],
        overrides: Option<&Value>,
        discovery: bool,
    ) -> Result<Self> {
        let profile_config = load_profile(config.profile.as_deref())?;
        let mut command = Command::new(binary);
        command.arg("app-server");
        command.args([
            "-c",
            r#"plugins."unified-computer-use@openai-bundled".enabled=true"#,
        ]);
        if let Some(value) = overrides.or_else(|| {
            profile_config.as_ref().and_then(|p| {
                p.get("skills.config")
                    .or_else(|| p.pointer("/skills/config"))
            })
        }) {
            command.args([
                "-c",
                &format!("skills.config={}", crate::skills::toml_overrides(value)?),
            ]);
        }
        command.envs(environment.iter().cloned());
        command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().context("start codex app-server")?;
        let input = child.stdin.take().context("Codex stdin unavailable")?;
        let stdout = child.stdout.take().context("Codex stdout unavailable")?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let control = EngineControl {
            input: Arc::new(Mutex::new(input)),
            pending: pending.clone(),
            next_id: Arc::new(AtomicU64::new(1)),
        };
        let approved_apps: Vec<String> = environment
            .iter()
            .find(|(key, _)| key == "MAC_COMPUTER_USE_ALLOWED_APPS")
            .map(|(_, value)| serde_json::from_str(value))
            .transpose()
            .context("invalid Computer Use app allowlist")?
            .unwrap_or_default();
        let computer_use_enabled = !approved_apps.is_empty();
        let reader_control = control.clone();
        let (tx, events) = mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("method").is_none() {
                    if let Some(id) = value.get("id").and_then(Value::as_u64) {
                        if let Some(waiter) = pending.lock().await.remove(&id) {
                            let result = if let Some(error) = value.get("error") {
                                Err(error.to_string())
                            } else {
                                Ok(value.get("result").cloned().unwrap_or(Value::Null))
                            };
                            let _ = waiter.send(result);
                        }
                    }
                } else if value["method"] == "item/tool/call" {
                    if tx.send(value).is_err() {
                        break;
                    }
                } else if value["method"] == "mcpServer/elicitation/request"
                    && value.get("id").is_some()
                {
                    let allowed = computer_use_approved(&value["params"], &approved_apps);
                    let action = if allowed { "accept" } else { "decline" };
                    let _ = reader_control.write(json!({"id":value["id"],"result":{
                        "action":action,"content":if allowed { json!({}) } else { Value::Null },"_meta":null
                    }})).await;
                    let _ = tx.send(json!({"method":"mac/elicitationResolved","params":{
                        "server":value["params"]["serverName"],
                        "app":value["params"]["_meta"]["tool_params"]["app"],
                        "action":action,"reason":"configured Computer Use app allowlist"
                    }}));
                } else if let Some(id) = value.get("id") {
                    // No approval UI is registered: never fabricate consent or hang.
                    let _ = reader_control.write(json!({"id":id,"error":{"code":-32601,"message":"Interactive requests are unavailable in this worker"}})).await;
                    let _ = tx.send(json!({"method":"mac/unsupportedRequest","params":value}));
                } else if tx.send(value).is_err() {
                    break;
                }
            }
            for (_, waiter) in pending.lock().await.drain() {
                let _ = waiter.send(Err("app-server disconnected".into()));
            }
        });
        let mut engine = Self {
            child,
            control,
            events,
            reader,
            thread_id: String::new(),
            turn_id: None,
            default_effort: None,
        };
        engine.control.request("initialize", json!({"clientInfo":{"name":"multi-agent-on-chat","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        engine
            .control
            .write(json!({"method":"initialized"}))
            .await?;
        if discovery {
            return Ok(engine);
        }
        if overrides.is_some() {
            let found = crate::skills::catalog(
                engine
                    .control
                    .request("skills/list", json!({"cwds":[cwd],"forceReload":true}))
                    .await?,
            )?;
            crate::skills::verify(&found, &config.skills)?;
        }
        let effective = engine
            .control
            .request("config/read", json!({"cwd":cwd,"includeLayers":false}))
            .await?;
        engine.default_effort = profile_config
            .as_ref()
            .and_then(|v| v["model_reasoning_effort"].as_str())
            .or_else(|| effective["config"]["model_reasoning_effort"].as_str())
            .map(str::to_owned);
        let mut params = json!({"cwd":cwd,"approvalPolicy":"never","sandbox":"danger-full-access"});
        if computer_use_enabled {
            // Let MCP app consent reach the adapter; keep shell/rules approvals disabled.
            params["approvalPolicy"] = json!({"granular":{
                "mcp_elicitations":true,"sandbox_approval":false,"rules":false,
                "request_permissions":false,"skill_approval":false
            }});
        }

        if let Some(profile_config) = profile_config {
            params["config"] = profile_config;
        }
        if !params["config"].is_object() {
            params["config"] = json!({});
        }
        apply_default_computer_use(&mut params["config"]);
        // Explicit host-level execution policy for both new and resumed Bot threads.
        params["config"]["sandbox_mode"] = json!("danger-full-access");
        if let Some(value) = overrides {
            if !params["config"].is_object() {
                params["config"] = json!({});
            }
            params["config"]["skills.config"] = value.clone();
            if let Some(object) = params["config"].as_object_mut() {
                object.remove("skills");
            }
        }
        if let Some(model) = &config.model {
            params["model"] = json!(model);
        }
        if !config.system_prompt.is_empty() {
            params["developerInstructions"] = json!(config.system_prompt);
        }
        let method = if let Some(id) = thread_id {
            params["threadId"] = json!(id);
            "thread/resume"
        } else {
            if !config.system_prompt.is_empty() {
                params["developerInstructions"] = json!(config.system_prompt);
            }
            params["dynamicTools"] = dynamic_tools();
            "thread/start"
        };
        let result = engine.control.request(method, params).await?;
        engine.thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("Codex did not return thread id")?
            .into();
        Ok(engine)
    }
    pub fn control(&self) -> EngineControl {
        self.control.clone()
    }
    pub async fn start_turn(&mut self, prompt: &str, config: &Template) -> Result<String> {
        self.start_turn_with_images(prompt, config, &[]).await
    }
    pub async fn start_turn_with_images(
        &mut self,
        prompt: &str,
        config: &Template,
        images: &[std::path::PathBuf],
    ) -> Result<String> {
        let mut params = json!({"threadId":self.thread_id,"input":text_input(prompt)});
        for path in images {
            anyhow::ensure!(
                path.is_absolute() && path.is_file(),
                "invalid local image input"
            );
            params["input"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"localImage","path":path}));
        }
        if let Some(model) = &config.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = config.effort.as_ref().or(self.default_effort.as_ref()) {
            params["effort"] = json!(effort);
        }
        let response = self.control.request("turn/start", params).await?;
        let id = response
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .context("Codex did not return turn id")?
            .to_owned();
        self.turn_id = Some(id.clone());
        Ok(id)
    }
    pub async fn next_event(&mut self) -> Result<EngineEvent> {
        loop {
            let event = self
                .events
                .recv()
                .await
                .ok_or_else(|| anyhow!("Codex event stream closed"))?;
            let params = &event["params"];
            if let Some(thread) = params["threadId"].as_str() {
                if thread != self.thread_id {
                    continue;
                }
            }
            if let Some(turn) = params["turnId"]
                .as_str()
                .or_else(|| params.pointer("/turn/id").and_then(Value::as_str))
            {
                if self.turn_id.as_deref().is_some_and(|id| id != turn) {
                    continue;
                }
            }
            return Ok(decode_event(event));
        }
    }
    pub async fn shutdown(mut self) -> Result<()> {
        self.reader.abort();
        self.child.kill().await?;
        let _ = self.child.wait().await;
        Ok(())
    }
}
impl Drop for CodexEngine {
    fn drop(&mut self) {
        self.reader.abort();
        let _ = self.child.start_kill();
    }
}
fn text_input(text: &str) -> Value {
    json!([{"type":"text","text":text,"text_elements":[]}])
}
/// Resolve native Codex profiles without printing their possibly secret contents.
fn merge_profile_config(base: &mut Value, overrides: &Value) {
    if let (Some(base), Some(overrides)) = (base.as_object_mut(), overrides.as_object()) {
        for (key, value) in overrides {
            if let Some(existing) = base.get_mut(key) {
                merge_profile_config(existing, value);
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = overrides.clone();
    }
}

fn catalog_provider(defaults: &Value, profile: Option<&Value>) -> Result<Option<Value>> {
    let mut effective = defaults.clone();
    if let Some(profile) = profile {
        merge_profile_config(&mut effective, profile);
    }
    let name = effective["model_provider"].as_str().unwrap_or("openai");
    let provider = &effective["model_providers"][name];
    if name == "openai" && provider["base_url"].as_str().is_none() {
        return Ok(None);
    }
    if provider["base_url"]
        .as_str()
        .filter(|s| !s.is_empty())
        .is_none()
    {
        bail!("当前 profile 的模型 provider 缺少 base_url，无法动态查询模型列表");
    }
    Ok(Some(provider.clone()))
}

/// The app-server CLI itself does not accept --profile; thread config is its
/// supported equivalent override surface.
fn codex_home() -> Result<std::path::PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".codex")))
        .context("Codex home unavailable")
}
fn valid_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn profile_toml(path: &Path) -> Result<toml::Value> {
    let data = std::fs::read_to_string(path).context("cannot read Codex profile configuration")?;
    if data.len() > 4 * 1024 * 1024 {
        bail!("Codex configuration too large");
    }
    toml::from_str(&data)
        .map_err(|_| anyhow!("invalid TOML in Codex configuration (contents withheld)"))
}
fn load_profile_at(home: &Path, name: &str) -> Result<Value> {
    if !valid_profile_name(name) {
        bail!("invalid profile name");
    }
    let config = home.join("config.toml");
    if config.exists() {
        let parsed = profile_toml(&config)?;
        if let Some(profile) = parsed.get("profiles").and_then(|p| p.get(name)) {
            if !profile.is_table() {
                bail!("profile must be a TOML table");
            }
            return Ok(serde_json::to_value(profile)?);
        }
    }
    let path = home.join(format!("{name}.config.toml"));
    if !path.is_file() {
        bail!("profile not found; use /profile list");
    }
    Ok(serde_json::to_value(profile_toml(&path)?)?)
}
fn load_profile(profile: Option<&str>) -> Result<Option<Value>> {
    match profile.filter(|p| !p.is_empty() && *p != "default") {
        Some(name) => Ok(Some(load_profile_at(&codex_home()?, name)?)),
        None => Ok(None),
    }
}
pub fn validate_profile(name: &str) -> Result<()> {
    load_profile(Some(name))?;
    Ok(())
}
fn profile_names_at(home: &Path) -> Result<Vec<String>> {
    let mut names = std::collections::BTreeSet::new();
    names.insert("default".to_owned());
    let path = home.join("config.toml");
    if path.exists() {
        let config = profile_toml(&path)?;
        if let Some(profiles) = config.get("profiles").and_then(toml::Value::as_table) {
            names.extend(profiles.keys().filter(|n| valid_profile_name(n)).cloned());
        }
    }
    if home.exists() {
        for entry in std::fs::read_dir(home)? {
            let entry = entry?;
            if let Some(name) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.strip_suffix(".config.toml"))
            {
                if valid_profile_name(name) && entry.path().is_file() {
                    names.insert(name.to_owned());
                }
            }
        }
    }
    Ok(names.into_iter().collect())
}
pub fn profile_names() -> Result<Vec<String>> {
    profile_names_at(&codex_home()?)
}
fn dynamic_tools() -> Value {
    let mut tools = json!([
        {"type":"function","name":"media_inspect","description":"Inspect attachment available to this conversation.","inputSchema":{"type":"object","properties":{"asset_id":{"type":"string"}},"required":["asset_id"],"additionalProperties":false}},
        {"type":"function","name":"media_extract","description":"Request background local text/PDF parsing, or query status/result/cancel/retry by extraction_id. Long results use offset/limit. Queued is not completed; release the turn and await completion notice.","inputSchema":{"type":"object","properties":{"asset_id":{"type":"string"},"extraction_id":{"type":"string"},"action":{"type":"string","enum":["request","status","result","cancel","retry"]},"options":{"type":"object","properties":{"start_page":{"type":"integer","minimum":1},"end_page":{"type":"integer","minimum":1}},"additionalProperties":false},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":16000}},"additionalProperties":false}},
        {"type":"function","name":"media_publish","description":"Publish a user-requested result file from this workspace to current conversation; never publish unrelated local files.","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"caption":{"type":"string"}},"required":["path"],"additionalProperties":false}},
        {"type":"function","name":"agent_profile","description":"Show/list Codex profiles or switch this Agent's profile when explicitly requested by the user. Set name=default to restore Codex defaults. No secret config contents are exposed. Existing explicit model/effort overrides remain in force.","inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["show","list","set"]},"name":{"type":"string"}},"required":["action"],"additionalProperties":false}},
        {"type":"function","name":"bot_add","description":"Add a Bot only when the user requests it. Returns a running registration; poll bot_command add-status with bot_id=name. Lark uses QR login when app_id/app_secret are omitted, or both credentials when provided (never echo credentials). QR PNG and URL are sent to the originating chat. Optional owner and brand. WeChat sends a login link to the originating chat. Creates a default Agent. Other apps must bind their own owner.","inputSchema":{"type":"object","properties":{"channel":{"type":"string","enum":["lark","wechat","local"]},"name":{"type":"string"},"app_id":{"type":"string"},"app_secret":{"type":"string"},"owner":{"type":"string"},"brand":{"type":"string","enum":["feishu","lark"]}},"required":["channel","name"],"additionalProperties":false}},
        {"type":"function","name":"bot_command","description":"Inspect your owning Bot and discover Bot commands. List/show/help are read-only. Enable/disable mutate the specified Bot (default: your Bot) and must only be used when the user explicitly requests that action. Disabling stops the current task and messages; another enabled Bot or local CLI can re-enable.","inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["show","status","list","help","enable","disable","add-status"]},"bot_id":{"type":"string"}},"required":["action"],"additionalProperties":false}},
        {"type":"function","name":"template_from_agent","description":"When requested, save the current or another Agent of your Bot as a template, overwriting the same name/version if it exists. Copies configuration only, no conversation history or Bot credentials. Existing Agent snapshots remain unchanged. Omitted version defaults to 1.","inputSchema":{"type":"object","properties":{"name":{"type":"string"},"agent_id":{"type":"string"},"version":{"type":"integer","minimum":1},"description":{"type":"string"}},"required":["name"],"additionalProperties":false}},
        {"type":"function","name":"agent_list","description":"List available agents and their owning bots.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"type":"function","name":"agent_query","description":"Send an asynchronous query. Finish this turn if waiting for its response to release your worker. Results arrive as input. Reuse collaboration_id for followups. No reply to responses is required. Sender identity is bound by the host.","inputSchema":{"type":"object","properties":{"target":{"type":"string"},"prompt":{"type":"string"},"collaboration_id":{"type":"string"},"idempotency_key":{"type":"string"}},"required":["target","prompt","idempotency_key"],"additionalProperties":false}}
    ]);
    for tool in tools.as_array_mut().expect("static tools array") {
        tool["inputSchema"]["properties"]["raw"] = json!({"type":"boolean","description":"Return original structured JSON instead of concise Markdown."});
        let description = tool["description"].as_str().unwrap_or_default().to_owned();
        tool["description"] = json!(format!(
            "{description} Results are Markdown by default; raw=true returns full structured JSON."
        ));
    }
    tools
}

fn decode_event(value: Value) -> EngineEvent {
    let params = &value["params"];
    match value["method"].as_str().unwrap_or("") {
        "item/tool/call" => EngineEvent::ToolCall {
            id: value["id"].clone(),
            name: params["tool"].as_str().unwrap_or_default().into(),
            arguments: params["arguments"].clone(),
        },
        "item/agentMessage/delta" => EngineEvent::Delta {
            id: params["itemId"].as_str().unwrap_or_default().into(),
            text: params["delta"].as_str().unwrap_or_default().into(),
        },
        "item/started" if params["item"]["type"] == "agentMessage" => EngineEvent::MessageStarted {
            id: params["item"]["id"].as_str().unwrap_or_default().into(),
            phase: params["item"]["phase"].as_str().map(str::to_owned),
        },
        "item/completed" if params["item"]["type"] == "agentMessage" => EngineEvent::Message {
            id: params["item"]["id"].as_str().unwrap_or_default().into(),
            text: params["item"]["text"].as_str().unwrap_or_default().into(),
            phase: params["item"]["phase"].as_str().map(str::to_owned),
        },
        "turn/completed" => EngineEvent::Completed {
            status: params["turn"]["status"]
                .as_str()
                .unwrap_or("unknown")
                .into(),
            error: params["turn"]["error"]["message"]
                .as_str()
                .map(str::to_owned),
        },
        _ => EngineEvent::Other(value),
    }
}

fn apply_default_computer_use(config: &mut Value) {
    if let Some(plugin) = config
        .get_mut("plugins")
        .and_then(|v| v.get_mut("unified-computer-use@openai-bundled"))
        .and_then(Value::as_object_mut)
    {
        plugin.remove("enabled");
    }
    config[r#"plugins."unified-computer-use@openai-bundled".enabled"#] = json!(true);
}

fn computer_use_approved(params: &Value, approved_apps: &[String]) -> bool {
    let meta = &params["_meta"];
    params["serverName"] == "cua_repl"
        && params["mode"] == "form"
        && meta["codex_approval_kind"] == "mcp_tool_call"
        && meta["connector_id"] == "computer-use"
        && meta["tool_name"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
        && meta["tool_params"]["app"]
            .as_str()
            .is_some_and(|app| approved_apps.iter().any(|a| a == app))
        && params["requestedSchema"]["type"] == "object"
        && params["requestedSchema"]["properties"]
            .as_object()
            .is_some_and(|p| p.is_empty())
        && params["requestedSchema"]
            .get("required")
            .is_none_or(|r| r.as_array().is_some_and(|a| a.is_empty()))
}

fn computer_use_report(server: &Value) -> Value {
    let loaded = server["tools"]
        .as_object()
        .is_some_and(|tools| tools.contains_key("js"));
    json!({
        "tools_loaded": loaded, "server": "cua_repl", "tool": "js",
        "interaction_verified": false, "browser_requires_active_turn": true,
        "note": if loaded {
            "Computer Use tool loaded. UI access and macOS permissions are checked when called within a turn."
        } else {
            "Computer Use tool not loaded. Check the unified-computer-use plugin in Codex configuration."
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_catalog_routes_each_profile_to_its_effective_provider() {
        let defaults = json!({"model_provider":"openai","model_providers":{
            "first":{"base_url":"http://first/v1","env_key":"FIRST_KEY"},
            "second":{"base_url":"http://second/v1"}
        }});
        assert!(catalog_provider(&defaults, None).unwrap().is_none());
        let first = catalog_provider(&defaults, Some(&json!({"model_provider":"first"})))
            .unwrap()
            .unwrap();
        assert_eq!(first["base_url"], "http://first/v1");
        assert_eq!(first["env_key"], "FIRST_KEY");
        let second = catalog_provider(&defaults, Some(&json!({"model_provider":"second"})))
            .unwrap()
            .unwrap();
        assert_eq!(second["base_url"], "http://second/v1");
        let legacy = catalog_provider(&defaults, Some(&json!({
            "model_provider":"first","model_providers":{"first":{"base_url":"http://override/v1"}}
        }))).unwrap().unwrap();
        assert_eq!(legacy["base_url"], "http://override/v1");
        assert_eq!(legacy["env_key"], "FIRST_KEY");
        assert!(catalog_provider(&defaults, Some(&json!({"model_provider":"missing"}))).is_err());
        assert_eq!(
            catalog_provider(
                &json!({"model_provider":"first","model_providers":defaults["model_providers"]}),
                None
            )
            .unwrap()
            .unwrap(),
            first
        );
        assert!(catalog_provider(
            &json!({"model_providers":{"openai":{"base_url":"http://proxy/v1"}}}),
            None
        )
        .unwrap()
        .is_some());
    }
    #[test]
    fn all_profiles_enable_computer_use_without_changing_other_plugins() {
        let mut config = json!({"plugins":{"unified-computer-use@openai-bundled":{"enabled":false},"other":{"enabled":false}},"model":"custom"});
        apply_default_computer_use(&mut config);
        assert_eq!(
            config[r#"plugins."unified-computer-use@openai-bundled".enabled"#],
            true
        );
        assert!(config["plugins"]["unified-computer-use@openai-bundled"]
            .get("enabled")
            .is_none());
        assert_eq!(config["plugins"]["other"]["enabled"], false);
        assert_eq!(config["model"], "custom");
    }
    #[test]
    fn app_consent_matches_only_explicit_computer_use_grants() {
        let apps = vec!["com.apple.systempreferences".to_owned()];
        let request = json!({"serverName":"cua_repl","mode":"form",
            "_meta":{"connector_id":"computer-use","codex_approval_kind":"mcp_tool_call",
                "tool_name":"get_app_state","tool_params":{"app":"com.apple.systempreferences"}},
            "requestedSchema":{"type":"object","properties":{}}});
        assert!(computer_use_approved(&request, &apps));
        assert!(!computer_use_approved(&request, &[]));
        for (pointer, value) in [
            ("/serverName", json!("untrusted")),
            ("/mode", json!("url")),
            ("/_meta/connector_id", json!("computer-use-audio")),
            ("/_meta/tool_params/app", json!("com.apple.Terminal")),
            (
                "/requestedSchema/properties",
                json!({"password":{"type":"string"}}),
            ),
        ] {
            let mut changed = request.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(!computer_use_approved(&changed, &apps), "{pointer}");
        }
    }

    #[test]
    fn computer_use_diagnostic_distinguishes_loaded_tools_from_ui_access() {
        for server in [
            Value::Null,
            json!({"tools":{}}),
            json!({"tools":{"js_reset":{}}}),
        ] {
            assert_eq!(computer_use_report(&server)["tools_loaded"], false);
        }
        let report = computer_use_report(&json!({"tools":{"js":{}}}));
        assert_eq!(report["tools_loaded"], true);
        assert_eq!(report["interaction_verified"], false);
        assert_eq!(report["browser_requires_active_turn"], true);
    }
    #[test]
    fn profiles_support_native_tables_and_legacy_files_without_exposing_secrets() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("config.toml"),
            "model='base'\n[profiles.work]\nmodel='work-model'\n",
        )
        .unwrap();
        std::fs::write(d.path().join("work.config.toml"), "model='legacy'\n").unwrap();
        std::fs::write(d.path().join("other.config.toml"), "model='other-model'\n").unwrap();
        assert_eq!(
            profile_names_at(d.path()).unwrap(),
            vec!["default", "other", "work"]
        );
        assert_eq!(
            load_profile_at(d.path(), "work").unwrap()["model"],
            "work-model"
        );
        assert_eq!(
            load_profile_at(d.path(), "other").unwrap()["model"],
            "other-model"
        );
        assert!(load_profile_at(d.path(), "missing").is_err());
        assert!(load_profile_at(d.path(), "../work").is_err());
    }
    #[test]
    fn failed_turn_is_not_success() {
        let event = decode_event(
            json!({"method":"turn/completed","params":{"turn":{"status":"failed","error":{"message":"quota"}}}}),
        );
        assert!(
            matches!(event, EngineEvent::Completed {status, error:Some(error)} if status == "failed" && error == "quota")
        );
    }
    #[test]
    fn completed_message_preserves_phase() {
        assert!(
            matches!(decode_event(json!({"method":"item/started","params":{"item":{"type":"agentMessage","id":"msg","phase":"final_answer"}}})), EngineEvent::MessageStarted{id,phase:Some(phase)} if id=="msg" && phase=="final_answer")
        );
        assert!(
            matches!(decode_event(json!({"method":"item/agentMessage/delta","params":{"itemId":"msg","delta":"hello"}})), EngineEvent::Delta{id,text} if id=="msg" && text=="hello")
        );
        let event = decode_event(
            json!({"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m","text":"done","phase":"final_answer"}}}),
        );
        assert!(
            matches!(event, EngineEvent::Message {phase:Some(phase),text,..} if phase == "final_answer" && text == "done")
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn protocol_handles_interleaved_notifications_and_exact_turn_steering() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("fake-codex");
        std::fs::write(&executable, r#"#!/usr/bin/env python3
import sys,json
def send(value):
    print(json.dumps(value),flush=True)
for line in sys.stdin:
    message=json.loads(line)
    method=message.get('method')
    if 'id' not in message: continue
    if method is None:
        assert message['id']=='tool-call-1'
        assert message['result']['success'] is True
        assert json.loads(message['result']['contentItems'][0]['text'])['agents']==[]
        continue
    if method=='initialize': assert message['params']['capabilities']['experimentalApi'] is True
    if method=='thread/start': assert {t['name'] for t in message['params']['dynamicTools']}=={'agent_list','agent_query','bot_command','template_from_agent','bot_add','agent_profile','media_inspect','media_extract','media_publish'}
    result={}
    if method=='config/read': result={'config':{'model_reasoning_effort':'low'}}
    if method in ['thread/start','thread/resume']:
        import os
        assert message['params']['sandbox']=='danger-full-access'
        assert message['params']['config']['sandbox_mode']=='danger-full-access'
        assert 'sandbox_workspace_write' not in message['params']['config']
        result={'thread':{'id':'thread-1'}}
    if method=='turn/start':
        assert message['params']['input'][1]['type']=='localImage'
        assert message['params']['input'][1]['path'].endswith('input.png')
        assert message['params']['effort']=='low'
        send({'method':'item/completed','params':{'threadId':'thread-1','turnId':'turn-1','item':{'type':'agentMessage','id':'m','phase':'commentary','text':'working'}}})
        result={'turn':{'id':'turn-1'}}
    if method=='turn/steer':
        assert message['params']['expectedTurnId']=='turn-1'
        assert message['params']['input'][0]['text']=='correction'
    send({'id':message['id'],'result':result})
    if method=='turn/steer':
        send({'id':'tool-call-1','method':'item/tool/call','params':{'threadId':'thread-1','turnId':'turn-1','tool':'agent_list','arguments':{}}})
        send({'method':'turn/completed','params':{'threadId':'thread-1','turn':{'id':'stale-turn','status':'failed'}}})
        send({'method':'turn/completed','params':{'threadId':'thread-1','turn':{'id':'turn-1','status':'completed'}}})
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let template = Template {
            id: "test".into(),
            version: 1,
            description: String::new(),
            system_prompt: "role".into(),
            profile: None,
            model: None,
            effort: None,
            skills: vec![],
            skills_mode: Default::default(),
        };
        let mut engine = CodexEngine::connect_scoped(
            executable.to_str().unwrap(),
            &template,
            temp.path(),
            None,
            &[(
                "MAC_DATA_DIR".into(),
                temp.path().to_string_lossy().into_owned(),
            )],
        )
        .await
        .unwrap();
        assert_eq!(engine.thread_id, "thread-1");
        let image = temp.path().join("input.png");
        std::fs::write(&image, b"fixture").unwrap();
        let turn = engine
            .start_turn_with_images("request", &template, &[image])
            .await
            .unwrap();
        assert_eq!(turn, "turn-1");
        engine
            .control()
            .steer(&engine.thread_id, &turn, "correction")
            .await
            .unwrap();
        assert!(
            matches!(engine.next_event().await.unwrap(), EngineEvent::Message {text,..} if text == "working")
        );
        match engine.next_event().await.unwrap() {
            EngineEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(name, "agent_list");
                assert_eq!(arguments, json!({}));
                engine
                    .control()
                    .respond_tool(id, json!({"agents":[]}), true)
                    .await
                    .unwrap();
            }
            event => panic!("expected tool call: {event:?}"),
        }
        assert!(
            matches!(engine.next_event().await.unwrap(), EngineEvent::Completed {status,..} if status == "completed")
        );
        engine.shutdown().await.unwrap();
        let resumed = CodexEngine::connect_scoped(
            executable.to_str().unwrap(),
            &template,
            temp.path(),
            Some("thread-1"),
            &[(
                "MAC_DATA_DIR".into(),
                temp.path().to_string_lossy().into_owned(),
            )],
        )
        .await
        .unwrap();
        resumed.shutdown().await.unwrap();
    }
}
