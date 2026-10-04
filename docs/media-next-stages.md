# 多媒体后续阶段实施方案

2026-09-30。状态：M1 已获确认实施，实际范围与限制见 [pdf-extraction.md](pdf-extraction.md)；其余为后续规划。基于当前代码和 `media-implementation.md`，衔接已批准的第一期。

## 1. 当前边界

当前已有不可变 Asset、Bot/会话隔离、引用和 GC、入站附件队列、Codex 图片输入、产物发布和 outbox 交付。后续沿这些边界扩展。

`src/service.rs::handle_media_tool` 中的 `media_extract` 目前仅提供有界 UTF-8 文本预览，其他格式返回 `parser_unavailable`。`media_extractions` 目前只是 id/data 表，不构成解析任务系统。不能把文件已下载、图片可识别视为文档和音视频解析已完成。

第一期验收记录仍把真实微信收图/识图、原会话产物回传列为待验收；实施后续阶段前检查最新记录，用实际用户消息补齐证据。不得用模拟入站或上传成功代替完整收发验收。

本机 PATH 检查：已发现 pdftotext、pdftoppm、ffmpeg、ffprobe；未发现 tesseract、whisper-cli。PATH 可用不等于依赖版本、语言包和模型已经验收。

## 2. 架构适配

继续使用单一 Rust 服务、SQLite、现有工作目录和 outbox；目前不引入额外数据库、消息中间件或解析微服务。

| 位置 | 后续职责 |
|---|---|
| src/media/extractors/ | 注册解析器、探测版本和能力、执行受限子进程、规范化解析结果 |
| src/media/extractions.rs | 持久化任务、领取租约、重试、取消、缓存和结果引用 |
| src/application/media.rs | extract/status/result/cancel 的共同入口及作用域校验 |
| src/media/input.rs | 从原始资产和解析结果生成有预算的模型输入；可从现有 media::input 抽出 |
| src/service.rs | 注册解析 worker、连接模型工具与应用服务；不放格式分支 |
| src/engine.rs | 编码文本/图片、提交模型；不调用 OCR/ffmpeg |
| src/channels/media.rs、delivery.rs | 继续负责渠道传输和交付，不引入文档解析逻辑 |

保留现有 Message、附件编号和 Asset 接口。优先为新增解析请求/结果增加 schema_version；完整公共 ContentMessage 协议等确有外部消费者再推进。避免一次替换消息协议、任务调度和全部解析器。

## 3. 必须先落实的解析契约

### 请求与缓存

ExtractionRequest 包含 asset_id、operation、规范化 options、requester/task 引用；Bot 与会话从调用上下文解析，不能由模型任意指定。operation 初期为 text、ocr，后续增加 transcribe、video_summary_input。

缓存键至少包含 Bot、会话作用域、原始 SHA-256、operation、解析器标识/版本、模型或语言包版本、规范化选项及结果 schema_version。每次命中仍校验原资产和结果资产的权限、有效期。认证二维码禁止解析；来源撤销或回收时缓存失效。

结果格式包含：

- schema_version、extraction_id、source_asset_id、parser/version、state。
- 有序 segments：type、text 或 derived_asset_id，以及 page、段落/表格位置或 start_ms/end_ms 来源坐标。DOCX 无可靠页码时仅返回段落/表格位置。
- warnings、truncated、实际处理页数/时长、结果字节数、错误码与 retryable。
- 完整结果 JSON 作为受管资产，工具只返回摘要和分页游标，避免一次将全文塞进模型上下文。

OCR、转写结果标记来源方法；不把引擎没有提供的置信度编造成百分比。文档中的指令作为附件内容处理，不作为系统指令执行。

### 状态与恢复

状态为 queued → running → succeeded / failed / blocked / cancelled；重试保留历史 attempt。blocked 表示缺程序、语言包、模型或凭据等需外部修复的条件。租约到期后可重新领取，结果提交必须同时校验 generation。

迁移增加可索引的 source_asset_id、scope、cache_key、state、attempts、next_at、lease_until、generation 列；结果内容仍可用 JSON。数据库迁移使用现有事务和备份流程，不另建独立状态库。

解析 worker 与 Agent、下载 worker 分开。建议初始全局 2、每 Bot 1，重型音视频另设全局 1。队列有总长度和每 Bot 上限，超限明确返回 busy。

原文件、结果 JSON、渲染页、音轨和关键帧全部通过现有 Asset 和 media_refs 管理。多个请求可引用同一解析任务；取消单个等待者不能误杀其他等待者。最后一个等待者取消时终止子进程并 fence 旧结果；成功缓存按保留期回收。

### 模型调用与会话顺序

第二期采用显式或按需解析，收到文件即可登记原始资产，不自动 OCR 全部文档。`media_extract` 命中缓存时直接返回；未命中时返回 extraction_id 和 queued/running。提供 status/result 分页读取，不要求模型反复短轮询。

第一版以明确的“解析中，可查询结果”闭环上线；自动完成通知使用现有 outbox，只发一次状态回执，不直接把完整全文回传聊天。工具调用的默认回执目标仅为发起任务的原会话；本地 CLI 默认只输出状态。

