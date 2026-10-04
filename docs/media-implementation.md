# 多媒体第一期实现与验收

2026-09-30。用户已确认实施，设计基准见 media-design.md。

## 已实现

- 2026-10-02：微信语音气泡读取平台提供的 `voice_item.text`，作为普通输入进入会话；已转写语音不依赖下载。没有转写时仅提示，不创建空任务；混合消息继续处理可用内容。重复投递去重，显式未完成消息忽略。音频自动转写及飞书语音转写尚未实现，真实语音端到端验收待完成。

- 统一媒体快照、SHA-256、Bot/会话隔离、引用、有效期、限额和 GC；SQLite v1→v2 事务迁移。
- 微信/飞书附件先持久化去重，再由 4 个独立下载 worker 准备，每 Bot 最多 2 个。原会话后来的普通文字排在附件之后，slash 控制命令可执行。
- 飞书平铺事件通过消息 GET 补取原始资源；图文位置以 [附件 n] 保留，图片以同一顺序输入模型。
- 图片真实解码、尺寸和内存预算检查；Codex localImage 输入；UTF-8 最多读取 16001 字节作有界预览。
- media_inspect / media_extract / media_publish 工具；发布事务内检查 generation，快照、caption、引用和 outbox 一起登记，相同内容重复调用复用交付。
- 上传和发送独立检查点；远端发送结果未知标记 unknown，停止自动重发；永久权限错误反馈文字。
- 飞书图片/文件经 stdin 上传；微信图片明确降级文件附件。
- 飞书 QR 经媒体队列，5 分钟过期，注册完成撤销有效期，禁止输入模型。
- /clear 取消未完成附件准备；重启恢复租约。GC 默认预览，--execute 才清理；先提交 expired 再删 blob，保留活动引用。
- 旧 File 路径在新建交付计划时快照化。

## 使用

    mac-cli media import ./image.png --bot om-assistant
    mac-cli media list --bot om-assistant
    mac-cli media show <asset-id>
    mac-cli media export <asset-id> --output ./copy.png
    mac-cli media send <asset-id> --bot om-assistant --chat <chat-id>
    mac-cli submit om-assistant-main '看看附件' --attach ./image.png --attach ./notes.txt
    mac-cli media retry <media-id>
    mac-cli media gc
    mac-cli media gc --execute

聊天：/media、/media show <asset-id>、/media retry <media-id>、/bot doctor。

## 验证

- cargo test --all-targets 与 cargo clippy --all-targets -- -D warnings 通过。
- 覆盖飞书平铺事件补取/下载/提交、图文顺序、去重、取消 fencing、隔离、解码、QR 拒绝、快照、发布幂等和 caption、GC 活动引用保护。
- Codex 协议测试直接检查 localImage，不代替真实视觉回答。
- CLI smoke：导入后修改源文件，导出保持原始内容；附件任务引用正确；GC 默认 dry-run。
- 真实上传：om-assistant、qz-assistant 的图片与普通文件均返回资源 key，2026-09-30。没有向其他人发送测试消息。

## 当前边界

- 真实用户图片下载并正确识图、微信端到端收图、原会话收到产物仍需真实消息验收。
- 微信原生图片未验收，当前走文件；PDF/OCR/音视频解析留待后续。
- 现有 Codex 线程动态工具取决于 resume 协议；/clear 可开始新线程。
- 上传/发送目前串行，低于设计并发上限；下载已实现全局 4、每 Bot 2。
- 沿用带类型 Message 与附件编号，独立 versioned ContentMessage 公共协议未提供；复杂解析缓存未启用。
- 媒体列表暂无分页和 pin/unpin。GC 保留活动/中断任务及可重试输入，历史终态引用按 7 天回收。
- CLI 下载每 25ms 检查落盘量并取消，可能有一个检查间隔的超额写入；最终限额严格校验。

## 运行部署状态

2026-09-30 13:45（Asia/Shanghai）已在无活动任务时切换新版，daemon PID 38133，20 workers。迁移前备份：/Users/autoclient/.multi-agent-on-chat/backups/before-media-v2-20260930-134548.sqlite3。数据库 v2，quick_check=ok；om-assistant、qz-assistant 监听 ready；启动 interrupted=0。真实微信收图/识图仍待用户消息验收。
