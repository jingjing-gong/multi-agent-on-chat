//! Command results stay structured internally; this module owns user-facing output.
use serde_json::Value;
pub const COMMANDS: &[&str] = &[
    "/stream",
    "/media",
    "/system",
    "/ask",
    "/recv",
    "/collaborations",
    "/collaboration",
    "/help",
    "/bot",
    "/template",
    "/status",
    "/tasks",
    "/task",
    "/cancel",
    "/retry",
    "/agent",
    "/profile",
    "/model",
    "/cd",
    "/clear",
    "/compact",
    "/skills",
    "/sh",
    "/cron",
    "/inbox",
    "confirm",
    "确认",
    "cancel",
    "取消",
];
/// A quoted/escaped token remains an argument; only a bare final -r is reserved.
pub fn split_raw(text: &str) -> (&str, bool) {
    let text = text.trim();
    if !COMMANDS.contains(&text.split_whitespace().next().unwrap_or("")) {
        return (text, false);
    }
    let Some(prefix) = text.strip_suffix("-r") else {
        return (text, false);
    };
    if !prefix.ends_with(char::is_whitespace) {
        return (text, false);
    }
    let (mut quote, mut escaped) = (None, false);
    for c in prefix.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if Some(c) == quote {
            quote = None;
        } else if quote.is_none() && (c == '\'' || c == '"') {
            quote = Some(c);
        }
    }
    if quote.is_some() || escaped {
        (text, false)
    } else {
        (prefix.trim_end(), true)
    }
}
fn plain(v: &Value) -> String {
    match v {
        Value::Null => "默认 / 未设置".into(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(plain).collect::<Vec<_>>().join("、"),
        Value::Object(map) => {
            if let Some(effort) = map.get("reasoningEffort") {
                return plain(effort);
            }
            map.iter()
                .map(|(k, v)| format!("{k}: {}", plain(v)))
                .collect::<Vec<_>>()
                .join("；")
        }
        _ => v.to_string(),
    }
}
fn esc(s: &str) -> String {
    esc_limit(s, 240)
}
fn esc_limit(s: &str, limit: usize) -> String {
    let mut out = String::new();
    for c in s.chars().take(limit) {
        match c {
            '\n' | '\r' | '\t' => out.push(' '),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '\\' | '|' | '*' | '_' | '[' | ']' | '`' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    if s.chars().count() > limit {
        out.push_str("…（-r 查看完整内容）");
    }
    out
}
fn cell(v: &Value) -> String {
    esc(&plain(v))
}
fn inherited_setting(v: &Value) -> String {
    if v.is_null() {
        "继承 Profile / Codex 默认".into()
    } else {
        cell(v)
    }
}
fn display(v: &Value, key: &str) -> String {
    let value = field(v, key);
    if matches!(
        key,
        "next_fire" | "expires_at" | "last_used" | "scheduled_for"
    ) {
        if let Some(time) = value
            .as_i64()
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        {
            return time
                .with_timezone(&chrono_tz::Asia::Shanghai)
                .format("%Y-%m-%d %H:%M:%S +08:00")
                .to_string();
        }
        if value.is_null() {
            return "无".into();
        }
    }
    cell(value)
}
fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
    if k.is_empty() {
        v
    } else if k.starts_with('/') {
        v.pointer(k).unwrap_or(&Value::Null)
    } else {
        &v[k]
    }
}
fn fields(title: &str, v: &Value, keys: &[(&str, &str)]) -> String {
    let mut out = format!("**{title}**\n");
    for (k, label) in keys {
        out.push_str(&format!("\n- {label}：{}", display(v, k)));
    }
    out
}
fn table(title: &str, v: &Value, keys: &[(&str, &str)]) -> String {
    let rows = v.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return format!("**{title}**\n\n暂无记录。");
    }
    let mut out = format!(
        "**{title}**（共 {} 项）\n\n| {} |\n| {} |\n",
        rows.len(),
        keys.iter().map(|(_, l)| *l).collect::<Vec<_>>().join(" | "),
        vec!["---"; keys.len()].join(" | ")
    );
    let limit = if title.ends_with("帮助") || title == "Bot 命令" {
        rows.len()
    } else {
        20
    };
    for row in rows.iter().take(limit) {
        out.push_str(&format!(
            "| {} |\n",
            keys.iter()
                .map(|(k, _)| display(row, k))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    if rows.len() > limit {
        out.push_str("\n仅展示前 20 项；--page 2 查看下一页，末尾加 -r 导出全部。\n");
    }
    out
}
const BOT: &[(&str, &str)] = &[
    ("id", "Bot"),
    ("channel", "渠道"),
    ("enabled", "启用"),
    ("app_id", "App ID"),
    ("listener_state", "消息监听"),
    ("listener_error", "监听错误"),
];
const AGENT: &[(&str, &str)] = &[
    ("id", "Agent"),
    ("bot_id", "Bot"),
    ("enabled", "启用"),
    ("/config/id", "模板"),
    ("/config/profile", "Profile"),
    ("/config/model", "模型"),
    ("/config/effort", "推理强度"),
];
const TASK: &[(&str, &str)] = &[
    ("id", "任务"),
    ("agent_id", "Agent"),
    ("kind", "类型"),
    ("state", "状态"),
    ("prompt", "请求"),
    ("result", "结果"),
];
const TEMPLATE: &[(&str, &str)] = &[
    ("id", "模板"),
    ("version", "版本"),
    ("description", "说明"),
    ("profile", "Profile"),
    ("model", "模型"),
    ("effort", "推理强度"),
    ("skills_mode", "Skills 模式"),
    ("skills", "Skills"),
];
const JOB: &[(&str, &str)] = &[
    ("id", "定时任务"),
    ("agent_id", "Agent"),
    ("state", "状态"),
    ("schedule", "计划"),
    ("timezone", "时区"),
    ("next_fire", "下次执行（北京时间）"),
    ("prompt", "指令"),
];
fn onboarding(v: &Value) -> String {
    if v.get("result").is_none() && v.get("state").is_some() {
        return fields("Bot 新增进度", v, &[("id", "Bot"), ("state", "状态")]);
    }
    if v.get("result").is_some() {
        format!(
            "{}\n\n{}",
            fields("Bot 新增进度", v, &[("id", "Bot"), ("state", "状态")]),
            if v["result"].is_object() {
                onboarding(&v["result"])
            } else {
                cell(&v["result"])
            }
        )
    } else if v["accepted"] == true {
        fields(
            "已开始新增 Bot",
            v,
            &[("name", "Bot"), ("status", "状态"), ("check", "查询进度")],
        )
    } else {
        fields(
            "Bot 新增结果",
            v,
            &[
                ("bot_id", "Bot"),
                ("channel", "渠道"),
                ("enabled", "启用"),
                ("pending_owner", "等待绑定 owner"),
                ("default_agent", "默认 Agent"),
            ],
        )
    }
}
pub fn error(command: &str, message: &str) -> String {
    if split_raw(command).1 {
        return serde_json::json!({"error":message}).to_string();
    }
    format!(
        "**{} 执行失败**\n\n{}",
        esc(command.split_whitespace().next().unwrap_or("命令")),
        esc(message)
    )
}
pub fn render(command: &str, v: &Value) -> String {
    let (text, raw) = split_raw(command);
    if raw {
        return serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    }
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        return error(text, &plain(e));
    }
    if v.get("help_topic").is_some() {
        return format!(
            "**{} 用法**\n\n{}\n\n{}\n\n{}",
            cell(&v["help_topic"]),
            cell(&v["description"]),
            v["commands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| format!("- {}", cell(s)))
                .collect::<Vec<_>>()
                .join("\n"),
            cell(&v["note"])
        );
    }
    if v.get("page").is_some() && v["items"].is_array() {
        let mut body = String::new();
        for items in v["items"].as_array().unwrap().chunks(20) {
            let content = match v["result_key"].as_str() {
                Some(key) => {
                    let mut data = v["context"].clone();
                    data[key] = serde_json::json!(items);
                    data
                }
                None => serde_json::json!(items),
            };
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(&render(text, &content));
        }
        let content = match v["result_key"].as_str() {
            Some(key) => {
                let mut data = v["context"].clone();
                data[key] = v["items"].clone();
                data
            }
            None => v["items"].clone(),
        };
        return format!(
            "{}\n\n第 {} 页 · 共 {} 项{}",
            if body.is_empty() {
                render(text, &content)
            } else {
                body
            },
            cell(&v["page"]),
            cell(&v["total"]),
            if v["has_more"] == true {
                "；使用 --page 下一页页码 继续查看"
            } else {
                ""
            }
        );
    }
    let mut p = text.split_whitespace();
    let cmd = p.next().unwrap_or("");
    let action = p.next().unwrap_or("");
    match cmd {
        "/stream" => fields("回复模式", v, &[("bot_id", "Bot"), ("channel", "渠道"), ("mode", "当前模式"), ("source", "设置来源"), ("default", "渠道默认"), ("notice", "说明")]),
        "/media" if v.get("source_asset_id").is_some() => {
            let mut out=fields("附件解析",v,&[("id","任务"),("state","状态"),("source_asset_id","源附件"),("result_available","结果可用")]);
            for segment in v["segments"].as_array().into_iter().flatten() {
                out.push_str(&format!("{}{}: {}",char::from(10),segment["page"].as_u64().map(|p|format!("第 {p} 页")).unwrap_or_else(||"文本".into()),esc_limit(segment["text"].as_str().unwrap_or(""),16000)));
            }
            if let Some(offset)=v["next_offset"].as_u64(){out.push_str(&format!("{}继续：/media result {} {}",char::from(10),v["id"].as_str().unwrap_or(""),offset));}
            if v["truncated"]==true {out.push_str("（仅解析选定范围或预算内内容）");}
            if let Some(w)=v["warnings"].as_array().filter(|w|!w.is_empty()){out.push_str(&format!("{}提示：{}",char::from(10),plain(&Value::Array(w.clone()))));}
            out
        }
        "/bot" if action=="doctor" => fields("Bot 能力诊断",v,&[("bot_id","Bot"),("channel","渠道"),("media","多媒体")]),
        "/media" => {
            if let Some(items) = v["assets"].as_array() {
                let mut lines = vec!["**当前会话附件**".to_owned()];
                for a in items { lines.push(format!("{} · {} · {} bytes · {}",cell(&a["id"]),cell(&a["name"]),cell(&a["size"]),cell(&a["state"]))); }
                for p in v["preparations"].as_array().into_iter().flatten() {lines.push(format!("准备 {} · {} {}",cell(&p["id"]),cell(&p["state"]),p["error"].as_str().map(esc).unwrap_or_default()));}
                if lines.len()==1 {lines.push("暂无附件".into());}
                lines.join("\n\n")
            } else {fields("附件",v,&[("id","ID"),("name","文件名"),("mime","格式"),("size","字节数"),("state","状态")])}
        }
        "/help" => v["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| {
                format!(
                    "**{}**\n{}",
                    cell(&entry["usage"]),
                    cell(&entry["description"])
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        "/bot" if action == "help" => format!(
            "{}\n\n{}",
            table("Bot 命令", &v["commands"], &[("", "命令")]),
            cell(&v["note"])
        ),
        "/bot" if action == "list" => table("Bot 列表", &v["bots"], BOT),
        "/bot" if matches!(action, "add" | "add-status") => onboarding(v),
        "/bot" => {
            let mut out = fields(
                "Bot 状态",
                if v.get("bot").is_some() { &v["bot"] } else { v },
                BOT,
            );
            if let Some(agents) = v.get("agents") {
                out.push_str(&format!(
                    "\n\n{}",
                    table(
                        "所属 Agent",
                        agents,
                        &[("id", "Agent"), ("enabled", "启用")]
                    )
                ));
            }
            if let Some(notice) = v.get("notice") {
                out.push_str(&format!("\n\n{}", cell(notice)));
            }
            out
        }
        "/profile" if action == "list" => table("可用 Profile", &v["profiles"], &[("", "名称")]),
        "/profile" => {
            let mut out = fields(
                "Agent Profile",
                v,
                &[
                    ("agent_id", "Agent"),
                    ("profile", "Profile"),
                    ("model_override", "显式模型"),
                    ("effort_override", "显式推理强度"),
                ],
            );
            if v.get("revision").is_some() {
                out.push_str(
                    "\n\n配置已保存，将在下一次引擎切换时生效；保留显式模型和推理强度设置。",
                );
            }
            out
        }
        "/model" if action == "list" => table(
            "可用模型",
            &v["data"],
            &[
                ("id", "ID"),
                ("displayName", "名称"),
                ("description", "说明"),
                ("defaultReasoningEffort", "默认强度"),
                ("supportedReasoningEfforts", "可选强度"),
            ],
        ),
        "/model" if action.is_empty() => format!(
            "**当前模型配置**\n\n- 模型：{}\n- 推理强度：{}\n\n/model list 查看可用模型",
            inherited_setting(&v["model"]), inherited_setting(&v["effort"])
        ),
        "/model" => fields(
            if v.get("config").is_some() {
                "模型设置已更新"
            } else {
                "模型设置"
            },
            if v.get("config").is_some() {
                &v["config"]
            } else {
                v
            },
            &[("model", "模型"), ("effort", "推理强度")],
        ),
        "/agent" if action == "list" => table("Agent 列表", v, AGENT),
        "/agent" if action.is_empty() || action == "show" => format!(
            "**当前 Agent**\n\n- Agent：{}\n- Bot：{}\n- 来源模板：{}@{}\n- Profile：{}\n- 模型配置：{}\n- 推理强度：{}\n- 工作目录：{}\n\n/agent list 查看当前 Bot 的 Agents\n/agent use ID 切换 Agent",
            display(v, "id"), display(v, "bot_id"), display(v, "/config/id"), display(v, "/config/version"),
            v["config"]["profile"].as_str().map(esc).unwrap_or_else(|| "default".into()),
            inherited_setting(&v["config"]["model"]), inherited_setting(&v["config"]["effort"]), display(v, "workspace")
        ),
        "/agent" if action == "use" => fields("已切换 Agent", v, &[("agent", "Agent")]),
        "/agent" if matches!(action, "create" | "delete" | "enable" | "disable") => fields(
            if action == "create" {
                "Agent 已创建"
            } else if action == "enable" {
                "Agent 已启用"
            } else {
                "Agent 已停用"
            },
            v,
            AGENT,
        ),
        "/agent" => fields("Agent 用法", v, &[("usage", "用法")]),
        "/template" if action.is_empty() || action == "list" => table("模板列表", v, TEMPLATE),
        "/template" if action == "help" => {
            let mut out = String::from("**模板命令**\n");
            if let Some(commands) = v["commands"].as_array() {
                for command in commands {
                    out.push_str(&format!("\n- {}", cell(command)));
                }
            }
            out
        }
        "/template" if action == "delete" => fields("模板已删除", v, &[("deleted", "模板版本")]),
        "/template" => fields(
            if action == "from-agent" {
                "模板已保存"
            } else {
                "模板配置"
            },
            v,
            TEMPLATE,
        ),
        "/status" => format!("{}\n\n{}",fields("Bot 运行概览",v,&[("bot_id","Bot"),("total","任务总数"),("counts","状态统计")]),table("活跃任务",&v["active"],&[("id","任务"),("state","状态"),("agent_id","Agent") ])),
        "/tasks" => table("任务列表", v, &[("id","任务"),("state","状态"),("agent_id","Agent"),("prompt","请求")]),
        "/task" => format!("{}\n\n**结果**\n\n{}\n\n{}\n\n{}", fields("任务详情",v,&[("id","任务"),("state","状态"),("agent_id","Agent")]),esc_limit(&plain(&v["result"]),usize::MAX),table("投递",&v["deliveries"],&[("state","状态"),("attempts","尝试次数")]),table("最近事件",&v["events"],&[("kind","类型"),("data","详情")])),
        "/ask" => fields("协作请求已提交", v, TASK),
        "/system" => fields(
            "系统提示词",
            if v.get("config").is_some() {
                &v["config"]
            } else {
                v
            },
            &[("system_prompt", "内容")],
        ),
        "/collaborations" => table(
            "协作会话",
            v,
            &[
                ("id", "会话"),
                ("agent_id", "Agent"),
                ("state", "状态"),
                ("last_used", "最近活动（北京时间）"),
            ],
        ),
        "/collaboration" => fields(
            "协作会话",
            v,
            &[
                ("id", "会话"),
                ("agent_id", "Agent"),
                ("state", "状态"),
                ("config_revision", "配置版本"),
            ],
        ),
        "/cancel" | "/retry" | "/compact" | "/sh" => fields("任务操作结果", v, TASK),
        "/cd" => format!(
            "**工作目录{}**\n\n{}",
            if v.is_object() { "已更新" } else { "" },
            if v.is_object() {
                cell(&v["workspace"])
            } else {
                cell(v)
            }
        ),
        "/clear" => "**会话已清空**\n\n下一条消息将开始新会话。".into(),
        "/skills" if v.is_array() => table("Agent Skills", v, &[("", "Skill")]),
        "/skills" if action == "show" => {
            let skill = &v["skills"][0];
            format!("**{}**\n\n启用：{}\n\n{}\n\n路径：{}", display(skill, "name"), display(skill, "enabled"), esc_limit(&plain(&skill["description"]), usize::MAX), esc_limit(&plain(&skill["path"]), usize::MAX))
        },
        "/skills" => format!(
            "**Agent**：{} · 模式：{}\n\n{}\n\n/skills show 名称 查看详情\n/skills available 查看全部可用；-r 导出原始结果",
            display(v, "agent_id"),
            display(v, "mode"),
            v["skills"].as_array().map(|rows| {
                if rows.is_empty() { return "暂无启用的 Skills。".into(); }
                rows.iter().map(|s| format!("- {}{}", display(s, "name"), if s["missing"] == true { "（缺失）" } else if s["enabled"] == true { "" } else { "（未启用）" })).collect::<Vec<_>>().join("\n")
            }).unwrap_or_default()
        ),
        "/inbox" | "/recv" => table(
            "协作消息",
            v,
            &[("task_id", "任务"), ("state", "状态"), ("text", "消息")],
        ),
        "/cron" if action.is_empty() || action == "list" => {
            let mut out = table("定时任务列表", v, JOB);
            if v.as_array().is_some_and(|rows| rows.is_empty()) {
                out.push_str("\n\n/cron add 计划 -- 指令 创建草稿；后续确认后生效。");
            }
            out
        },
        "/cron" if action == "history" => table(
            "定时任务历史",
            v,
            &[
                ("status", "触发"),
                ("execution_state", "执行"),
                ("delivery_state", "投递"),
                ("task_id", "任务"),
                ("scheduled_for", "计划时间（北京时间）"),
            ],
        ),
        "/cron" if action == "add" => format!(
            "{}\n\n请在下一条消息回复 确认 或 confirm 创建，回复 取消 或 cancel 放弃。",
            fields(
                "定时任务草稿（尚未创建任务）",
                &v["draft"],
                &[
                    ("id", "草稿"),
                    ("schedule", "计划"),
                    ("timezone", "时区"),
                    ("next_fire", "下次执行（北京时间）"),
                    ("expires_at", "过期时间（北京时间）"),
                    ("prompt", "指令"),
                    ("/agent/config/profile", "Profile"),
                    ("/agent/config/model", "模型"),
                    ("/agent/config/effort", "推理强度")
                ]
            )
        ),
        "/cron" if v.get("usage").is_some() => fields("定时任务用法", v, &[("usage", "用法")]),
        "/cron" => fields("定时任务", v, JOB),
        "confirm" | "确认" => fields("定时任务已创建", &v["job"], JOB),
        "cancel" | "取消" => fields("草稿已取消", v, &[("cancelled", "草稿")]),
        _ => fields("任务已提交", v, TASK),
    }
}
/// Format command results without knowing which channel will deliver them.
pub fn message(command: &str, value: &Value) -> crate::message::Message {
    use crate::message::{Message, Span};
    let (command_text, raw) = split_raw(command);
    if raw {
        return Message::Plain(render(command, value));
    }
    if let Some(view) = crate::application::output::view(command_text, value) {
        return view.message();
    }
    if command_text.split_whitespace().next() == Some("/help")
        && value.get("help_topic").is_none()
        && value.get("error").is_none()
    {
        let mut rows = Vec::new();
        if let Some(entries) = value["commands"].as_array() {
            for entry in entries {
                if !rows.is_empty() {
                    rows.push(vec![]);
                }
                rows.push(vec![Span {
                    text: entry["usage"].as_str().unwrap_or_default().into(),
                    bold: true,
                }]);
                rows.push(vec![Span {
                    text: entry["description"].as_str().unwrap_or_default().into(),
                    bold: false,
                }]);
            }
        }
        return Message::RichText(rows);
    }
    Message::command_text(&render(command, value))
}
pub fn tool(name: &str, args: &Value, v: &Value) -> Value {
    if args["raw"] == true {
        return v.clone();
    }
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        return Value::String(error(name, &plain(e)));
    }
    let command = match name {
        "media_extract" | "media_inspect" => "/media".into(),
        "agent_profile" => format!(
            "/profile {}",
            if args["action"] == "list" { "list" } else { "" }
        ),
        "bot_command" => format!("/bot {}", args["action"].as_str().unwrap_or("show")),
        "bot_add" => "/bot add".into(),
        "template_from_agent" => "/template from-agent".into(),
        "agent_list" => {
            return Value::String(table(
                "可协作 Agent",
                &v["agents"],
                &[("id", "Agent"), ("bot_id", "Bot"), ("description", "说明")],
            ))
        }
        "agent_query" => {
            return Value::String(fields(
                "协作请求已提交",
                v,
                &[
                    ("accepted", "已接受"),
                    ("request_id", "请求"),
                    ("collaboration_id", "协作会话"),
                    ("state", "状态"),
                ],
            ))
        }
        _ => return Value::String(error(name, "未识别的工具结果，请使用 raw=true 查看")),
    };
    Value::String(render(&command, v))
}

/// Shell output is literal code; choose a fence the output cannot terminate.
pub fn shell_output(text: &str) -> crate::message::Message {
    let (status, text) = text.split_once('\n').unwrap_or((text, ""));
    if text.trim().is_empty() {
        return crate::message::Message::Markdown(status.into());
    }
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == char::from(96) {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = char::from(96).to_string().repeat(3.max(longest + 1));
    let newline = if text.ends_with('\n') { "" } else { "\n" };
    crate::message::Message::Markdown(format!("{status}\n\n{fence}bash\n{text}{newline}{fence}"))
}
