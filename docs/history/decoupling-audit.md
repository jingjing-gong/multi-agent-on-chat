# 历史记录：早期解耦审查

归档日期：2026-10-05。以下保留归档前原文，仅调整相对链接。原文的“当前”“已确认”“待实现”属于当时语境，不能作为现行接口或部署保证。现行入口见[当前设计](../current-design.md)。

---

# 全面解耦审查与决策清单

状态：只读审查现有设计后形成的评审，不实现、不新增默认授权。结论：领域对象已基本解耦，尚未形成足以验证模块可替换性与独立演进的接口契约；不能宣称“已充分解耦”。Rust 不替代这些契约。

## 1. 已建立的合理边界

Bot / AgentTemplate / Agent 实例分开；用户、协作与 cron 会话分开；Task / Run 分开；配置版本与即时权限分开；任务成功与投递成功分开；平台适配与引擎适配分开。这些方向保留。模块化单体足够，不以拆进程/微服务数量衡量解耦。

## 2. 优先修正的缺口

### A. Codex 能力与调度生命周期仍隐式耦合（实现前必须关闭）

证据：architecture.md 第 4 节引擎接口仅列 start/resume/run/cancel/close；collaboration-sessions.md 第 3 节和 current-design.md 第 6 节要求 steer、配置切换、等待释放与多会话并发，但对应接口尚未定义。

风险：等待子请求时的父 Codex turn 若仍占着活跃执行器，把配额标记为已释放会突破20上限；不释放又可能20个父任务全部等待子任务。把 paused Session 当作可恢复事实也可能超出引擎能力。

修正建议：明确 EngineCapabilities、TurnStarted/InputAccepted/ConfigApplied/Checkpointed/TurnFinished 等事件；记录输入序号、配置 revision、turn ID、execution generation。只有确认安全暂停或结束一轮并持久化 continuation 后才归还执行许可。阻塞等待工具若不支持暂停，则改异步提交→结束当前轮→结果到达新轮续接，不伪装暂停成功。引擎具体接口核实属技术工作，无需用户自行选 RPC。用户只需决定运行中不能应用某项配置时接受何种可见降级。

### B. profile、技能与工作区仍可能突破配置快照

证据：template-fields.md:25–29 保存解析摘要，但工作区 AGENTS、技能文件、MCP 配置与 profile 本身仍可能变化；cron-design.md 固定 config_revision。

建议拆为通用 AgentSpec 和 CodexEngineSpec：profile 等专用字段只能由 Codex adapter 解释，核心调度不读取 profile 语义。ResolvedConfig 保存实际生效字段与可追溯依赖，不只记录 profile 名称。凭据保留受限引用。

需用户决定冻结范围：建议冻结 prompt/model/effort/解析后的执行设置；技能、项目指令等外部依赖记录版本/摘要，变化时可见而非静默声称完全复现。严格固定技能内容则增加快照分发和存储。不能把“配置固定”宣传为文件系统与所有外部服务都固定。

### C. Bot 平台身份、模型登录身份与系统进程身份未完全拆开

证据：bot-agent-contract.md 第8节与 agent-communication.md 第1节强调所属 Bot 身份，模板又接受 Codex profile；尚未明确模型账号和 /sh 的 OS 身份。

建议分为 Principal（请求者）、ChannelIdentity（Bot 平台账号）、EngineCredential（Codex 登录/计费身份）、ExecutionIdentity（OS/沙箱）和 AgentId。通过运行时绑定引用，模型不可改写。Bot 身份只决定平台调用与来源归属，不表示每 Bot 必须独立登录 Codex，也不表示 shell 在平台身份下运行。

需用户决定：个人部署建议所有 Agent 共用本人 Codex 登录，Bot 分别持有平台凭据；是否有需要独立模型账号/费用归属的 Bot。

### D. 个人“全通”与入口控制权、数据可见性尚未闭合

证据：current-design.md 第4/6节、template-fields.md 第5节；群聊/话题粒度未选择。

全通策略应只确定哪些 Agent 可发现和调用；群成员能不能创建任务、steer、取消、查询则是入口策略。即便所有内部 Agent 可互通，工具动作仍需要由运行时核验资源和来源。不是用户显式授予的跨 Bot 私聊共享不能从全通自动推出。

建议个人版只允许本人账号发起/控制任务，群聊若支持则仅本人发给 Bot 的消息触发；私聊按 Bot+Agent+本人，群聊按 Bot+Agent+群+话题隔离。同部署本人管理的 Agent 可全通，跨会话资料按任务显式提供，默认不读别的完整私聊历史。该组需用户拍板。

### E. 20 Worker、/sh 与文件写入的资源模型未统一

证据：template-fields.md 第5节、commands.md:13、collaboration-sessions.md 第3节。/sh 若独立绕过执行预算，会破坏20并发语义；同 workspace 的协作仅复制上下文并不能隔离写入。

建议区分 ExecutionPermit、Session、Worker process、Engine connection。20定义为用户/协作/cron活动执行总额；/sh 使用同一执行预算（嵌套工具复用父执行许可），网络接收/投递不占该预算。等待归还许可须满足A项。共享项目读可并行，写操作保守串行或用独立 worktree；不做首版自动合并。用户决定 worktree与串行写的偏好，预算映射可由设计细化。

