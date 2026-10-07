# 多版本运行操作

2026-10-08。本页记录本轮实现接口；完整目标见[计划](version-environments-plan.md)。统一入口为安装后的 mac-cli 启动器，Rust 发布文件仍叫 mac-cli。

## 环境选择

    mac-cli status
    MAC_ENV=dev mac-cli status
    MAC_ENV=dev MAC_VERSION=<release-id> mac-cli version --json

未指定 MAC_ENV 时使用已登记 prod，未登记则报错。MAC_VERSION 不改变数据目录；与环境配置不一致的 MAC_DATA_DIR/--data-dir 被拒绝。不存在的版本不会回退。实例标记、数据库版本以及运行服务版本在写库前校验。

MAC_INSTALL_ROOT 默认 ~/.local/share/mac，含 releases、instances、locks、deployments。生产数据保留 ~/.multi-agent-on-chat，开发使用独立 ~/.multi-agent-on-chat-dev。开发 CODEX_HOME 可单独登记；新目录不自动复制生产凭据或历史会话，真实模型使用前需配置。

## 版本与实例管理

    mac-cli release install /absolute/path/to/built/mac-cli
    mac-cli release list
    mac-cli release verify <release-id>
    mac-cli env list
    mac-cli env show
    mac-cli env check
    mac-cli env register dev --release <id> --data-dir <path> --workers 4 --codex-home <path>

install 读取嵌入版本信息并计算 SHA-256，拒绝覆盖已存在版本；prod 拒绝 dirty 构建。release ID 默认干净 commit 前 12 位，可在构建时用 MAC_BUILD_RELEASE 显式命名。启动器是 scripts/mac-launcher.py 的安装副本，需要 Python 3.9+；可用绝对 Python shebang 固定解释器。

env check 当前检查配置绑定、发布哈希和服务状态；不宣称完整检查 Codex 登录、插件和真实渠道可用性。非空目录登记必须 --adopt-existing 且服务已停止；托管已有数据库目前只接受 schema 3，无隐式跨版本迁移。

## 启停和排空

    MAC_ENV=dev mac-cli service start --mock
    MAC_ENV=dev mac-cli service restart --mock --timeout 60
    mac-cli service drain
    mac-cli service readiness
    mac-cli service resume
    mac-cli stop

版本晋升保留当前服务的 mock 模式，避免试用环境意外发起真实请求。start/restart 使用独立日志和固定发布路径，启动后校验实际 daemon 身份。stop 仍是原始停止请求，可能中断未完成工作；日常发布应使用 restart/promote 的排空流程。当前为脱离终端的后台进程管理，不包含 launchd 自动重启或开机启动。

排空期间停止 cron 新触发，平台新输入得到维护提示，已接收任务、子查询、媒体、命令和投递继续完成。readiness 列出阻塞项；终态 failed 投递保留在数据库并报告数量，不自动重试。中断任务、待发送或未完成卡片仍会阻塞。超时保持原服务及 draining 状态，需要继续等待或 service resume。

同平台 channel/account_id 的监听使用本机跨实例 OS 锁；local Bot 不占平台身份锁。锁使用已登记、接入时验证的身份字段，不对第三方机器提供分布式锁。Agent 创建、目录修改和服务启动检查其他登记实例的 Agent/Session 工作目录重叠；不是操作系统沙箱，也不防外部程序任意写目录。

## 发布、回滚与恢复

    mac-cli release promote <release-id> --timeout 60
    mac-cli release rollback <release-id> --timeout 60

两者均为 schema 3 兼容版本间切换：排空、停服、备份 SQLite 和实例配置、原子选择发布版本、启动校验。不会自动恢复旧数据库或执行不可逆迁移。备份注明范围，媒体和凭据仍留原目录，因此不是全量灾备。

发布过程记录 deployments/<env>.json。未完成发布阻止再次直接 promote；检查日志、兼容性且服务停止后可显式选择：

    mac-cli release recover --use old
    mac-cli release recover --use new

recover 不恢复数据库、不重发历史消息，仅恢复兼容程序选择并启动。不要手工修改清单来绕过数据库兼容检查。

## 开发循环与验证

在开发 worktree 修改代码，运行相关 Rust 测试，构建到独立产物后 install，再将 dev 切换到该产物。生产晋升使用同一份已验证产物。内部任务固定 MAC_CLI 和 MAC_DATA_DIR，并携带实例身份，环境清单变化不重定向已运行的任务。

测试入口：cargo test --all-targets；cargo clippy --all-targets -- -D warnings；python3 scripts/test-environments.py（依赖已构建 target/debug/mac-cli）。集成测试只使用临时实例和 mock 服务，不接入正式 Bot。

尚未实施：launchd 服务安装、外部依赖/Skills 不可变快照、配置导出工具、自动数据库迁移、跨机器身份租约和完整客户端端到端验收。这些不应被“版本隔离完成”一并视为已验证。

## 本机部署记录（2026-10-08）

统一入口安装到 ~/.local/bin/mac-cli，并在 /opt/homebrew/bin/mac-cli 提供链接。prod 沿用现有数据目录、20 个执行槽和现有 Codex 配置；dev 使用独立数据与 CODEX_HOME、4 个执行槽，仅创建 local Bot，当前以 mock 服务运行。两者通过同一固定 release 基线启动，可独立升级。

已保留迁移前二进制和 SQLite 在线备份，首次迁移在空闲窗口完成，启动 interrupted=0。生产飞书监听 ready 已核对，但没有自动发送真实测试消息。真实开发 Bot 接入、开发 Codex 凭据和 Skills 配置仍需按后续试用需求完成，不从生产复制身份。

若终端已经设置旧的 MAC_DATA_DIR，先 unset MAC_DATA_DIR，再通过 MAC_ENV 选择实例；启动器会拒绝冲突而不会猜测。开发用例验证后通过 service restart（不带 --mock）进入真实引擎模式；未配置独立模型凭据前保持 mock。
