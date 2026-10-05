# 现行命令参考

核对日期：2026-10-05。依据 src/help.rs、src/service.rs、src/cli.rs；此页列当前聊天接口，具体选项用 /help 命令名、命令 help 或 mac-cli 子命令 --help 查询。[旧规划](history/commands.md)已归档。

## 入口与输出

聊天输入通过当前 Bot 路由到 Agent；mac-cli chat AGENT MESSAGE 使用同一命令用例。本地其他 CLI 命令主要输出 JSON，聊天默认使用共用格式化结果。末尾裸 -r 切换原始 JSON；引号中的 '-r' 不作为开关。支持的列表可用 --page N --limit N；不要假设每个命令都接受列表参数。

| 命令 | 当前用途 |
| --- | --- |
| /help [命令名] | 完整帮助或子帮助 |
| /stream [card / progress / off / default] | 查看或修改当前 Bot 回复模式 |
| /clear | 开启新用户上下文；不隐式取消旧任务 |
| /compact | 提交上下文压缩操作 |
| /model [list / 模型 [强度] / effort 强度 / default] | 模型目录及覆盖配置 |
| /profile [list / 名称 / default] | 查看、列出或选择 Codex profile |
| /system [提示词] | 查看或修改 Agent 系统提示词 |
| /cd [目录] | 查看或修改工作目录 |
| /sh 命令 | 独立 shell 任务，支持结果回传和取消 |
| /skills [list / available / show / mode / enable / disable] | 查看和修改 Skills 选择 |
| /status | 查看状态 |
| /tasks [--page N --limit N] | 当前 Bot 的任务列表 |
| /cancel [ID] | 取消指定任务；省略 ID 时取消当前 Agent 的运行任务 |
| /retry ID | 显式重试任务 |
| /bot [list / show / enable / disable / add / add-status] | Bot 查看、启停与接入，参数见子帮助 |
| /agent | 当前 Agent 信息 |
| /agent list [-g] | 当前 Bot 的 Agent；-g 查看全局公开信息 |
| /agent use / create / delete / enable / disable | Agent 路由与管理，参数见子帮助 |
| /template [list / show / from-agent / delete] | 登记模板与导出配置 |
| /ask Agent 内容 | 发起协作请求 |
| /collaborations | 列出协作上下文 |
| /collaboration show ID | 协作上下文详情 |
| /inbox、/recv | 当前 Bot 的近期消息投递记录 |
| /media [list / show / extract / status / result / cancel / retry / retry-extraction] | 资产、本地解析、分页结果与重试 |
| /cron [add / list / show / pause / resume / history / delete] | 定时草案、Job 管理与触发历史 |
| 确认 / confirm、取消 / cancel | 后续消息确认或取消定时草案 |

## 配置语义

- /stream：飞书默认 card，微信默认 off。default 清除当前 Bot 覆盖项；微信不可选择 card。配置文件写入采用锁和原子替换，无需服务重启。运行中的消息处理边界见[流式回复](reply-streaming.md)。
- /model list：自定义网关读取 /models，内置 OpenAI 使用 Codex model/list。列表缺少可选强度表示未提供信息。/model effort high 可以保存覆盖，但保存成功不证明网关支持该档位。/model default 清除模型覆盖，/model effort default 清除强度覆盖。无 --api 选项。
- /profile 与 model/effort 覆盖分别保存。主用户会话在引擎边界使用新的配置；已有协作和 cron 快照不会被整体替换。
- /skills mode 支持 inherit、allowlist；enable/disable 使用已解析的 Skill 选择，具体可用列表以运行环境为准。
- /template from-agent 导出配置而非对话/凭据；覆盖登记模板不修改已有实例快照。default@1 不可删除。

## 执行与回复

普通文本用于指导当前工作，无 /steer 或 /notify 命令。/sh 不进入模型，使用独立 4 槽队列；Agent worker 上限 20。同队列同工作区串行，两条队列可并行操作同一目录。

/sh、/retry、/ask、/compact、媒体解析/重试、/bot add 在具有持久化后续回复目标时不额外发送受理回执；同步结果、错误、缓存命中和授权二维码提示仍立即返回。/cancel 与强制停用聚合最终完成通知，避免命令回执与任务终态重复。

## 定时和身份范围

/cron add 使用 at、every、cron，可带 --tz，计划与工作提示词之间用 -- 分隔。它仅生成草案，后续输入确认创建；草案 15 分钟过期。暂停 Job、删除 Job 与取消某次运行是不同操作。聊天操作按当前 Bot/owner 范围检查，不以用户输入的任意身份代替来源。

命令结果与通知路由以服务端 ReplyTarget 为准；命令使用 -r 只改变呈现，不改变权限或执行行为。详细持久化与错误分类见[命令架构](command-architecture.md)和[输出契约](command-output.md)。
