# 首次启动前检查（2026-09-28）

> 分类：历史运维记录及补记。本文机器状态、依赖和登录结论具有时间范围，操作前重新查询；当前架构与命令见 [文档索引](README.md)。

结论：本地依赖和文本适配层检查通过；尚未登录 Bot，不能声称已经完成平台端到端验收。

## 本机状态

- lark-cli 1.0.92，/opt/homebrew/bin/lark-cli；命令 flags 和 event schema 已本地核对。
- 默认 PATH 的 codex 为 /opt/homebrew/bin/codex，0.154.0。
- /Users/autoclient/.local/bin/codex 为 0.155.1，与项目此前动态工具协议验证版本一致。启动服务前建议将此目录放到 PATH 前端，再检查 codex --version。
- 默认 ~/.multi-agent-on-chat 不存在，未发现已初始化的新系统 Bot 配置；未读取旧 COW 凭据。
- ilinkai.weixin.qq.com 和 open.feishu.cn 的 DNS/TLS 可达，匿名 HEAD 返回 405/404。这仅确认网络可达，不验证业务 API、权限或授权。

## 此次修复

1. Lark 兼容 lark-cli 处理后的扁平消息事件以及原生嵌套事件。扁平 content 直接按文本处理，不误当 JSON。
2. Lark 登录使用 auth status --verify + whoami --as bot 联合验证，要求 bot ready/available/verified，应用身份和品牌一致；不再从任意嵌套字段找到 app_id 就视为验证成功。
3. Lark 消息如携带 app_id，需与注册 Bot 一致。拒绝 Bot 自身事件及非文本消息。
4. Lark 消费进程正常或异常退出均进入有延迟的重连路径，错误记录到 events，避免无提示快速重启。
5. WeChat X-WECHAT-UIN 按 COW 使用 32 位随机十进制字符串的 base64。
6. WeChat 回归测试覆盖数值 message_id、cursor、原始 context_token、过滤 Bot 消息。

## 首次交互启动

    cd /Users/autoclient/projects/multi-agent-on-chat
    export PATH="$HOME/.local/bin:$PATH"
    codex --version
    ./target/debug/mac-cli init
    ./target/debug/mac-cli bot add wechat --name personal-wechat
    ./target/debug/mac-cli bot add lark --name personal-lark

扫码属于交互操作。本次未调用这些 add 命令。WeChat owner 由登录返回；Lark 需要对应应用下自己的 open_id：

    ./target/debug/mac-cli bot owner personal-lark <your-open-id>
    ./target/debug/mac-cli bot enable personal-lark

每个 Bot 还需要至少一个启用的 Agent；只有 Bot 没有 Agent 时不会执行用户任务。template register + agent create 的完整步骤见 README。运行前检查：

    ./target/debug/mac-cli bot list
    ./target/debug/mac-cli agent list
    ./target/debug/mac-cli serve --workers 20

Lark 必须在平台侧具有机器人能力、可用的消息接收事件与发送权限，owner 在应用可用范围内。config init 是否已自动完成这些设置，需要首次订阅与收发确认。

## 首次登录后的验收

- owner 发 /status，确认消息能够进入；发简短任务并收到回复。
- 在任务运行中追加指导，核对 tasks、events 和结果。
- 查看 deliveries 确认发送成功，查看 events 中 channel:<bot-id> 的异常。
- 若进程断开，确认重连且消息不重复执行。
- WeChat 回复使用该次入站 context_token；Lark 始终 --as bot 并使用所属 Bot 的隔离目录。

当前仅文本链路。图片、附件和长消息分段未完成；平台真实授权、消息投递、模型执行与恢复仍需登录后实测。扫码过程本身可能更新 lark-cli 的外部配置，本次检查未触发。29 项本地测试及严格 Clippy 通过，CLI 双 Bot mock smoke 用于验证内部流转，不替代真实平台验收。

## 使用已有 App ID / App Secret

