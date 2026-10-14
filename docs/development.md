# 开发维护与 GitHub 协作

更新日期：2026-10-14。项目目录为 `~/projects/multi-agent-on-chat`，目标仓库为 [jingjing-gong/multi-agent-on-chat](https://github.com/jingjing-gong/multi-agent-on-chat)，SSH 地址为 `git@github.com:jingjing-gong/multi-agent-on-chat.git`。

## 开始工作

先读取适用的 `AGENTS.md`，检查 `git status --short --branch`、`git remote -v`、`git worktree list` 和近期提交。保留已有修改；并行开发时使用各自的分支/worktree，避免同时编辑同一文件。工具不会自动创建开发 worktree。

运行数据、数据库、Bot 凭据、模型认证、媒体和日志属于实例目录，不提交到 Git。首次推送还应检查已有提交历史，避免只检查当前工作区。GitHub remote 缺失时可配置：

```sh
git remote add origin git@github.com:jingjing-gong/multi-agent-on-chat.git
```

若已有 origin，先核对地址，不直接覆盖。推送前查询远端状态；远端有本地未知提交时，先获取并评审差异，不能用强制推送覆盖。

## 环境职责与隔离

| 项目 | prod | dev |
| --- | --- | --- |
| 用途 | 主要日常使用 | 开发验证、诊断和恢复入口 |
| 数据目录基线 | `~/.multi-agent-on-chat` | `~/.multi-agent-on-chat-dev` |
| CODEX_HOME 基线 | `~/.codex` | `~/.multi-agent-on-chat-dev/codex-home` |
| 程序选择 | 已登记的固定 release | 独立登记的固定 release |

实际值以 `~/.local/share/mac/instances/*.json` 和运行状态为准。数据、版本、任务与工作目录分离属于应用层约束，不是 macOS 用户或文件访问权限隔离。统一启动器、Codex/lark-cli、Keychain 和部分 Skills/插件仍可能共享；改动这些依赖需要评估两边的影响。

从独立终端操作时显式指定 `MAC_ENV=prod` 或 `MAC_ENV=dev`。Bot 内的 `$MAC_CLI` 是当前发布的 Rust 文件，`MAC_DATA_DIR`、`MAC_BOUND_DATA_DIR`、`MAC_INSTANCE_ID` 和 `MAC_VERSION` 已绑定当前实例；仅修改 `MAC_ENV` 不能安全地切换环境。跨环境管理应在独立进程中清理源任务绑定，再由统一启动器解析目标实例。不要手工绕过检查。

## 隔离验证

需要 Rust/Cargo 和 Python 3.9+。下面在子进程中隔离实例与平台变量，所有安装根目录和默认数据目录都指向临时位置；环境集成测试再创建自己的临时实例，仅运行 local Bot 和 mock 服务。不会对真实 dev/prod 执行生命周期命令。

在项目根目录执行：

```sh
python3 - <<'PY'
import os
import pathlib
import subprocess
import tempfile

env = dict(os.environ)
env['PATH'] = str(pathlib.Path.home() / '.cargo/bin') + os.pathsep + env.get('PATH', '')
for key in list(env):
    if key.startswith(('MAC_', 'LARKSUITE_', 'CODEX_WECHAT_')):
        env.pop(key)
with tempfile.TemporaryDirectory(prefix='mac-check-') as temp:
    env['MAC_INSTALL_ROOT'] = str(pathlib.Path(temp) / 'install')
    env['MAC_DATA_DIR'] = str(pathlib.Path(temp) / 'data')
    for command in [
        ['cargo', 'test', '--locked', '--all-targets'],
        ['cargo', 'build', '--locked', '--bin', 'mac-cli'],
        ['python3', 'scripts/test-environments.py'],
        ['cargo', 'clippy', '--locked', '--all-targets', '--', '-D', 'warnings'],
    ]:
        subprocess.run(command, env=env, check=True)
PY
```

环境集成测试覆盖发布不可覆盖、实例选择和冲突拒绝、工作目录重叠、mock 启停与排空、升级保留 mock 模式、任务凭据不继承、文件权限以及数据库检查禁止业务写入。Rust 测试与 mock 检查不等于真实飞书/微信/模型端到端验收。测试数量和结果应以本次输出为准。

## 提交与推送

1. 更新对应实现文档，区分已实现行为、历史部署记录与未来方案。检查 Markdown 链接和 `git diff --check`。
2. 审查工作区与暂存区，只添加本次相关文件；运行与变更匹配的验证。
3. 创建说明问题、行为和验证结果的提交，执行 `git push -u origin main`（功能分支替换为对应分支）。
4. 用 `git ls-remote origin refs/heads/main` 与 `git rev-parse HEAD` 核对远端提交，并确认剩余工作区状态。

commit/push 只发布源码，不代表安装、晋升或重启了任何实例，也不会把正在运行的 dirty 产物变成干净产物。

## 服务发布与可用性

以[环境操作](environments.md)中的 release install/promote/rollback/recover 为实际接口。先完成测试与提交，再从干净状态构建唯一 release；先在 dev 验证，生产晋升使用同一份已验证的干净产物。启动器是独立安装副本，更新 Rust release 不会自动更新启动器。

一次只变更一个实例。变更前确认另一实例可用，记录旧版本、数据库兼容性、备份位置与恢复命令；变更后检查实际版本、心跳、必要渠道和模型执行能力。不要仅凭 PID 判断健康。外部测试消息需要相应授权。

当前维护 Agent 位于 dev，不得在自己的任务中直接 stop、restart、drain 或 promote 自身。排空可能等待此任务结束，停服也会切断后续恢复步骤。更新 dev 需要独立管理终端或已经验证的外部监督流程接管。另一实例异常时暂停非必要发布，优先保住最后一个可用入口。

尚未实现的 supervisor 不能被当作现有保障；设计及验收条件见[监督与恢复方案](supervisor-plan.md)。程序回滚只切换兼容程序，SQLite 与实例配置备份不覆盖全部媒体、凭据或外部依赖。
