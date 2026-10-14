# multi-agent-on-chat

GitHub 仓库：[jingjing-gong/multi-agent-on-chat](https://github.com/jingjing-gong/multi-agent-on-chat)，SSH 地址为 `git@github.com:jingjing-gong/multi-agent-on-chat.git`。开发验证、提交和发布约定见[开发维护指南](docs/development.md)。

多版本入口仍为 mac-cli：MAC_ENV=dev mac-cli status 选择开发实例，MAC_VERSION 可选固定版本。安装与排空发布见[环境操作](docs/environments.md)。源码二进制需通过启动器登记后才具有托管环境选择行为。

设计入口：[文档索引](docs/README.md) · [当前架构](docs/current-design.md) · [现行命令参考](docs/commands.md)。历史方案集中在 docs/history；下文保留使用和操作说明。

dev 是开发与恢复入口，prod 承载主要日常使用；运行实例与 Git 提交独立。独立 supervisor / launchd 守护尚未实现，目标及验收条件见[监督与恢复方案](docs/supervisor-plan.md)。

渐进回复支持按 Bot 开关：微信完整进度消息、飞书 CardKit 流式卡片，共用任务输出与持久化投递状态。配置、降级与验收边界见 [渐进回复说明](docs/reply-streaming.md)。

`/sh` 使用独立的 4 个 shell 执行槽，不等待 Agent 任务或其工作区占用；同工作区的 shell 命令按提交顺序执行。Agent worker 数量仍由 `serve --workers` 控制。命令仍在提交时固定的工作目录执行，沿用任务记录、取消和结果回传。

`/model list` 每次按当前 Agent 的有效 profile/provider 动态查询：自定义 provider 请求其 `base_url` 下的 `/models`（沿用 provider 鉴权配置），内置 OpenAI provider 使用 Codex `model/list` 并读取全部分页。命名 profile 与兼容的 `<name>.config.toml` 均支持；不会把配置中的模型名作为查询结果。接口未提供的推理强度保持未知，查询失败直接报告错误。

Rust 个人多 Bot / Agent 运行时。主要入口 mac-cli，mac 为兼容名称。当前为可运行首版，Lark owner 绑定和回复已实测，完整模型对话验收仍在进行。

命令使用：`/help agent` 与 `/agent help` 查看同一份详细帮助；列表支持 `--page N --limit N`。`/agent show ID` 和 `/task show ID` 展开详情；`/status` 显示概览，`/tasks` 按最新优先列任务。`/cron history ID` 区分触发、执行和投递状态。Agent 可用 enable/disable，delete 保留停用兼容语义；已删除 cron 不能再 pause/resume。原生 CLI 新增 `mac-cli skills AGENT [list|available|show|enable|disable|mode] ...`。

实现边界及兼容性见 [命令架构](docs/command-architecture.md)。无参数命令与 `-r` 保留各自语义；`/status -r` 返回概览对象，原任务数组由 `/tasks -r` 提供。

## 构建与运行

多媒体第一期提供图片输入、文件资产管理与显式产物回传；命令、验证与当前边界见 [多媒体实现说明](docs/media-implementation.md)。

需要 Rust/Cargo；真实引擎需要已登录的 codex，Lark 接入需要 lark-cli。

    cargo build --release
    ./target/release/mac-cli --help
    ./target/release/mac-cli doctor

默认数据目录 ~/.multi-agent-on-chat，可用 --data-dir 或 MAC_DATA_DIR 指定独立环境。不会自动迁移或修改 codex-on-wechat 数据库及凭据。结果为 stdout JSON，诊断为 stderr，失败返回非零退出码。


完全本地运行示例（mock 不调用模型、不执行 shell、不连接平台）：

    export MAC_DATA_DIR=/tmp/mac-demo
    ./target/release/mac-cli init
    ./target/release/mac-cli bot add local --name demo
    ./target/release/mac-cli template register examples/researcher.yaml
    ./target/release/mac-cli agent create researcher --bot demo --template researcher@1 --workspace "$PWD"
    ./target/release/mac-cli submit researcher '研究这个项目'
    ./target/release/mac-cli serve --mock --workers 2

另一终端设置相同 MAC_DATA_DIR 后可使用：

    mac-cli submit researcher '优先检查消息路由'
    mac-cli tasks
    mac-cli status
    mac-cli events --follow
    mac-cli stop

真实执行使用 mac-cli serve --workers 20。Agent 执行槽上限 20；Agent 队列内相同会话和规范化工作目录串行，等待子查询释放 worker。/sh 使用另一条最多 4 槽的队列，只与同队列工作目录互斥。


## 默认开箱即用流程

init 自动注册 default@1 模板，profile/model/effort/额外 system prompt 均不覆盖本机 Codex 默认配置。bot add 自动创建 <bot-name>-main Agent，并使用数据目录下 workspaces/<bot-name> 的独立工作区。无需手动注册 researcher 模板或创建 Agent；下文的自定义模板命令是可选流程。

已有 Bot 没有任何 Agent 时，init 或服务启动会补齐默认 Agent；已有 Agent（包括停用的）和自定义配置保持原样。重复 init 不会重置配置。绑定 owner 后直接发送新消息即可对话。

## CLI 控制面

以各命令 --help 为准确参数说明。

| 命令组 | 操作 |
| --- | --- |
| init / doctor / status / serve / stop | 初始化、依赖检查、心跳状态、服务、停止 |
| bot | add local/lark/wechat、list、show、enable、disable、owner |
| template | validate、register、list、show、archive；YAML 不可变版本 |
| agent | create、list、show、delete（停用）、config |
| submit / chat | 用户指导；CLI 调用聊天命令 |
| tasks / task / cancel / retry | 查询、取消、失败重试 |
| query | source-task、target、key；collaboration 继续多轮 |
| collaborations / collaboration | 协作列表/详情 |
| session | list、show、clear、close |
| events | task/after 过滤、--follow NDJSON |
| deliveries | 投递检查、--retry 重试 |
| inspect | agent/bot/session/task/job/template/command/draft/control |
| cron | add 草案、confirm、cancel-draft、drafts、list、show、pause、resume、delete、history、update |

聊天实现：/help、/stream、/profile、/media、/clear、/compact、/cd、/sh、/skills、/status、/tasks、/cancel、/retry、/agent（查看/list [-g]/use/create/delete；list 默认当前 Bot，-g 列出全部）、/template（列表/list/show/from-agent/help）、/system、/model（查看/设置/effort/list）、/ask、/collaborations、/collaboration show、/inbox、/recv、/cron。普通消息指导当前任务，无需 /steer；无 /notify。

无参数的 `/agent` 显示当前 Agent、所属 Bot、来源模板和当前配置；`/bot`、`/profile`、`/model`、`/system`、`/cd` 显示各自当前配置。模型和推理强度未显式设置时标明继承来源，不推断实际运行值。`/cron` 等同 `/cron list`，只列当前 Bot 未删除的任务；空列表提示创建草稿的用法。`/skills`、`/template` 保留列表默认行为，执行类命令保留原有语义。

/sh 创建独立任务，真实执行限制 30 秒，每个输出流最多保存 16 KiB，支持取消、进程组清理和真实退出状态。普通文本无法通过前缀变成 shell 任务。

## 协作、配置与定时任务

Bot 固定拥有 Agent；模板只提供配置。A 查询 B 建立只继承 B 配置的独立会话，不复制 B 主历史；返回 request_id 和 collaboration_id。多轮复用 collaboration_id；无未完成任务且闲置 24 小时的临时会话到期。

Codex 注册 agent_list / agent_query 动态工具。源身份由服务绑定；B 使用所属 Bot 配置，结果作为 A 的输入。A 决定继续追问或向用户呈现。Lark worker 设置独立 Bot CLI 配置目录。

模板字段：id、version、description、system_prompt、profile、model、effort、skills_mode、skills。主会话 prompt 更新追加指导，恢复时更新 developerInstructions；model/profile/effort 改动中断当前 turn 后恢复线程，新 turn 使用新参数。协作和 Job 保存创建时配置与工作目录。

### 按 Agent 选择 Skills

Skills 由 Codex 从共享安装目录和 Agent 工作目录中发现。`/skills` 管理当前 Agent 的选择，不改写全局 Codex 配置。普通列表每行一个名称，未启用项标注状态；`/skills show 名称` 展开完整描述和路径。微信中超过 3000 字节的 `-r` 原始结果以 `command-output.json` 附件发送，内容原样保留，不解析 JSON；短结果仍在聊天中展示。

```text
/skills available
/skills mode allowlist
/skills enable code-review
/skills list
/skills show code-review
/skills disable code-review
/skills mode inherit
```

- `inherit` 默认沿用 Codex 原生发现和启用状态；旧配置缺少 `skills_mode` 时也按此处理。
- `allowlist` 只启用 `skills` 中列出的 Skill；空列表表示全部关闭，包括系统 Skills。首次从空配置切换后，逐个 enable 即可减少目录上下文。
- 在 inherit 下直接 enable/disable，会先保存当前已启用集合，再应用修改并切换到 allowlist。
- 名称必须唯一；同名时用 `/skills available -r` 中的完整路径，含空格的路径加引号。缺失或无法唯一解析的选择会报错，执行时不会自动退回全部启用。
- `list` 查看启用集合，`available` 查看全部发现结果，`show` 查看指定 Skill 的元数据。

选择随 Agent 持久化，并进入模板、协作与 Job 的配置快照；快照不冻结 SKILL.md 文件内容。修改主会话配置在引擎切换时生效，运行中的 turn 可能被中断后恢复，既有 Job/协作快照不变。旧历史里已读入的内容不会消失，需要全新上下文时用 `/clear`。

每个 Codex worker 使用独立 `skills.config` 覆盖并核对启用集合。共用工作目录的 Agent 也不会修改彼此的选择。禁用项的目录描述不会注入新的模型请求；这是上下文筛选，不是文件访问隔离。安装仍使用 Codex 原生 Skill 目录。

`python3 scripts/skills-context-smoke.py` 可用临时工作区和本地假模型服务验证上下文筛选，不调用真实模型。

cron 支持 at、every、五字段 cron，默认 Asia/Shanghai。创建先生成 15 分钟有效草案，后续独立 confirm；聊天回复 确认/confirm，取消为 取消/cancel。CLI 管理员明确调用 cron update 修改 Job。每次触发独立会话，同 Job 重叠跳过，超过一小时的遗漏触发跳过；Job+scheduled 去重。星期字段使用 0/7=周日、1=周一；cron 的日/月/星期组合使用 Rust cron crate 语义，不承诺完全兼容 POSIX crontab。

## 平台接入

    mac-cli bot add wechat --name personal-wechat
    mac-cli bot add lark --name personal-lark
    mac-cli bot owner personal-lark <your-open-id>
    mac-cli bot enable personal-lark

WeChat 原生二维码轮询；Lark 使用独立目录下 lark-cli config init --new。Lark 没有 owner 时不执行任务，但 serve 会监听首次用户私聊：自动绑定 owner、启用 Bot 并回复“owner 设置成功”；绑定消息不执行任务。已绑定后仅 owner 入站文本进入任务。手动 bot disable 会暂停自动绑定。凭据文件 0600、目录 0700；CLI 返回引用，不输出 token。

## 持久化与验证

SQLite WAL、短事务、generation fencing、入站去重、响应输入、投递 outbox。执行中停止标记 interrupted，不自动重跑未知副作用；子查询中断唤醒请求方。聊天命令先持久化再执行；执行期间崩溃标记 interrupted，检查后人工处理。

    cargo test --offline
    cargo clippy --offline --all-targets -- -D warnings
    cargo fmt --all -- --check
    cargo build --offline
    python3 scripts/smoke.py

当前 26 项自动测试通过，严格 Clippy 和格式检查通过；双 Bot CLI smoke 通过。测试覆盖去重/steer、配置副本与多轮复用、身份绑定、动态工具协议、并发、fencing、等待/返回、取消/恢复、cron、确认与 shell。CLI smoke 使用双本地 Bot 和 mock 验证 /ask 返回、续跑、steer、状态、停止。

## 尚未完成与限制

- 真实模型 turn、真实 Lark/WeChat 扫码和收发尚未端到端实测。动态工具对照本机 Codex 0.155.1 experimental app-server schema，需要保持版本兼容性验证。
- 目前文本渠道；附件、图片和平台长消息分段未实现。
- 聊天 /cron update、独立 engine-config 组待补齐，部分已有本地 CLI 替代。Skills 已支持按 Agent 选择，安装沿用 Codex 原生方式。
- Job/协作冻结 profile 名称；profile 文件和外部 MCP 内容仍在启动时读取，严格依赖快照未完成。命名 profile 优先从 CODEX_HOME/config.toml 的 [profiles.NAME] 读取，兼容 CODEX_HOME/<name>.config.toml。
- 无交互工具审批 UI；不支持的 app-server 请求明确报错。Bot 身份目录隔离不是 OS 权限沙箱；本版本面向单一可信个人账户。
- 自动 worktree、launchd 安装、数据库升级链、物理历史回收及跨机器部署未实现。interrupted 任务需核对外部结果。

架构：[CLI 控制面](docs/cli-control-plane.md)、[决定及历史](docs/decisions.md)、[目标命令表](docs/commands.md)。

本机真实协议探测：initialize 和含动态工具的 thread/start 被 Codex 接受；未执行 turn 的空线程无法 resume（无 rollout），因此真实上下文恢复仍待有实际 turn 的验收。

## 在 Agent 中管理 Bot 与保存模板

聊天命令：

    /bot
    /bot show
    /bot list
    /bot help
    /template from-agent my-template@1
    /template from-agent qz-assistant-main my-template@2
    /template delete my-template@1

delete 精确删除指定模板版本，已有 Agent 的配置快照保留。内置 default@1 用于新 Bot 的自动接入，不允许删除。命令及子命令的渠道回复统一使用通用富文本结构，尖括号和方括号按字面显示。

/bot enable [bot-id]、/bot disable [bot-id] 默认管理当前 Bot，也可指定其他 Bot。disable 当前 Bot 后无法再通过它收命令或回复，可从另一 Bot 或本地 CLI 恢复。

    /bot add lark --name assistant --app-id cli_xxx --app-secret 'secret'
    /bot add wechat --name wx-assistant
    /bot add-status assistant

新增异步执行并自动创建默认 Agent。Lark 可指定 --owner open_id、--brand feishu|lark；未指定 owner 时等待首次用户私聊绑定。WeChat 在原聊天发送扫码链接。Lark 扫码接入仍使用本地 CLI。App Secret 不回显、不写入命令数据库，但用户发出的聊天消息仍包含原始 Secret。

新 Agent 线程提供 bot_command、bot_add、agent_profile 和 template_from_agent 工具；已有线程可通过 MAC_CLI/MAC_DATA_DIR/MAC_BOT_ID/MAC_AGENT_ID 使用本地 chat CLI，无需清空上下文。

    /profile
    /profile list
    /profile fast
    /profile default

Profile 切换只修改当前 Agent，保留显式 model/effort 设置，不改写既有协作和定时任务快照。list 仅展示名称，不展示配置文件或密钥。

所有聊天命令和 mac-cli chat 默认输出 Markdown 关键信息；末尾加 -r 返回完整原始 JSON。Agent 工具对应 raw: true。本地其他 CLI 子命令保持 JSON。列表默认最多 20 项，长字段截断并提示 -r；帮助始终完整展示。带引号或转义的 '-r' 是普通参数，例如 /sh echo '-r'；自由文本末尾裸 -r 保留为输出开关。

逐项解析方案和测试边界见 [command-output.md](docs/command-output.md)。

CLI 保存模板：

    mac-cli template from-agent qz-assistant-main my-template --version 1

导出当前 Agent 配置（prompt/profile/model/effort/skills/description），不带对话、工作区、Bot 凭据或运行状态。from-agent 直接覆盖同名同版本的模板；省略版本时使用 @1。已有 Agent、会话和定时任务的配置快照不变。聊天和动态工具只能导出所属 Bot 的 Agent 配置；本地 CLI 可指定任意 Agent。跨 Bot list/show 仅返回公开身份与启用状态，不显示凭据。

Computer Use uses Codex’s installed unified-computer-use plugin (cua_repl.js)
inside an active Agent turn and inherits the host Codex plugin configuration.
mac-cli doctor reports computer_use.tools_loaded from MCP discovery without
calling a model or interacting with the desktop. A loaded tool does not establish
macOS accessibility or browser permissions; actual calls report those errors.
The diagnostic uses the default Codex profile; Agents with custom profiles may
have different tool availability. All Agents share the host desktop.

Computer Use application consent is explicit: MAC_DATA_DIR/computer-use-apps.json
contains a JSON array of approved macOS bundle IDs. An empty or absent list grants
no apps. The worker enables only MCP elicitations and accepts app-consent requests
from cua_repl / computer-use matching that list; other forms and audio consent
are declined. Shell permission escalation remains disabled. Changes take effect
on the next worker connection, including resumed threads. Elicitation decisions
are recorded as mac/elicitationResolved events. Tool discovery in doctor alone
does not verify this app-consent path.

Bot 的新建和恢复线程均显式使用 danger-full-access，取消 Codex 文件与网络沙箱。
普通执行无需审批；已配置的 Computer Use 应用授权仍通过独立 MCP 流程处理。
这相当于执行层的 yolo 模式，但不会授予 root、绕过 macOS 隐私权限或组织策略。
原会话在下一次任务恢复时应用。宿主机的 DNS、代理和 VPN 设置保持原样。
python3 scripts/sandbox-write-smoke.py 验证线程策略、工作区外临时文件写入及 TCP/HTTP。

聊天中 /bot add lark --name om-assistant 不提供凭据时进入扫码配置：
当前聊天会收到原始登录链接及 login-qr.png（飞书为图片、微信为 PNG 文件附件）。
后台等待扫描完成并校验应用身份；通过 /bot add-status om-assistant 查看结果。
提供凭据时必须同时提供 --app-id 和 --app-secret。新应用仍需绑定自己的 owner 后启用。

扫码中断或过期后，可重新执行同名 /bot add：若原目录只有二维码、日志和缓存，
新尝试使用独立凭据目录，保留旧二维码供发送队列使用；已有配置的目录不会被覆盖。

Lark 运行路径由 lark_runtime 适配层统一管理。凭据目录过长时，使用用户私有的
~/.mac-ipc/<目录摘要> 符号链接入口，保持原始凭据和应用身份不变。所有 Lark
运行调用共用此入口；启动前校验 macOS Unix socket 路径长度及别名目标一致性。
/bot show 显示消息监听状态：starting、ready、retrying；ready 来自 CLI 的明确就绪标记，
与 Bot 注册完成、owner 绑定是独立状态。首次私聊仍只绑定 owner 并回复确认。

所有 Agent 的 Codex worker 默认启用 unified-computer-use 插件，适用于新建和恢复会话，
覆盖 profile 对该插件的禁用设置。插件需已安装；应用授权继续使用统一的
computer-use-apps.json，默认加载不会自动批准其他应用或音频权限。

本地文本/PDF 解析的提交、状态、分页读取和限制见 [PDF 解析说明](docs/pdf-extraction.md)。