若希望解析完成后自动继续回答，必须先在现有 Task 调度中实现持久化依赖等待：释放 Agent 槽、记录 task/extraction 依赖、完成后以唯一键唤醒、恢复前校验取消和 generation。保留会话中的原始顺序位置，后续普通消息继续排队，控制命令仍可执行。已结束的模型 turn 不补写工具返回。此项是独立验收项，不能靠后台直接 start_turn 或无限阻塞动态工具实现。

### 共同接口

计划增量增加 `mac media extract <asset> --operation ...`、`extraction show/result/cancel`，mac-cli 共用实现；聊天提供对应的短查询命令。动态工具保留 media_extract，增加受限的 status/result 能力，参数和错误码共用 application 层。

`media capabilities` 与 Bot doctor 展示解析器版本、可用语言/模型、限制和缺失依赖；渠道上传权限与本地解析能力分别显示。错误码至少包括 unsupported_type、dependency_missing、encrypted_document、invalid_media、limit_exceeded、timeout、cancelled、provider_unavailable。用户回执不包含凭据或下载 URL。

## 4. 解析器与资源默认值

以下为建议初始配置，需要基准测试后调整，均支持本地配置覆盖；不会自动安装工具、下载模型或调用外部付费服务。

| 能力 | 建议实现 | 初始限额及降级 |
|---|---|---|
| UTF-8/Markdown | 将现有预览迁入统一解析器 | 全文结果最多 2 MiB；模型单次读取最多 16 KiB，显示截断与下一页 |
| 文本 PDF | pdftotext；按页保存文本与页码 | 最多 100 页、120 秒、2 MiB 文本；加密文件明确提示 |
| DOCX | ZIP/XML 提取正文、段落和表格，禁外部实体和外链获取 | 解压总量 100 MiB、最多 5000 项；不执行宏、不承诺原排版 |
| 扫描 PDF/图片 OCR | pdftoppm 按需渲染 + Tesseract chi_sim/eng 可选插件 | 最多 20 页、单页 200 DPI 和 20 MP、总渲染预算 256 MiB、300 秒；未安装时 blocked |
| 音频转写 | ffprobe 检测、ffmpeg 规范化、Transcriber 接口 | 初期最多 30 分钟；保留时间戳，不默认说话人识别；后端/模型需选择 |
| 视频 | ffprobe、ffmpeg 抽取有时间戳的关键帧和音轨，复用图片/转写 | 初期最多 10 分钟、30 帧；明确这是采样理解，不能声称逐帧看完 |

所有子进程只接收经过校验的本地路径和结构化参数，不使用 shell 拼接；输入按文件特征识别。禁用解析器主动网络读取，限制 stdout、stderr、临时目录和执行时间，取消时处理子进程组。ZIP 需限制解压后体积/压缩比与路径穿越。CPU/内存资源限制的实际能力须验证；不能把超时和临时目录称为安全沙箱。

分页、页码范围、时间区间应在执行前参与预算；超过限制可返回已完成的部分与 warnings，但不能标记完整成功。原文件始终保留供用户导出。

## 5. 实施顺序和完成标准

| 里程碑 | 范围 | 通过条件 |
|---|---|---|
| M0：一期收尾 | 补真实渠道验收、确认已有恢复/权限反馈；记录已知限制 | 图片输入回答正确、原会话产物可打开、文件摘要一致；无活动任务丢失 |
| M1：解析底座与文本 PDF/DOCX | 契约、任务表、worker、缓存、结果分页、能力诊断；从 service 移出文本解析 | CLI/聊天/工具共用接口；重启恢复、取消 fencing、跨 Bot 隔离、版本缓存失效、GC 均通过 |
| M2：OCR 与自动续答 | 可选 OCR 插件；Task 依赖等待/唤醒 | 中文扫描页可定位回原页；缺语言包明确提示；长解析不占 Agent 槽，完成只续答一次 |
| M3：音频 | 选定并接入一个转写后端，时间戳及结果分页 | 已知录音内容可核对，超时/取消可恢复；成本和文件外发行为有明示配置 |
| M4：视频 | 关键帧与音轨派生资产，复用 M3 | 回答引用时间点、采样范围明确、资源上限生效、派生资产能回收 |

每一步包括边界样本：损坏/加密文档、扫描/文字混合 PDF、长文本、压缩炸弹、依赖缺失、租约重领、结果写入前后崩溃、取消与完成竞争。不因 PDF 解析通过就宣称 OCR 或音视频完成。

分页/pin 管理、上传并发提升、微信原生图片展示可以独立迭代；它们不阻塞本地文档解析。完整公共消息协议和搜索索引没有当前消费者，暂不作为前置工程。

## 6. 仍需确认的产品选择

M1 可以按以上本地优先、显式解析、结构化结果的默认方案开始，无须等待音视频选型。

M2 上线前确认 OCR 程序和中英文语言包安装方案。M3 前确定本地转写模型或外部服务；外部服务还需明确文件是否允许外发、费用上限、凭据和保留策略。未配置时返回 dependency_missing，不静默上传或换供应商。自动解析所有附件、长视频、说话人识别、完整表格布局还原均不作为默认承诺。

M1 实施与部署情况以 pdf-extraction.md 为准；本文件保留后续阶段设计。
