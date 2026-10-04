//! User-facing command catalog shared by normal and raw help output.
use serde_json::{json, Value};
pub const HELP: &[(&str, &str)] = &[
    ("/help [命令名]", "查看帮助；命令 help 查看子帮助，末尾 -r 返回原始结果"),
    ("/stream [card | progress | off | default]", "查看或设置当前 Bot 回复模式；飞书默认 card，微信默认 off；default 清除覆盖设置"),
    ("/media [list | show | extract | status | result | cancel | retry | retry-extraction]", "查看附件、提交本地文本或 PDF 解析、查询状态和分页结果、取消或重试"),
    ("/clear", "清空当前会话，下一条消息开始新会话"),
    ("/compact", "压缩当前会话上下文"),
    (
        "/model [list | 模型 [强度] | effort 强度 | default]",
        "查看、列出或切换模型与推理强度；default 恢复默认值",
    ),
    (
        "/profile [list | 名称 | default]",
        "查看、列出或切换 Profile；default 恢复默认配置",
    ),
    (
        "/system [提示词 | default]",
        "查看、修改或恢复模板的系统提示词",
    ),
    ("/skills [list | available | show/enable/disable 名称 | mode inherit/allowlist]", "查看可用 Skills，按 Agent 启用、禁用或设置继承/白名单模式"),
    ("/cd [路径]", "查看或切换工作目录"),
    ("/sh <Shell 命令>", "在工作目录执行 Shell 命令"),
    ("/tasks · /status", "tasks 列任务，status 显示当前 Bot 运行概览；列表支持 --page/--limit"),
    ("/task show ID", "查看任务完整结果、失败事件与投递状态"),
    (
        "/cancel [任务ID]",
        "取消指定任务；省略 ID 时取消当前 Agent 的运行任务",
    ),
    ("/retry <任务ID>", "重试失败任务"),
    (
        "/bot [show | list | enable | disable | add | add-status | help] [参数]",
        "查看、新增或启停 Bot，可指定其他 Bot ID；接入参数见 /bot help",
    ),
    (
        "/agent [list [-g] | show/use/enable ID | disable/delete ID [--force] | create 名称 --template 名称@版本]",
        "无参数显示当前 Agent；list 列当前 Bot，-g 查看全部；支持切换、创建和停用",
    ),
    (
        "/template [list | show 名称@版本 | from-agent [Agent ID] 名称@版本 | delete 名称@版本 | help]",
        "列出、查看、删除指定模板版本，或将 Agent 配置保存为模板",
    ),
    ("/ask <Agent ID> <内容>", "向其他 Agent 发起协作请求"),
    ("/collaborations", "列出协作会话"),
    ("/collaboration show <会话ID>", "查看协作会话详情"),
    ("/inbox · /recv", "查看当前 Bot 的近期消息投递记录"),
    (
        "/cron [add 计划 [--tz 时区] -- 指令 | list | show/pause/resume/history/delete ID]",
        "无参数列出当前 Bot 的定时任务；支持 at、every、cron，创建草稿需后续确认",
    ),
    (
        "确认 / confirm · 取消 / cancel",
        "在后续消息中确认或取消上一份定时草稿",
    ),
];
pub fn catalog() -> Value {
    let entries = HELP;
    json!({"commands":entries.iter().map(|(usage,description)|json!({"usage":usage,"description":description})).collect::<Vec<_>>()})
}
pub fn detail(topic: &str) -> anyhow::Result<Value> {
    let command = format!("/{}", topic.trim_start_matches('/'));
    let (usage, description) = HELP
        .iter()
        .find(|(usage, _)| usage.split_whitespace().any(|part| part == command))
        .ok_or_else(|| anyhow::anyhow!("未找到该命令；/help 查看全部命令"))?;
    let examples: Vec<&str> = match command.as_str() {
        "/stream" => vec![
            "/stream — 查看当前模式",
            "/stream card — 飞书流式卡片",
            "/stream progress — 完整过程说明逐条发送",
            "/stream off — 仅最终结果",
            "/stream default — 恢复渠道默认值",
        ],
        "/bot" => vec![
            "/bot show [ID] — 查看当前或指定 Bot",
            "/bot list — Bot 列表",
            "/bot enable/disable [ID] — 启停",
            "/bot add lark --name ID --app-id APP_ID --app-secret SECRET — 接入飞书",
            "/bot add wechat --name ID — 扫码接入微信",
            "/bot add-status ID — 查询接入进度",
        ],
        "/model" => vec![
            "/model — 查看当前配置",
            "/model list — 可用模型",
            "/model 模型 [强度] — 切换模型",
            "/model effort 强度 — 修改推理强度",
            "/model default — 清除模型覆盖，保留当前强度",
            "/model effort default — 清除强度覆盖",
        ],
        "/template" => vec![
            "/template list — 列出模板",
            "/template show <name@version> — 查看配置",
            "/template from-agent [agent-id] <name@version> — 保存为模板",
            "/template delete <name@version> — 删除指定模板版本",
        ],
        "/agent" => vec![
            "/agent — 当前 Agent",
            "/agent list [-g] — 当前 Bot / 全部 Agents",
            "/agent show ID — 查看详情",
            "/agent use ID — 切换",
            "/agent enable ID — 启用",
            "/agent disable ID [--force] — 停用（delete 为兼容别名）",
            "/agent create 名称 --template 名称@版本 — 创建",
        ],
        "/cron" => vec![
            "/cron list — 当前 Bot 未删除任务",
            "/cron show ID — 配置和目标",
            "/cron history ID — 执行和投递结果",
            "/cron add every 1h -- 工作内容 — 创建草稿",
            "/cron pause ID — 暂停",
            "/cron resume ID — 恢复",
            "/cron delete ID — 删除",
        ],
        "/skills" => vec![
            "/skills available — 全部可用",
            "/skills show 名称 — 详情",
            "/skills mode inherit/allowlist — 选择模式",
            "/skills enable 名称 — 启用",
            "/skills disable 名称或路径 — 移除（允许失效路径）",
        ],
        _ => vec![usage],
    };
    Ok(
        json!({"help_topic":command,"usage":usage,"description":description,"commands":examples,"note":"列表支持 --page N --limit N；末尾 -r 返回原始结果。"}),
    )
}
