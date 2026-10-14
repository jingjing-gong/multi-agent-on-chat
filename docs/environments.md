# 多版本运行操作

更新日期：2026-10-14。本页记录已实现接口；完整目标见[计划](version-environments-plan.md)。统一入口为安装后的 mac-cli 启动器，Rust 发布文件仍叫 mac-cli。下方带日期的部署与验证记录仅说明当时状态，不代表当前健康。开发和 GitHub 提交流程见[开发维护指南](development.md)，独立监督与恢复能力见[待实现方案](supervisor-plan.md)。

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

以下生命周期命令应从独立管理终端执行。若当前 Agent 运行在待变更实例里，不要在该任务中直接排空、重启或晋升自身：当前任务本身可能阻塞排空，停服后也无法继续执行恢复。先准备可用的外部执行入口，确认另一实例健康，一次只变更一个实例。

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

上述测试若从 Bot 任务运行，必须隔离继承的实例绑定；可直接使用[开发维护指南中的临时环境验证步骤](development.md#隔离验证)。源码仓库、Git 分支与已安装 release 是不同层次：提交或推送代码不会升级服务，重新构建也不会替换已安装的 release。

尚未实施：launchd 服务安装、外部依赖/Skills 不可变快照、配置导出工具、自动数据库迁移、跨机器身份租约和完整客户端端到端验收。这些不应被“版本隔离完成”一并视为已验证。

## 本机部署记录（2026-10-08）

统一入口安装到 ~/.local/bin/mac-cli，并在 /opt/homebrew/bin/mac-cli 提供链接。prod 沿用现有数据目录、20 个执行槽和现有 Codex 配置；dev 使用独立数据与 CODEX_HOME、4 个执行槽，仅创建 local Bot，当前以 mock 服务运行。两者通过同一固定 release 基线启动，可独立升级。

已保留迁移前二进制和 SQLite 在线备份，首次迁移在空闲窗口完成，启动 interrupted=0。生产飞书监听 ready 已核对，但没有自动发送真实测试消息。真实开发 Bot 接入、开发 Codex 凭据和 Skills 配置仍需按后续试用需求完成，不从生产复制身份。

若终端已经设置旧的 MAC_DATA_DIR，先 unset MAC_DATA_DIR，再通过 MAC_ENV 选择实例；启动器会拒绝冲突而不会猜测。开发用例验证后通过 service restart（不带 --mock）进入真实引擎模式；未配置独立模型凭据前保持 mock。

## 本机双版本部署（2026-10-10）

生产实例继续使用 `2026.10.08-5ef0abb648a4`，数据目录为 `~/.multi-agent-on-chat`，保留原有 14 个 Bot 和 20 个执行槽；本次没有重启生产服务。开发实例使用 `dev-2026.10.10-envsplit`，数据目录为 `~/.multi-agent-on-chat-dev`，4 个执行槽，以真实引擎运行，不带 `--mock`。

| 开发 Bot | 飞书应用 | 默认 Agent | Codex profile |
| --- | --- | --- | --- |
| `feishu-assistant-01`（Feishu Assistant 01） | `cli_aa1f9145d7b95bee` | `feishu-assistant-01-main` | `default` |
| `lisa`（Lisa） | `cli_aa147d4b4238dbcf` | `lisa-main` | `qwen` |

开发实例保留 `dev-local` 作为本地检查入口。两个飞书应用已从 Cowbot 停用，旧 profile 分别为 `lark-ca87239e` 和 `lark-f3b09547`；Cowbot 其余四个启用应用继续运行。不要在 Cowbot 或生产实例重新启用这两个应用，否则可能重复接收同一消息。本机 MAC 实例锁不能代替 Cowbot 或其他机器的应用停用操作。

开发 `CODEX_HOME` 为 `~/.multi-agent-on-chat-dev/codex-home`。只复制模型配置，不复制生产或 Cowbot 的会话、任务和历史消息。飞书凭据位于开发实例自己的 `credentials/<bot-id>`，沿用已验证的 Keychain 引用和 owner。旧 Cowbot 历史保留在原数据库，未导入或重放到开发实例。切换备份和记录位于 `~/.multi-agent-on-chat-dev/backups/split-20261010-165038`，它不是包含全部媒体、Keychain 和外部依赖的全量灾备。

日常操作：

    mac-cli status
    MAC_ENV=dev mac-cli status
    MAC_ENV=dev mac-cli bot list
    MAC_ENV=dev mac-cli service restart --timeout 60

开发版更新：为每次构建选择新的 release ID，再安装并晋升，只操作 `dev`。晋升保留真实/mock 模式，不需要额外重启。发布目录不可覆盖；生产发布应使用干净、经过验证的构建，不能直接晋升当前带 dirty 标记的开发产物。

    MAC_BUILD_RELEASE=dev-<unique-id> cargo build --locked
    MAC_ENV=dev mac-cli release install "$PWD/target/debug/mac-cli"
    MAC_ENV=dev mac-cli release promote dev-<unique-id> --timeout 60

从 Bot 内部自动化启动服务时，启动器清除父任务的飞书身份、Cowbot 和任务标识环境变量，使用实例自己的配置，并为服务子进程设置 `umask 077`。SQLite 检查使用禁止写入的连接，允许 SQLite 管理 WAL 辅助文件，但不会创建缺失数据库或修改业务表。相关回归检查包含在 `scripts/test-environments.py`。

`~/.local/share/mac/bin/mac-cli` 和 `/opt/homebrew/bin/mac-cli` 都链接到 `~/.local/bin/mac-cli`，避免 Bot 的 PATH 与终端各自运行不同的启动器副本。内部任务的 `$MAC_CLI` 仍指向其不可变 Rust 发布文件，不随晋升改变。

验证结果：两个开发飞书监听均为 `ready`，两个 Agent 的本地模型测试均返回 `MAC_DEV_READY`，未向飞书发送测试消息。Feishu Assistant 01 首次初始化探针曾报 app-server 断开，随后新的本地任务成功，失败记录保留，未重置会话或重放外部任务；这不是“永不失败”的保证。环境集成测试与 127 项 Rust 测试通过，Rust 测试需清除父 Bot 的 `MAC_BOUND_DATA_DIR` 等实例绑定变量后运行。飞书客户端收发仍需要人工验收。

两实例仍共享本机安装的 Codex/lark-cli 和 Keychain；本次未增加外部依赖版本快照，也未安装 launchd 开机启动或自动拉起服务。更新这些外部依赖前仍需排空并验证兼容性，不能把应用发布隔离视为所有依赖都已隔离。

## 源码提交前验证（2026-10-14）

清理测试子进程的父任务绑定并指定临时安装根目录和数据目录后，`cargo test --locked --all-targets` 的 127 项 Rust 测试、`cargo build --locked --bin mac-cli`、`python3 scripts/test-environments.py` 和 `cargo clippy --locked --all-targets -- -D warnings` 均通过。文档本地链接与 `git diff --check` 通过。

本轮整理开发与 GitHub 协作说明，并记录待实现的 supervisor 架构。验证只启动临时 mock 实例，没有安装或晋升正式 release、重启 dev/prod 或发送真实平台测试消息；不构成新的真实渠道健康或故障恢复验收。
