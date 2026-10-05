# Rust 选型与 COW 对比

> 分类：阶段性比较。当前实现与剩余能力以 [当前架构](current-design.md) 和专题文档为准。

用户已确认 Rust 控制平面与 CLI，Codex 引擎、个人使用、最多 20 Worker。仍只设计，不创建 Cargo 工程。建议 Tokio 异步运行时、Serde 接口数据、SQLite 持久化；库与版本、执行器拓扑待定。外部 Codex/渠道 CLI 无须随控制面全部重写。

## COW 已有能力

本机旧项目 architecture.md:29 和 plan.md 的并发章节描述普通 Task/mailbox 共用每 Agent 串行槽。src/runtime/worker.py 已有 steering pump、取消和临时 cron 运行时。tests/test_cron_ephemeral_worker.py 验证新 clone 与独立中断、不复用原 thread。架构文档 Durable cron scheduling/Persistence and recovery 已有持久化确认、租约、fencing、Outbox 和原入口投递。src/channels/commands.py 已有跨渠道命令注册。因此不能把 steer、临时 cron、多渠道、进程隔离或消息持久化本身算作新功能。

## 新设计差异

| 方面 | 当前新方案 | 相对旧项目的预期收益 |
| --- | --- | --- |
| 调度粒度 | 同会话串行、不同协作会话可并行，共享20执行预算 | 普通协作不再必须占用目标 Agent 主会话的串行槽；额度与工作区冲突仍可导致排队 |
| 协作模型 | 只复制配置，collaboration_id 多轮续接自己的历史，TTL 分层 | 显式表达协作状态和主会话隔离；不表示旧项目必然存在上下文泄露 |
| Bot 与 Agent | 实例固定属于一个 Bot，跨 Bot 复用模板 | 执行身份、实例状态和可复用配置边界更容易解释；旧项目已有 bot-local 路由和身份校验 |
| 配置 | 模板版本、实例 config_revision、会话/Job 快照分开 | 清楚界定更新影响范围；旧项目已有快照，此处是整理和细化 |
| 命令与接入 | 统一 mac bot add lark/wechat，整理命令组 | 交互一致、减少历史别名；扫码能力本身旧项目已存在 |
| 模块 | domain/application、scheduler/sessions、collaboration/cron、channels/engines、storage/delivery、cli | 可围绕已确定模型缩小修改波及面，不先拆分布式服务 |
| 语言 | Rust 控制面与 CLI | 类型约束、控制面资源和发布形态可能改善，无性能基准，不能保证任务提速 |

旧 manager.py 约 9569 行，sqlite_store.py 约 46781 行，累积多阶段兼容路径；重设计是重新划分接口的机会，但文件行数不证明旧代码质量差，Rust 也不能自动避免巨型模块。

## 典型改善与代价

用户会话正在改代码，其他 Agent 请求分析：COW 常规 mailbox 路径受目标 Agent 串行槽限制；新设计在有预算和无资源冲突时启动独立协作会话，同时用户 steer 仍进入主会话。改善的是调度隔离，不是模型推理速度。

协作并发需要限额、公平调度、等待释放、工作区锁、回收竞争和多轮路由，会比串行复杂。真实收益依赖引擎支持多独立会话与安全工具执行。个人部署全通可简化通讯 ACL，但不自动共享历史或扩大操作权限。

Rust 增加开发和调试成本；当前 COW 是经过实际运行的实现，新项目只是设计，不能称新项目已经更可靠。Codex、工具进程和网络推理仍是主要潜在成本。应复用 COW 的可靠性契约与验收场景，不为重写丢失恢复、幂等、身份与投递保障。

建议验证：交互会话被协作阻塞的时间、20 并发下控制面 RSS/CPU/队列延迟、Worker/控制面崩溃恢复、上下文续接正确性。当前不执行基准、不迁移生产服务。