### F. postpend、/clear、结果引用与配置命令的作用域需统一

证据：commands.md 的 /clear、/system、/cd、/model；current-design.md 第5节与 template-fields.md 第4节。

建议有明确 ResolvedMessageTarget：Bot、Agent、Session、Task/Turn、reply target。引用协作或cron结果时仅该引用消息续接相应会话，不永久改掉默认接待 Agent；无引用消息进入当前用户会话。/clear 生成新 Session 并更新路由，旧执行结果带任务来源标签回原入口。共享Agent实例的 /system 修改究竟影响一个主会话还是所有用户主会话，需明确：建议修改实例配置，新主会话用新配置，当前用户会话按已选策略更新，其余已有用户会话在下一轮显示版本切换。

配置更新写入config revision不等于引擎已应用。postpend追加可信配置事件，不伪造系统角色优先级；真实 model/effort/profile 参数通过adapter应用。旧协作和Job不变。上述目标路由与用户主会话影响范围需要用户选择。

### G. 存储模块拆表不等于事务边界解耦

证据：architecture.md 第4/8节列出storage统一写入所有者，cron-design.md 需要原子创建Occurrence+Task+next_fire_at。未明确跨模块如何共享事务。

建议 application 定义 UnitOfWork/事务端口，SQLite adapter 实现；Inbox+Task、终态+Outbox、Occurrence+Task+next_fire_at、会话回收+消息接收保持原子边界。不能让各Repository独立提交或把数据库事务抽象强制传进引擎/渠道。核心域不依赖SQL、Tokio、Codex JSON、平台消息类；composition root组装具体adapter。属于技术设计，不需用户选trait名称。

### H. 文档存在多个版本的当前决定

证据：decisions.md 顶部已有Rust/Codex/个人使用确认，但下方旧D03/D04仍列为待确认；bot-agent-contract.md/current-design.md一些段落仍保留上下文与profile更新旧候选，README先后入口较多。

建议 decision ledger 每项只保留一个当前状态，明确 accepted/proposed/superseded；历史方案移到独立附录。当前汇总和专项契约双向链接，避免“上文新决定覆盖下文旧文本”长期存在。本次不自动替用户批准旧候选，后续接口定稿前统一整理。

### I. 内部通讯与可见协作展示应继续分开

Mailbox 的请求/回信是权威工作协议，群聊中的进度文本只是展示投影。展示消息不能重新被另一 Bot 摄取为有授权的协作请求，否则可能形成循环。建议默认原入口 Bot 展示协作摘要，目标 Bot 不自动在另一聊天主动发言；是否以后支持共同群聊双 Bot 展示，可单独选择，不改变内部通讯协议。

## 3. 建议依赖方向

```text
CLI / Chat Command Adapter / Scheduler trigger
                    ↓
             Application use cases
                    ↓
              Domain + Ports
                    ↑
Codex / Channel / SQLite / Credential / Process adapters

Composition root：组装各adapter与策略
```

domain 不导入渠道或Codex库；session manager通过EnginePort管理会话；scheduler只处理Task/Session状态及预算，不解析平台消息；BotContext通过ContextPort按需提供；Delivery只接收语义输出和不可变目标；外部发送格式化由channel adapter完成。

可验证解耦场景：假引擎测试队列与状态机；假渠道测试原入口回复；命令和cron产生同一种Task；fake credential验证Bot无法选他人身份；测试SQLite事务边界与重复入站；profile变更不影响已固定版本；/sh和协作共用资源约束。均为后续验收计划，此轮不编写测试或代码。

## 4. 用户决策建议清单

| ID | 需决策 | 建议默认 |
| --- | --- | --- |
| U1 | 群聊及入口控制权 | 仅本人可发起/steer/取消；群聊按话题隔离，私聊不混入群上下文；可选择首版只做私聊 |
| U2 | 全通和数据范围 | 本人同部署Bot/Agent默认互通；不给任意外部聊天用户管理权，不自动共享全部私聊历史 |
| U3 | Codex登录身份 | 共用本人Codex登录，各Bot独立平台凭据 |
| U4 | 同项目并发写与 /sh | 先保守串行写；需要并行修改时独立worktree，不自动合并；/sh计入20执行预算 |
| U5 | 更新与回复路由 | 真正运行中改不了的参数标为pending，在下一可用边界应用，不强制取消重开；引用结果只续接对应会话，不改默认路由 |
| U6 | 配置冻结的外部依赖 | 冻结解析配置、记录外部依赖变化；需要严格可复现的技能再固定版本，变更可见 |
| U7 | 部署/迁移范围 | 首版单机；需确认macOS、Linux或两者，以及是否导入旧COW数据。建议先不迁移 |

不再要求用户重复选择Rust、Codex、个人使用、20Worker、fork仅配置、多轮复用、cron三项规则、/sh、/notify和扫码接入。TTL/退避/队列容量、错误码与接口版本可由设计提供一组默认值集中审阅。
