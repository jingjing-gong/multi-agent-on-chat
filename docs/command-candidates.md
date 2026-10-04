# 聊天命令候选清单

此文件保留旧命令对照记录。用户已删除 /notify 并接受其余精简方向，当前清单以 [规划命令表](commands.md) 为准。

状态：供用户删选，全部新命令及去留建议待确认；不修改旧项目、不实现命令。来源为 codex-on-wechat/src/channels/commands.py 的 COMMAND_REGISTRY、LARK_COMMAND_REGISTRY_EXTENSION 及 _CRON_COMMAND_HELP，优先于可能滞后的 plan.md。

## 1. 旧项目实际注册命令

| 编号 | 命令 | 用途 | 新项目建议 |
| --- | --- | --- | --- |
| C01 | /help | 帮助 | 保留，按权限/渠道显示可用项 |
| C02 | /clear | 当前会话开始新上下文 | 保留，说明不删除任务审计、不默认取消运行任务 |
| C03 | /reset | /clear 的公开别名 | 删除重复入口 |
| C04 | /compact | 压缩当前会话上下文 | 保留，按引擎能力提供 |
| C05 | /cd [path] | 查看或切换工作目录 | 保留但绑定实例/会话配置版本；在途任务不跟随改变 |
| C06 | /sh <command> | 直接执行有时限的 shell | 用户后续明确保留，见当前规划命令表 |
| C07 | /skills | 列出启用的技能 | 保留 |
| C08 | $<skill> <task> | 指定技能执行任务 | 可选保留快捷语法；它不是斜杠命令 |
| C09 | /status | 当前任务状态 | 保留，突出当前会话 |
| C10 | /tasks [limit] | 列出自己的任务 | 保留，可查完成和历史任务 |
| C11 | /retry <task-id> | 重试失败或失联任务 | 保留，检查未知副作用，不一律重跑 |
| C12 | /cancel [task-id] | 取消指定或当前任务 | 保留，明确主会话/协作/cron 目标 |
| C13 | /report <message> | 写入持久化运维报告 | 首版可移除，不是生成工作报告 |
| C14 | /cron <add|list|delete|help> ... | 管理定时任务 | 保留命令组，新增操作见下节 |
| C15 | /agents | 列出 Agent | 保留，区分本 Bot 实例与可协作目标 |
| C16 | /agent [agent-id] [profile] | 查看、切换或创建 Agent | 建议拆出明确的 use/create，不再用不存在的 ID 隐式创建 |
| C17 | /delagent <agent-id> [force] | 删除动态 Agent，force 取消未完工作 | 合并到 /agent delete，检查计划和会话依赖 |
| C18 | /ask <agent-id> <prompt> | 发起带关联的 Agent 请求 | 可选调试/显式协作入口，Agent 内部调用不依赖用户输入此命令 |
| C19 | /system [default|<role>] | 查看、设置或清除角色提示 | 保留候选，落到 Agent 配置版本；不承诺运行中热替换 |
| C20 | /profile [default|<profile>] | 更改动态 Agent 的 Codex 配置 | 移出通用顶层命令，改为引擎专用配置；不等于新项目模板 |
| C21 | /mode [chat|plan|review|execute] | 查看/切换工作模式 | 待定；若模板和实例配置已覆盖可删除 |
| C22 | /modes | 列模式 | 若保留模式，合并到 /mode list |
| C23 | /model [model-id [effort|default]|effort <value|default>] | 模型与推理强度 | 保留，不能隐式改变已固定配置的协作/Job |
| C24 | /models | 列模型及推理强度 | 合并到 /model list |
| C25 | /notify [on|off] | 查看/设置通知开关 | 可选；必须区分进度与终态、cron 结果等通知语义 |
| C26 | /inbox [agent-id|all] | 查看未读通知 | 可选；与 Agent 内部 mailbox 是不同界面 |
| C27 | /recv | 获取微信发送额度限制暂缓的后续回复 | 仅微信适配器确有需要时保留，不暴露为全平台功能 |
| C28 | /lark add [profile] | owner 注册飞书/Lark Bot | 收拢为管理员 Bot 管理命令或 CLI，名称待定 |

隐藏兼容别名 /listskill、/listskills 都指向 /skills，新项目建议不引入。/recv 受渠道能力过滤；/lark add 是 Lark 扩展且受 owner 权限限制。其他命令的实际可执行性也受配置和权限检查约束。

## 2. 旧 cron 子命令的准确范围

旧注册帮助支持 add、list、delete、help；并未在该注册表列出 pause/resume/update/history。add 支持：

```text
/cron add at <ISO datetime> [选项] -- <prompt>
/cron add every <duration> [选项] -- <prompt>
/cron add cron <minute> <hour> <day-of-month> <month> <day-of-week> [选项] -- <prompt>
/cron list
/cron delete <job-id>
/cron help
```

旧选项包括 --tz、--profile、--model、--effort。新项目是否将这些配置通过 Job 编辑界面提供、是否保留所有内联选项待定。自然语言草案确认属于另一创建路径，不应据 slash 命令帮助绕过当前运行环境的确认协议。

## 3. 新项目新增或重组候选（尚不存在）

| 编号 | 候选命令 | 作用与建议 |
| --- | --- | --- |
| N01 | /agent use <id> | 切换当前会话接待 Agent，只能切本 Bot 实例 |
| N02 | /agent create <name> --template <id@version> | 从可见模板创建归属当前 Bot 的实例 |
| N03 | /agent delete <id> | 取代 /delagent，处理未完成任务、Job 与会话的依赖 |
| N04 | /template list、/template show <id@version> | 查看可用模板；模板编写/注册首版建议仍在管理 CLI |
| N05 | /cron show|pause|resume|update|history <job-id> | 单个计划详情、启停、修改及执行历史；各子命令参数另定 |
| N06 | /collaborations、/collaboration show <id> | 查看协作会话、来源、状态和结果；先做可见性，是否需要聊天命令待定 |
| N07 | /bot ... | 管理员查看/注册/配置 Bot；建议首版由管理 CLI 承担创建与凭据配置 |

不建议新增普通 /fork 或强制要求用户输入 /steer：协作临时会话由 Agent call 自动建立；用户普通消息的运行中指导通过路由与引擎契约实现。若需要新开独立用户任务，其显式入口语法另行确认。

命令负责确定性控制，模型不自行解析管理命令获得权限。删除命令入口与删除底层功能是不同决定；可将低频管理能力留在 CLI，同时精简聊天帮助。
