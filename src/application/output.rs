//! Typed views contain literal text, never Markdown that must be parsed again.
use crate::message::{Message, Span};
use serde_json::Value;
#[derive(Clone, Debug)]
pub enum Output {
    Detail {
        title: String,
        fields: Vec<(String, String)>,
        hints: Vec<String>,
    },
    Listing {
        title: String,
        items: Vec<String>,
        footer: Option<String>,
    },
    Help {
        title: String,
        description: String,
        entries: Vec<String>,
    },
}
fn text(value: &Value) -> String {
    match value {
        Value::Null => "默认 / 未设置".into(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}
impl Output {
    pub fn message(self) -> Message {
        let mut rows = Vec::new();
        let mut add = |s: String, bold| rows.push(vec![Span { text: s, bold }]);
        match self {
            Self::Detail {
                title,
                fields,
                hints,
            } => {
                add(title, true);
                add(String::new(), false);
                for (label, value) in fields {
                    add(format!("{label}：{value}"), false);
                }
                for hint in hints {
                    add(hint, false);
                }
            }
            Self::Listing {
                title,
                items,
                footer,
            } => {
                add(title, true);
                add(String::new(), false);
                if items.is_empty() {
                    add("暂无记录。".into(), false);
                }
                for item in items {
                    add(format!("- {item}"), false);
                }
                if let Some(footer) = footer {
                    add(footer, false);
                }
            }
            Self::Help {
                title,
                description,
                entries,
            } => {
                add(title, true);
                add(description, false);
                for entry in entries {
                    add(format!("- {entry}"), false);
                }
            }
        }
        Message::RichText(rows)
    }
}
pub fn view(command: &str, value: &Value) -> Option<Output> {
    if value.get("error").is_some() {
        return None;
    }
    if let Some(changes) = value["changes"].as_array() {
        return Some(Output::Detail {
            title: "配置已保存".into(),
            fields: changes
                .iter()
                .map(|v| {
                    (
                        text(&v["field"]),
                        format!("{} → {}", text(&v["before"]), text(&v["after"])),
                    )
                })
                .collect(),
            hints: vec![text(&value["notice"])],
        });
    }
    if let Some(topic) = value["help_topic"].as_str() {
        return Some(Output::Help {
            title: format!("{topic} 用法"),
            description: text(&value["description"]),
            entries: value["commands"].as_array()?.iter().map(text).collect(),
        });
    }
    let mut words = command.split_whitespace();
    let cmd = words.next()?;
    let action = words.next().unwrap_or("");
    let detail = match (cmd, action) {
        ("/agent", "" | "show") => Some((
            if action.is_empty() {
                "当前 Agent"
            } else {
                "Agent 详情"
            },
            vec![
                ("id", "Agent"),
                ("bot_id", "Bot"),
                ("/config/id", "来源模板"),
                ("/config/version", "版本"),
                ("/config/profile", "Profile"),
                ("/config/model", "模型配置"),
                ("/config/effort", "推理强度"),
                ("workspace", "工作目录"),
            ],
        )),
        ("/cron", "show") => Some((
            "定时任务详情",
            vec![
                ("id", "任务"),
                ("agent_id", "Agent"),
                ("state", "状态"),
                ("schedule", "计划"),
                ("timezone", "时区"),
                ("prompt", "指令"),
                ("config", "执行配置快照"),
                ("workspace", "工作目录"),
                ("delivery_destination", "投递目标"),
            ],
        )),
        ("/task", "show") => Some((
            "任务详情",
            vec![
                ("id", "任务"),
                ("state", "状态"),
                ("agent_id", "Agent"),
                ("prompt", "请求"),
                ("result", "结果"),
                ("deliveries", "投递"),
                ("events", "最近事件"),
            ],
        )),
        ("/template", "show") => Some((
            "模板详情",
            vec![
                ("id", "模板"),
                ("version", "版本"),
                ("description", "说明"),
                ("profile", "Profile"),
                ("model", "模型"),
                ("skills_mode", "Skills 模式"),
                ("skills", "Skills"),
            ],
        )),
        _ => None,
    };
    if let Some((title, fields)) = detail {
        return Some(Output::Detail {
            title: title.into(),
            fields: fields
                .into_iter()
                .map(|(key, label)| {
                    (
                        label.into(),
                        if matches!(key, "/config/model" | "/config/effort")
                            && value.pointer(key).is_none_or(Value::is_null)
                        {
                            "继承 Profile / Codex 默认".into()
                        } else {
                            text(if key.starts_with('/') {
                                value.pointer(key).unwrap_or(&Value::Null)
                            } else {
                                &value[key]
                            })
                        },
                    )
                })
                .collect(),
            hints: if cmd == "/agent" {
                vec!["/agent list 查看列表；/agent use ID 切换".into()]
            } else {
                vec![]
            },
        });
    }
    if let Some(items) = value.as_array() {
        let keys: &[&str] = match (cmd, action) {
            ("/agent", "list") => &["id", "bot_id", "enabled"],
            ("/template", "" | "list") => &["id", "version", "description"],
            ("/tasks", _) => &["id", "state", "agent_id", "prompt"],
            _ => return None,
        };
        return Some(Output::Listing {
            title: format!("{cmd}（共 {} 项）", items.len()),
            items: items
                .iter()
                .take(20)
                .map(|v| {
                    let mut line = keys
                        .iter()
                        .map(|k| text(&v[*k]).chars().take(100).collect::<String>())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    if v["current"] == true {
                        line.push_str(" · 当前");
                    }
                    line
                })
                .collect(),
            footer: Some(if items.len() > 20 {
                "--page 2 查看下一页；--limit N 设置每页数量".into()
            } else {
                match cmd {
                    "/agent" => "/agent show ID 查看详情",
                    "/tasks" => "/task show ID 查看详情",
                    _ => "/template show 名称@版本 查看详情",
                }
                .into()
            }),
        });
    }
    None
}
