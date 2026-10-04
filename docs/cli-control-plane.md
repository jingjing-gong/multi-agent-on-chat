# mac-cli 控制与调试架构

2026-09-28：用户新增并要求先更新计划再实现。已授权实现，CLI 优先，主程序名 mac-cli，mac 保留兼容入口。

## 职责与依赖

CLI、聊天命令和调度触发共用 application/service 层，复用身份检查、状态转换、事务、幂等和配置解析；不为CLI另写一套业务规则。CLI支持结构化JSON输出、明确退出码和独立 --data-dir。

CLI允许服务运行时查询、提交输入、取消、配置管理；执行者只消费持久化任务，不在CLI命令中直接改造运行中的Codex线程。默认使用本地私有数据目录及OS用户边界，控制面单实例锁。平台发来的请求仍验证Bot owner，不因CLI可管理就放开外部用户。

当前单机实现可通过应用层短SQLite事务提交管理操作，后台服务轮询/唤醒读取，所有跨对象操作必须保持事务；CLI不持有长期执行租约。将来增加本地socket时替换控制transport，不改变业务用例。不得让SQLite结构成为用户需要直接修改的接口。

## 必备命令组

| 组 | 功能 |
| --- | --- |
| init / serve / doctor / status | 初始化、启动服务、依赖与存储检查、活动预算与队列状态 |
| bot | list/show/add lark/add wechat/add local/enable/disable/owner |
| template | validate/register/list/show/archive |
| agent | create/list/show/delete/config，配置支持system/model/effort/profile |
| chat / submit | 经过应用层提交命令或普通指导消息，可指定scope |
| task / tasks / cancel / retry | 任务查询与控制；重试不忽略未知副作用 |
| events | 按task读取事件，支持after游标和follow NDJSON |
| session | list/show/clear/close，清理上下文不删除审计 |
| query / collaborations / collaboration | 手工复现A query B、指定协作ID续问、查看请求与结果；明确来源任务 |
| cron | add草案/confirm/list/show/pause/resume/update/history/delete |
| deliveries | 查看投递状态、错误和重试次数，显式重试或标记已处理 |
| inspect | 只读检查允许的领域对象，不支持任意SQL或任意记录写入 |

模拟模式 mac-cli serve --mock 与 local Bot 只操作临时/指定数据目录，不调用真实平台和模型。测试fixture隔离创建，生产服务启动不会自动接管COW配置和数据库。

## 调试与安全边界

显示task/session/collaboration/bot/agent/config_revision和关联事件，避免只能从自然语言输出猜测状态；credential只显示引用，不打印令牌。输入持久化、引擎接受和应用分开标记。

CLI本地管理员可主动提交query与shell，但模型内部协作应使用绑定当前Task身份的引擎工具，不通过任意伪造CLI参数提升权限。/sh使用独立任务类型，不以可被普通文本伪造的前缀触发执行。工作目录固定到任务/会话，不能因稍后/cd改变旧执行。

## 实现顺序与验收

1. 先完成领域/存储/命令CLI及mock闭环。
2. 接入Codex app-server标准协议、动态query工具、steer和取消。
3. 接入扫码和消息适配，不自动执行真实登录/发送。
4. 用CLI复现多轮协作、消息去重、并发限制、cron冻结/跳过、重启恢复和原入口投递。
5. README列明可用命令、验证证据与未覆盖的生产验收，不把mock通过当作真实渠道验收。

## 首版代码映射与验证边界

- domain.rs 定义 Bot/Agent/Template/Session/Task/Job，store.rs 负责状态事务、查询关联与触发去重。
- engine.rs 只处理 Codex app-server 协议；不依赖 SQLite 或渠道发送。service.rs 编排 engine、store、渠道和调度，绑定 query 的源身份。
- channels.rs 管理扫码、入站文本解析、按 Bot 配置发送。main.rs 的 mac-cli/mac 作为本地管理适配器，输出 JSON。
- 用户指导、子响应和配置切换经持久化输入及引擎边界生效；单会话和共享工作目录串行。等待依赖不占 worker，取消与恢复把结果传回请求方。
- 入站命令先与 receipt 原子写入，执行中断保留 interrupted 状态；输出与命令完成原子提交。

这是单进程模块边界，不是分布式服务。仍需把部分 CLI 配置修改统一为更细应用用例；完整命令规划、严格 profile 依赖快照和真实平台验收见 README。
