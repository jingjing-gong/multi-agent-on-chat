# 当前架构与运行契约

核对日期：2026-10-05。本文描述当前代码中的实现，作为设计主入口；操作语法见[命令参考](commands.md)，专题和验证边界见[文档索引](README.md)。早期方案保留在[历史目录](history/README.md)，不作为现行契约。代码事实与部署、验收状态必须分开；未来方案不混入下列实现说明。

## 1. 范围与模块

这是面向个人使用的 Rust 模块化单体：一个本地服务管理多个 Bot、Agent 和任务；首个且当前唯一的执行引擎是 Codex app-server。主要控制入口为 mac-cli，mac 保留兼容入口。渠道支持 local、微信和飞书；渠道凭据与模型 provider 配置分开。

| 层 | 当前职责 | 代码入口 |
| --- | --- | --- |
| 领域对象 | Bot、Template、Agent、Session、Task、Job 与回复目标 | src/domain.rs |
| 应用用例 | Scope 校验、Agent 配置、命令、cron 草案、媒体与回复模式 | src/application/ |
| 存储 | SQLite 事务、JSON 快照、输入、任务、触发记录与投递队列 | src/store.rs、src/store/ |
| 运行编排 | 接入监听、命令执行、任务调度、引擎事件、回收 | src/service.rs |
| 引擎 | Codex app-server 请求、事件、工具与 profile 配置 | src/engine.rs、src/skills.rs |
| 呈现和投递 | 共用消息格式、持久化投递计划、渠道编码、流式卡片 | src/presentation.rs、src/message.rs、src/delivery.rs、src/reply_stream.rs、src/channels/ |
| 媒体 | 附件下载、资产与授权检查、本地解析任务 | src/media/、src/application/media.rs |

service.rs 仍承担较多编排职责；这不是已经完全拆分的独立组件系统。已有应用层和渠道适配边界见[命令架构](command-architecture.md)。

## 2. 对象、身份和配置

- Bot 保存渠道、账号、凭据引用、owner 与启用状态。平台接入和监听属于 Bot；Bot 身份不表示独立操作系统账户或独立 Codex 登录。
- Template 是创建 Agent 时使用的配置快照来源。字段为 id、version、description、system_prompt、profile、model、effort、skills、skills_mode；没有独立图片 API 路由字段。
- Agent 固定归属一个 Bot，保存配置、revision、workspace 与启用状态。创建时复制模板，修改模板不会自动更新已有 Agent。
- Session 保存工作目录和配置快照，区分 user、collaboration 和 cron 上下文。Task 保存一次工作请求、generation、结果、父请求关联及回复目标。
- Job 保存调度、时区、冻结的 Agent 配置和工作目录；Occurrence 表示一次应触发时间，关联 Task。任务执行状态与投递状态独立。

主用户会话在下一次引擎边界应用 Agent 配置 revision 的变化；协作/cron 保持自己的快照。profile 的外部文件或依赖仍可能变化，快照不等于冻结整个外部运行环境。

模板 from-agent 导出当前配置并覆盖同名同版本的登记模板；已有 Agent、Session 和 Job 快照不变。模板删除只删除登记版本，default@1 不能删除。

聊天入口执行 owner 与 Bot 范围校验；跨 Agent query 的来源由服务端绑定。该部署按个人使用的信任模型运行，不提供多租户操作系统隔离，也不把模型输出中的任意目标当成发送授权。

## 3. 接入与消息路由

Bot 接入支持微信扫码、飞书应用凭据或扫码流程。凭据留在数据目录；注册、owner 绑定和监听 ready 是不同状态。飞书 CLI 通过配置目录区分 Bot，长路径经 lark_runtime 的短路径入口适配。

入站事件经渠道解析、去重和命令/普通文本路由。命令持久化后由命令处理流程执行；普通用户输入提交到对应用户上下文，已有活动工作时作为持久化追加输入，在引擎支持的边界 steer/继续执行。/clear 创建新的用户上下文，不隐式取消旧任务。

Agent query 建立关联任务和隔离的协作上下文；可复用协作上下文不等于共享用户主会话。父请求与子响应通过服务端关联和持久化输入传递。等待依赖不持续占用执行槽。历史设计中的完整通用 Mailbox/任意 fan-out 状态机不应视为全部已实现。

## 4. 调度、取消和恢复

Agent 队列由 serve --workers 控制，范围 1–20。队列内同一会话及规范化工作目录串行；不同 Agent 共享工作目录时也受此限制。

/sh 使用独立的最多 4 个 shell 执行槽，与 Agent 队列并行；shell 队列内同工作目录串行。因此 shell 和 Agent 工具可能同时操作同一工作目录，不能宣称全局工作目录互斥。shell 的目录在提交时固定，当前限时 30 秒，每个输出流最多保存 16 KiB，并支持进程组清理和取消。

当前任务主要状态包含 queued、running、waiting_dependency、cancel_requested、succeeded、failed、cancelled、interrupted；其他状态是否可用以实际处理分支为准，不沿用早期候选状态表。generation 和事务条件拒绝过期执行器写回。取消请求与最终完成分开，终态写入再次检查当前代次和取消状态。

服务重启将未完成执行转入中断恢复流程，不自动重放可能已经造成外部副作用的请求。执行前失败和提交后结果未知有不同错误分类。投递失败只重试投递，不重新执行模型任务。

## 5. 定时任务

实现入口：src/application/cron.rs、src/store/cron.rs、src/schedule.rs。

聊天 /cron add 生成 15 分钟有效的草案，后续输入确认才创建 Job；重复确认复用创建结果。支持 at、every、cron 和显式 IANA 时区。应使用本项目 /help cron 的语法，不把其他机器人的定时 CLI 当成本项目命令。

