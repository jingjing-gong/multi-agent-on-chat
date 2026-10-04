# 本地文本与 PDF 解析（M1）

2026-09-30。实现独立持久化解析作业，使用现有资产、SQLite 和交付队列；没有接入外部 OCR/ASR。

## 使用

CLI（同一个 data-dir 的服务必须在运行）：

    mac-cli media import ./report.pdf --bot om-assistant
    mac-cli media extract <asset-id> --start-page 1 --end-page 10
    mac-cli media status <extraction-id>
    mac-cli media result <extraction-id> --offset 0 --limit 4000
    mac-cli media cancel <extraction-id>
    mac-cli media retry-extraction <extraction-id>

聊天：

    /media
    /media extract <asset-id> [起始页 结束页]
    /media status <extraction-id>
    /media result <extraction-id> [offset limit]
    /media cancel <extraction-id>
    /media retry-extraction <extraction-id>

动态工具复用 media_extract，action 为 request/status/result/cancel/retry。新线程具有更新后的 schema；旧线程可通过已存在的 shell 工具调用 mac-cli chat 当前 Agent 的上述命令，或明确新开线程。没有静默清空旧会话。

request 返回 queued 表示解析已排队，不代表读过文件。原会话会收到一次完成或失败回执；本地 CLI 默认无聊天通知。自动唤醒模型继续回答不在本次实现，用户可在完成后让 Agent 读取 result；模型不应在同一 turn 循环查询。

## 行为与限制

- 文本严格 UTF-8，有界缓存；PDF 使用本机 pdfinfo/pdftotext，结果按页返回。扫描页标记 needs_ocr，混合文档列出空文字页警告，损坏/加密 PDF 明确失败。
- 每请求最多 50 页；最长 60 秒；缓存文本最多 1 MiB；结果 JSON 最多 2 MiB；每次最多返回 16000 字符。页范围/预算截断明确提供 coverage/truncated。
- 子进程标准输入关闭，stdout/stderr 有界；取消/超时会终止子进程。每 100ms 观察 RSS，超过 512 MiB 终止，存在一次采样间隔的超额；这不是操作系统硬内存沙箱。
- 解析全局 2 worker、每 Bot 1；队列全局最多 128、每 Bot 最多 32。解析不占模型 worker。
- 缓存键含 Bot、会话、摘要、解析器版本指纹与选项。当前仅复用同一源 asset_id 的请求；相同文件的另一份导入不借用原授权。读取状态/结果仍检查源资产，auth_qr 一律禁止。
- 结果是受管资产。分页基于 Unicode 字符，并保留页码与片段偏移。文档内容属于不可信附件数据。
- 作业租约、generation、取消、/clear、重启恢复均落库。最终提交检查取消状态，过时结果不发回执。
- 本地解析错误不自动盲重试；依赖缺失/配额问题修复后可以显式 retry-extraction。新请求前缺失依赖直接返回 dependency_missing；执行期间缺失依赖进入 blocked。

## 一期可靠性收尾

- GC 保留活动解析源文件和有效源文件关联的结果。
- 保留期从首次观察到有效引用释放后开始计时，至少保留 7 天；新增引用通过 DB trigger 清除旧计时。GC 默认仍是预览，执行模式负责登记释放时间，因此实际回收可能晚于 7 天。
- /bot doctor 和解析结果使用对应格式化逻辑，避免显示无关空字段；CLI doctor 展示本机 PDF 后端存在性。
- SQLite v2→v3 事务迁移新增 media_extraction_jobs 和引用计时 trigger。升级前备份，旧版不能读取 v3 库。

## 验证与未完成项

覆盖真实 Poppler 提取、扫描/损坏 PDF、中文分页、缓存与会话隔离、源撤销、取消、clear、重启重领、迁移重入、GC 引用保护，以及子进程输出/时限。
CLI 独立服务 smoke 已验证 PDF 导入→队列→提取→页码结果→缓存复用。此测试使用本地 Bot，不替代飞书/微信真实 PDF 下载和模型问答。

OCR、Office、音视频、自动恢复模型回答、pin/unpin、分页资产列表、上传并发提升、微信原生图片仍未实现。当前改动只落实 M1 和直接相关的可靠性修复。

## 部署记录

2026-09-30 15:33（Asia/Shanghai）升级上线，PID 44063；数据库 v3、quick_check=ok，两个飞书监听 ready，interrupted=0。升级前备份 /Users/autoclient/.multi-agent-on-chat/backups/before-extraction-v3-20260930-153323.sqlite3。

101 项测试全部通过；Clippy --all-targets -- -D warnings 通过。线上 worker 的本地 PDF 导入/解析/分页读取验证通过，没有发送测试消息。真实用户渠道下载与 PDF 问答仍待验收。
