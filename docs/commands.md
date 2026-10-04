# 规划命令表

最新入口：主CLI名为 mac-cli，mac保留兼容；本文mac示例也适用于mac-cli。新增完整调试控制组见 [CLI控制平面](cli-control-plane.md)，用户已要求先更新计划再实施。

状态：已进入实现；完成情况见 [README](../README.md)。用户明确删除 /notify，其余按上一轮精简建议整理。此表取代 command-candidates.md 作为当前规划；新增的精确子命令拼写及参数为设计细化，不代表已有实现。此表为目标规划，不代表全部子命令已完成。

## 1. 会话与任务

| 命令 | 行为 |
| --- | --- |
| /help [command] | 按权限、渠道和引擎能力显示帮助 |
| /clear | 开始新用户会话上下文，保留历史任务记录；不隐式取消正在执行的旧会话 |
| /compact | 压缩当前会话，需引擎支持且处于可压缩状态 |
| /cd [path] | 查看/修改工作区；修改受权限约束，新配置不重定向在途任务、已有协作或 Job |
| /sh <command> | 在当前 Agent 的已授权工作目录直接执行有时限的 shell 命令；受 Bot/Agent 工具权限与运行环境约束 |
| /skills | 列出当前 Agent 可用技能 |
| $<skill> <task> | 指定技能的任务快捷输入，不是斜杠命令 |
| /status | 当前会话任务、执行状态和必要健康信息 |
| /tasks [limit] | 列出调用者有权查看的任务及历史 |
| /retry <task-id> | 明确重试符合条件的失败/失联任务；未知副作用先核实 |
| /cancel [task-id] | 取消指定或当前会话任务；当前目标不唯一时要求明确 ID |

## 2. Agent、模板与协作

| 命令 | 行为 |
| --- | --- |
| /agents | 列出本 Bot 实例与可协作对象，区别两者 |
| /agent | 查看当前接待 Agent |
| /agent use <id> | 切换到本 Bot 下的 Agent；不改变已运行任务的来源和回复目标 |
| /agent create <name> --template <id@version> [--workspace <ref>] | 从模板创建固定归属当前 Bot 的实例，按需补齐工作区，校验管理权限 |
| /agent delete <id> [--force] | 删除/停用实例；默认有活动任务或启用 Job 则拒绝；force 显式停用后续触发并请求取消活动任务，不盲删审计和文件 |
| /template list | 查看可用模板 |
| /template show <id@version> | 查看指定模板版本 |
| /ask <agent-id> <prompt> | 人工指定目标发起协作，新请求创建临时协作会话 |
| /collaborations [limit] | 查看有权限访问的协作会话与状态 |
| /collaboration show <id> | 查看协作详情和允许披露的结果 |

多轮续接通过引用协作结果或运行时的明确 collaboration_id 关联，Agent 工具接口直接携带该 ID；关联不清时不猜测。是否需要另加人工续接子命令留到交互契约细化，不新增手动 /fork。

## 3. 配置与通知

| 命令 | 行为 |
| --- | --- |
| /system | 查看当前 Agent 的角色/system prompt 配置 |
| /system <text> | 为有管理权限者发布新的实例配置版本 |
| /system default | 恢复实例创建时固定模板版本的角色默认值，并产生新配置版本 |
| /model | 查看模型和推理强度 |
| /model list | 列出当前引擎支持的模型和推理强度 |
| /model <id> [<effort|default>] | 设置模型及可选推理强度 |
| /model effort <effort|default> | 设置或清除推理强度覆盖 |
| /inbox [agent-id|all] | 查看有权访问的未读用户通知，不直接暴露内部 Agent 邮箱 |
| /recv | 仅微信渠道确有发送额度暂缓机制时提供 |

配置更新遵守 config_revision；已建立协作及 CronJob 固定配置版本，不被命令隐式改写。主用户会话的配置生效边界仍需在引擎契约中明确，不承诺热替换。/notify 已删除；必要结果、失败和状态反馈由统一通知策略处理，不以缺少开关为由关闭。

## 4. Cron