每次触发创建独立 cron 会话，使用 Job 冻结配置和目录，不污染用户主会话。同一 Job 有 queued/running/waiting_dependency/cancel_requested/interrupted/needs_attention 的关联工作时，本轮记为 skipped_overlap；迟到超过 3600 秒记为 missed。Occurrence 对 Job 和计划时间去重，阻止重复触发。Agent/Bot 不可用时 Job 可进入 blocked，后续修复由管理操作处理。

暂停/恢复/删除 Job 与取消某次 Task 是不同操作。创建、触发、执行和回传分别持久化；实时启用与身份检查仍适用，冻结配置不授予额外权限。

## 6. 模型、工具与 Skills

Agent 的 profile、model、effort 传给 Codex 引擎；/model default 和 /model effort default 分别清除对应覆盖。/model list 根据有效 profile/provider 查询：自定义 provider 的 /models，或内置 OpenAI 的 Codex model/list（完整分页）。未返回的推理强度信息表示未知，不证明模型不支持。

当前没有直接调用 /images/generations 或 /images/edits 的专用执行器，没有 --api 模型选择选项；相关实验已回退。图片任务如通过 Skill 实现，仍由支持工具的对话引擎执行该 Skill，而非把图片模型当作 Codex 对话模型。

Skills 支持继承和允许列表，配置路径/依赖在引擎连接时解析。Agent worker 默认启用已安装的 unified-computer-use 插件；默认加载不扩大应用访问授权。工具执行返回结果，附件发布由媒体用例限定到当前任务目标，不接受任意替代收件人。

## 7. 回复与卡片

命令通过公共 presentation/message 层生成输出，-r 保持原始 JSON；投递计划及各部分检查点持久化。远端接收与本地确认之间存在未知窗口，依赖渠道幂等能力，不能宣称所有渠道严格恰好一次。

异步命令只在持久化记录具备后续回复目标时省略受理回执。/cancel 和强制停用 Agent 使用完成聚合记录避免重复回复；同步查询与错误立即回复。

飞书默认 card，微信和其他渠道默认 off，显式 Bot 配置优先。progress 发送完整过程消息；card 收集 commentary/final_answer 为独立过程卡片和答案卡片。/stream 可查看、设置或清除当前 Bot 覆盖项，无需重启。

卡片生成时持续更新，完成后关闭流式再进行独立折叠操作。过程卡片默认折叠，终态“— 已完成”等固定放在面板后的末尾；答案超过 300 个 Unicode 字符或 8 行时按安全段落拆分，避免摘要与展开正文重复。折叠失败保留展开正文，不补发重复普通消息；历史 closed 卡片不批量重写。协议、续开与验收边界见[流式回复](reply-streaming.md)。

## 8. 媒体与解析

附件通过持久化入站准备流程成为受 Bot/会话范围约束的资产。模型输入使用已准备资产；media_publish 校验当前任务状态、generation 和工作区文件路径，导入不可变快照并原子关联发布，重试复用发布记录。

已实现本地文本/PDF 解析作业、分页结果和限制；未配置 OCR/ASR 时不会自动选择外部服务。微信语音只使用平台已有 voice_item.text；缺少转写时不声称完成自动语音识别。详见[媒体实施](media-implementation.md)、[PDF 解析](pdf-extraction.md)和[后续阶段](media-next-stages.md)。

## 9. 持久化、运行和验证

运行数据位于独立数据目录，包括 SQLite、凭据、媒体、日志和回复模式配置；源码仓库不保存这些内容。代码已纳入本地 Git；是否存在远程及推送状态以 git remote 和远端核验为准，不在设计文档中假定。

变更执行逻辑需运行相应测试；文档整理仅需核对代码和链接，不为文档变更重启服务。服务更新在无活动任务时进行，启动后核对 heartbeat 和恢复状态。

自动化测试覆盖事务、取消、恢复、命令路由、媒体快照、模型目录及卡片协议等；这不等于真实平台端到端验收。外部平台限流、长时间续开、最新客户端视觉、音视频/OCR 和完整灾备演练的边界，应在对应专项文档逐项维护。


## 10. 代码与测试索引

后续修改优先核对这些对应关系，而不是沿用历史方案中的候选接口。

| 契约 | 实现 | 主要测试 |
| --- | --- | --- |
| 对象与任务事务 | [domain](../src/domain.rs)、[store](../src/store.rs) | [runtime](../tests/runtime.rs) |
| 模型目录与 profile | [engine](../src/engine.rs)、[model_catalog](../src/model_catalog.rs) | [model_catalog](../tests/model_catalog.rs)及 engine 内测试 |
| 命令范围与输出 | [help](../src/help.rs)、[application](../src/application/mod.rs)、[presentation](../src/presentation.rs) | [application_commands](../tests/application_commands.rs)、[command_presentation](../tests/command_presentation.rs) |
| 回复模式与卡片 | [stream](../src/application/stream.rs)、[reply_stream](../src/reply_stream.rs)、[lark_cards](../src/lark_cards.rs) | [reply_stream](../tests/reply_stream.rs)及模块内测试 |
| cron 草案与触发 | [application cron](../src/application/cron.rs)、[store cron](../src/store/cron.rs)、[schedule](../src/schedule.rs) | runtime 与应用命令测试 |
| 媒体快照与发布 | [media](../src/media/mod.rs)、[application media](../src/application/media.rs) | [media_publish](../tests/media_publish.rs)、[media_assets](../tests/media_assets.rs)、[media_extractions](../tests/media_extractions.rs) |

本次整理核对了以上代码入口和现存测试定义，没有重新执行真实平台验收或全套运行测试。