mac-cli bot add lark 已支持 --app-id、--app-secret-stdin、--brand（feishu/lark，默认 feishu）。Secret 由子进程从 stdin 读取，不放在参数中。以下命令在本地终端提示输入，输入不回显：

    python3 -c 'import getpass; print(getpass.getpass("Lark App Secret: "))' | ./target/debug/mac-cli bot add lark --name personal-lark --app-id cli_REPLACE_ME --app-secret-stdin

海外 Lark 增加 --brand lark。Bot 注册前验证真实应用身份；没有 owner 时保持禁用。随后执行 bot owner personal-lark <your-open-id> 和 bot enable personal-lark，再按 README 创建所属 Agent。已有 App 的机器人能力、事件订阅、发送权限及应用发布状态需在开发者后台配置；导入凭据不会自动完成这些设置。本入口经过本地测试与编译检查，未使用真实 Secret 调用平台。

### 参数输入与成功通知

现在也支持 --app-secret，和 --app-secret-stdin 互斥：

    ./target/debug/mac-cli bot add lark --name personal-lark --app-id cli_REPLACE_ME --app-secret 'REPLACE_WITH_SECRET' --owner ou_REPLACE_ME

App Secret 会出现在 mac-cli 的进程参数中，直接输入也可能保留在 shell 历史；mac-cli 将其通过管道传给 lark-cli，不再复制到子进程参数。不要把真实 Secret 提交到仓库或聊天记录。

有 --owner 时，Bot 注册成功后自动以 --as bot 向该用户发送“接入成功”通知，无需先启动 serve。输出的 notification.status 为 sent、failed 或 pending_owner。发送失败保留已注册 Bot；没有 owner 时不猜测收件人，返回 pending_owner。owner 必须是该应用下的用户 open_id，不是 Bot 的 open_id。

模拟接入测试：python3 scripts/onboarding-smoke.py。仅使用测试凭据和假 lark-cli，无外部请求。

### 已添加但 pending_owner 的 Bot

    mac-cli bot owner personal-lark ou_REPLACE_ME
    mac-cli bot notify personal-lark

notify 以 Bot 身份向配置的 owner 发送接入成功消息，无需启动服务或创建 Agent，也不会自动启用 Bot。未设置 owner 时明确报错；发送失败返回非零退出码并保留 Bot。要开始接收任务，再执行 bot enable 并按 README 启动服务。此本地 bot notify 是接入诊断命令，不恢复已删除的聊天 /notify 命令。


## 自动绑定 owner（最新行为）

已有 pending_owner 的 Lark Bot 无需重新添加。启动最新版本 mac-cli serve，然后在飞书私聊该 Bot 发一条文本。服务从第一条用户私聊提取 sender_id，原子保存 owner、启用 Bot，并将“owner 设置成功”回复加入持久化投递队列。绑定消息不会作为命令或 Agent 任务执行，也不要求预先创建 Agent。

群消息、Bot 消息、未知 chat_type 不触发绑定；已有 owner 不会被覆盖。重复消息仅产生一条绑定回复；并发绑定只有一个成功。bot disable 显式暂停首次绑定，bot enable 可恢复等待（仍不执行普通任务）。此最新流程取代上文“必须手动设置 owner”的限制。回复依赖 Lark 发送权限，失败可在 deliveries 查看及重试。

    ./target/debug/mac-cli serve --workers 20

服务运行期间，用飞书给目标 Bot 私聊发“你好”。绑定成功后创建 Agent，再发新消息开始任务。正在运行旧二进制的服务需 stop 后重新启动。自动测试涵盖绑定竞态与去重；scripts/owner-binding-smoke.py 使用假 lark-cli 跑服务监听到绑定回复闭环，无真实发送。


## 默认 Agent 自动创建（最新）

init 注册 default@1；bot add 原子保存 Bot 与默认 Agent，无需额外操作。已存在但无 Agent 的 Bot 在 init/serve 时补齐。qz-assistant 已补齐 qz-assistant-main 并重启新版服务。配置不指定 model/profile/effort/额外提示词，使用本地 Codex 默认值；工作区为 ~/.multi-agent-on-chat/workspaces/qz-assistant。之前“必须手动创建 Agent”的说明已被此默认流程取代。