| 命令 | 行为 |
| --- | --- |
| /cron help | 定时任务帮助 |
| /cron add at <ISO datetime> [--tz <zone>] -- <prompt> | 提议一次性计划 |
| /cron add every <duration> [--tz <zone>] -- <prompt> | 提议固定间隔计划 |
| /cron add cron <m> <h> <dom> <mon> <dow> [--tz <zone>] -- <prompt> | 提议日历计划 |
| /cron list | 查看有权管理或查看的计划 |
| /cron show <job-id> | 查看规则、配置版本、目标、状态与下次触发 |
| /cron pause <job-id> | 停止后续触发，不自动取消当前执行 |
| /cron resume <job-id> | 恢复未来触发，默认不补暂停期间任务 |
| /cron update <job-id> <changes> | 提议修改时间/任务/配置快照，展示差异；changes 字段语法待细化 |
| /cron history <job-id> [limit] | 查看执行、跳过、失败和投递记录 |
| /cron delete <job-id> | 停用计划并保留历史，不隐式取消已经发起的任务 |

暂建议 add/update 显示草案后再按统一确认流程生效；精确确认交互尚待定。以上只定义产品规划，不调用当前环境实际 cron 工具。默认使用当前 Agent 配置生成 Job 快照，首版不引入旧 --profile 参数；是否需要 add 内联 --model/--effort 留待配置接口细化，当前表不将其列为计划语法。取消一次 cron 执行使用 /cancel <task-id>。

## 5. 管理 CLI（首版低频管理入口）

以下 `mac` 仅为命令行程序名占位，不代表已确定包名或已存在可执行文件；不发到聊天执行。

| 规划命令 | 功能 |
| --- | --- |
| mac bot list | 查看已配置 Bot |
| mac bot show <id> | 查看脱敏配置与健康信息 |
| mac bot add lark [--name <name>] | 扫码接入飞书/Lark Bot，校验后注册 |
| mac bot add wechat [--name <name>] | 扫码登录并接入微信渠道账号，校验后注册本地 Bot |
| mac bot add --config <file> | 注册 Bot，密钥用受限引用 |
| mac bot update <id> --config <file> | 更新 Bot 配置并校验影响 |
| mac bot disable <id> | 停止新入口与新任务，活动任务策略明确反馈 |
| mac bot enable <id> | 校验身份/配置后启用 |
| mac template validate <file> | 校验模板格式与引用 |
| mac template register <file> | 注册不可变版本，不运行模板内容 |
| mac template list | 查看可访问模板 |
| mac template show <id@version> | 查看精确版本 |
| mac template archive <id> | 禁止用该模板新建实例，保留既有引用 |
| mac agent engine-config show <agent-id> | 查看脱敏的引擎配置 |
| mac agent engine-config set <agent-id> --config <file> | 更新引擎专用配置并形成实例配置版本 |

CLI 的身份认证和管理员角色待总体身份方案确定，不能因为本地命令就跳过授权。Bot 删除和物理数据清理首版不另设快捷命令，先用 disable；恢复、部署与数据库维护等运维命令后续另定，不在本聊天控制面命令表承诺。

用户已确认两条平台扫码命令；`--name` 为建议参数，扫码、重复接入和启用状态规则见 [Bot 扫码接入](bot-onboarding.md)。

## 6. 删除或合并的入口

删除 /notify、/reset、/listskill、/listskills、/report。/delagent → /agent delete；/models → /model list。/profile 和 /lark add 移到引擎配置及 Bot 管理 CLI。用户明确保留 /sh，已列入首版聊天命令。

此前 /mode 是否与模板重复尚未定；本轮按精简目标暂不纳入 /mode 和 /modes，以模板/角色配置表达用途。如果用户需要“当前任务临时 plan/review”的确定性模式切换，再单独恢复设计，不把既有模式功能静默视为已经实现。

普通消息可作为运行中指导的方向保持不变，不要求 /steer；实际生效时机按引擎能力确定。未知命令和参数错误返回确定性错误，不透传给模型当作管理授权。
