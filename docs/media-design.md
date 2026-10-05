# 多媒体消息、资产与交付设计

> 分类：目标设计。当前已实现范围以 [媒体实施](media-implementation.md) 和 [PDF 解析](pdf-extraction.md) 为准；下文待确认/后续能力不表示已经提供。

日期：2026-09-30。状态：用户已确认实施；实际完成范围、验证与边界见 [media-implementation.md](media-implementation.md)。下文保留目标设计，不将尚未完成的目标视为已交付。

## 1. 目标与范围

用户直接向 Bot 发图片、图文或文件，Bot 能明确接收状态，下载资源，并将支持的内容交给 Agent；Agent 的文件产物可以可靠地回传到原会话。终端与聊天使用同一应用服务，不再各自编排注册二维码、文件上传和发送重试。

第一期交付：
- 飞书与微信图片接收、持久化下载、图片模型输入；图文顺序保留。
- 本地 CLI 附件提交，以及 Agent 文件产物的显式登记和回传。
- 飞书原生图片/文件发送；微信文件发送、经协议验证的原生图片发送。原生图片不可用时明确使用文件附件，不把降级算成原生图片通过验收。
- 普通文件的接收、存储、导出、发送；UTF-8 纯文本有界预览。
- 二维码使用同一资产与交付机制，带有效期。
- 状态、错误、配额、任务重启恢复、兼容迁移和端到端验证。

后续阶段：PDF/DOCX 等文档解析与 OCR；音频转写；视频关键帧与音轨。第一期接到这些文件时保留文件并说明尚无解析器，不静默丢弃、不伪造解析结果。

不包含：无限量云盘、媒体编辑器、自动扫描整个工作区并上传文件、因上传失败自动开通飞书权限、默认外部 OCR/转写服务。

## 2. 当前实现与问题

依据 src/message.rs、channels.rs、channels/wechat_file.rs、engine.rs、service.rs、delivery.rs、domain.rs、store.rs：
- mac 与 mac-cli 均调用 cli::entry；应共用能力定义与行为。
- Message 是 Plain / Markdown / RichText / File(PathBuf)；File 在飞书被当成图片，在微信被当成普通附件。
- Incoming 只有 text；飞书只支持 text/post，post 中图片成为占位；微信只提取 text_item。
- Task.prompt/result 是字符串；Codex turn/start 使用 text_input，尚无图片输入。
- deliveries 与 delivery_plan 已有持久化计划和分片检查点；微信上传请求有缓存，应复用，而不是另建发送队列。
- 图片权限不足时上传失败；现有失败记录未提供用户可用的能力诊断。
- 接收去重存在，但媒体下载若嵌入监听循环，会阻塞后续消息。将下载当成模型任务也会占满 Agent worker。
- computer use 的截图属于工具结果，并不会自动成为聊天附件；工具可用不等于图片接收或发送可用。

## 3. 核心约束

1. 原始消息先落库，再异步下载；监听器只验证身份、归一化、去重和入队。
2. asset_id 是逻辑资产引用；渠道 key、URL、本地路径都不是资产 ID。
3. 下载、解析、上传、发送分开记录状态；模型失败不能清除文件，发送失败不能重跑模型。
4. 执行中的任务引用不可变输入快照；新图片和 steer 作为新的输入批次记录，不能覆写旧输入。
5. 每次读取、下载、发送资产都校验当前 Bot、会话和用途；CLI 管理员访问与 Agent 工具身份分开处理。
6. 纯文本行为、既有任务、会话、cron 关联逻辑和 owner 绑定规则保持兼容。
7. 不宣称 exactly-once：远端成功而本地尚未提交时存在未知状态；依赖渠道幂等键、对账或明确的人工重试。
8. 当前 worker 使用 danger-full-access，资产授权是应用层防误用边界，不是同一宿主机上恶意进程之间的强隔离。

## 4. 模块边界

| 模块 | 职责 | 不负责 |
|---|---|---|
| channels | 解析渠道事件、识别资源引用、渠道上传下载与发送协议 | 任务排队、OCR、存储保留策略 |
| application/media | import/export/inspect/prepare-input/publish 的统一用例及作用域校验 | 渠道协议细节 |
| media/store | 不可变文件、元数据、引用、配额、回收 | 模型推理 |
| media/transfers | 下载/上传工作队列、lease、超时、重试和取消 | 生成自然语言回答 |
| media/extractors | 格式识别、解析产物与版本缓存 | 自行发送聊天消息 |
| engine | 文本/图片输入编码、模型能力检查、工具产物协议 | 调用飞书/微信上传 API |
| delivery | 渠道交付计划、顺序、幂等键、发送状态 | 重新执行模型 |
| CLI / chat / dynamic tools | 解析参数、显示结果，调用应用服务 | 重复实现传输与授权 |

建议新增 src/media/{mod,types,store,transfers,extractors}.rs 和 src/application/media.rs，拆出渠道 media 适配器。避免继续将所有逻辑追加在 service.rs。

## 5. 数据模型

### 5.1 消息

新增版本化 ContentMessage { schema_version: 2, parts: [...] }：
- Text { text, format: plain | markdown | rich_text }：复用已有文字表示。
- Attachment { asset_id, role: image | file | audio | video, caption? }。

IncomingMessage：渠道、Bot、事件 ID、消息 ID、发送者、会话/话题、接收时间、parts、最小资源来源元数据。图片只消息允许存在，不能因 text 为空而丢弃。

渠道消息中的下载地址、密钥和完整 API payload 不进入 ContentMessage 或 prompt。多张图保持原消息顺序；分开发送的文字与图片按会话事件顺序处理，第一期不做时间窗口自动拼接。

### 5.2 资产与文件

MediaAsset：
- id、owner_bot_id、origin_scope、origin_message_id/producer_task_id、purpose。
- display_name（清理后的显示名）、declared_mime、detected_mime、size、sha256。
- width/height/duration（可选）、availability、created_at、expires_at、retention_class。
- blob_ref（就绪后填写）、source_ref（受保护记录，不对模型公开）。

Blob：内容不可变。存储建议 MAC_DATA_DIR/media/blobs/<bot-id>/<digest>，临时下载 MAC_DATA_DIR/media/tmp/<transfer-id>.part。Bot ID 需验证，文件名不决定存储路径。相同 Bot 内可按摘要复用文件；不同会话仍需独立资产引用与授权；不同 Bot 默认不共享物理缓存。

ChannelResource：channel、bot_id、credential_identity、source_message_id、provider_key、加密或下载所需参数、有效期。不能根据 image_key/file_key 猜测所属 Bot。渠道 token 受同等文件权限保护，日志与用户结果不回显。

Artifact：asset_id、producer_task_id、role、caption、delivery_intent。工具截图若要回传，必须通过受控导出形成资产；仅出现在模型上下文中的图片不自动保存或发送。

### 5.3 状态记录

MediaTransfer：direction、asset_id、destination_identity、state、attempts、lease_owner、lease_until、generation、next_retry_at、error_code、provider_request_id。

MediaExtraction：asset_id、parser_id、parser_version、options_hash、state、result_asset_ids、source_locations、error_code。

AssetReference：asset_id、reference_type、reference_id、bot/scope、expires_at。任务、历史消息、未完成交付是引用；垃圾回收只能删除无有效引用的对象。

CapabilityObservation：bot_id + credential_identity + capability，status=unknown/supported/verified/denied/unsupported，checked_at、有效期、可公开错误与修复指引。服务进程重启或凭据改变后，旧观察不能继续表示当前已验证。

## 6. SQLite 与文件提交

保留 records 作为业务对象存储，但新增显式表承载需要查询与并发约束的资产、引用、传输、解析和媒体 inbox。迁移时使用 schema version；不把 base64 或文件内容写入 events。

- 媒体 inbox 唯一键：(bot_id, provider_event_id)。接收事务同时登记消息、资产占位、引用和下载任务，再允许事件确认/去重完成。
- 文件流式写入 .part，限制字节数并计算摘要，完成后 fsync、原子 rename，再事务标记 available。
- 崩溃在 rename 后、数据库提交前：留下不可见孤儿，由延迟清理回收；不假装数据库和文件系统有单一事务。
- 下载 lease 到期可重取；失败重试需要 generation/fencing，旧 worker 不得覆盖新结果。
- 任务创建使用 inbox 唯一关联，恢复时不重复 submit；媒体准备中的消息在 /tasks 有明确状态。
- 上传缓存键包含 Bot、渠道凭据身份、文件摘要、媒体角色/格式转换版本；不跨 Bot 重用。

## 7. 接收与模型输入流程

渠道事件 → owner/来源检查 → 原子登记 inbox 和占位资产 → 下载 → 类型校验/派生图 → 输入准备 → submit/resume/steer → Agent。

状态建议：received → preparing → ready → submitted；分支 blocked/failed/cancelled。

- 注册等待 owner 时沿用首次私聊只绑定的规则，不下载并处理陌生人的附件。绑定回复说明需再发一条消息或附件。
- 图片下载失败：保留消息和错误，告诉用户哪张图片失败及是否可重试；默认阻止本次带图任务，避免模型按缺图输入给出看似完整的回答。
- 同消息内不支持解析的普通文件：仍作为文件资产交给任务。传达“文件已接收，但内容尚未解析”；用户可明确选择只处理文字。
- 文件准备不消耗 Codex worker。每会话的媒体输入保持顺序，超时后转为可见 blocked 状态，而非无限卡队列。
- /cancel 同时取消输入准备及其未使用传输；不会移除被其他任务引用的文件。
- 当前 Codex app-server schema 有 localImage/image 类型；实现时以安装版本协议测试为准。图片输入使用受管理的本地文件或受控 data URI，不暴露第三方带 token URL。
- 模型能力来自 provider/model 的实际声明或可验证配置；未知不等于支持。拒绝图片时记录 model_vision_unsupported，不自动切模型，也不偷偷上传到外部 OCR 服务。
- 超大图生成派生图并保留原图、尺寸映射；多图/总输入预算超限先说明，不能悄悄丢弃后几张。
- steer 附件同样先入库。与当前模型 turn 的时序不允许时排到下一轮，不直接丢给只接收字符串的旧 steer 接口。

## 8. 输出产物、上传与交付

Agent 使用工具 media_publish：path、kind、caption；调用者身份由运行时注入，禁止模型传 bot_id 伪装身份。路径先导入成不可变快照，再生成产物记录。

默认只将用户要求的结果文件发送到当前任务的 reply target；没有 reply target 时仅保存产物供 CLI 导出。跨聊天发送是单独命令，不由模型自行改写收件人。

- 优先让模型在工作区产生文件；其他路径需本地管理员显式 import，不能仅凭回答出现路径自动上传。
- 工具生成文件、CLI 导入文件、登录二维码走相同 Asset 与 delivery 服务。
- 任务结果保留 text + artifacts；每个 artifact 对应独立交付状态。
- plan 阶段决定原生图片或附件降级并持久化，重试不重新猜测发送类型。
- 飞书：Image 对应图片上传，File 对应文件上传；保存已上传的 key 并绑定 App 身份。消息回复保持原话题和幂等键。
- 微信：按已验证协议实现下载解密、图片/文件上传。原生图片未确认时返回附件降级说明；不能靠换 media_type 推测协议。
- 同消息的说明与附件按固定计划顺序发送。上传/消息远端确认与本地 checkpoint 间的未知状态单独呈现；无渠道幂等或对账能力时，未知结果不盲目重发。
- 永久错误（缺权限、格式不支持）立即停止无效重试，并在原会话发送一次文字说明。临时错误按有界退避重试；失败的说明本身不递归生成说明。

二维码：purpose=auth_qr，URL 与图共同带有效期，禁止解析模型读取登录 token；仅原命令会话可访问。到期未发送的队列项直接 expired，不补发旧码。若 CLI 不提供到期时间，以注册会话截止时间保守约束；用户完成或取消后撤销后续发送并按短保留期清理。

## 9. 解析器设计

统一 Extractor 接口：probe(metadata)、extract(input, budget)、返回结构化文字、派生资产和来源定位。

- 第一期：UTF-8 纯文本预览、图片元数据与模型视觉输入，不引入强制 OCR。
- PDF：页级文本、表格及页码；扫描页才 OCR。DOCX：正文、标题、表格及内嵌图。明确外部依赖与文档渲染的隔离进程。
- 音频：转写段落、时间戳和语言。视频：帧时间戳、音轨转写与抽帧预算。
- 解析结果按输入摘要+解析器版本+选项缓存；失败、部分成功与完整成功区分。
- 文档内文字是用户提供的数据，不提高其指令优先级。压缩包、宏和可执行附件第一期不自动执行/展开。
- OCR/转写如需外部 API，必须先配置服务及数据发送范围；没有服务时显示 unavailable。

## 10. CLI、聊天和工具

以下为新增接口提案，不代表现在已支持。

终端：
- mac-cli media import ./chart.png --bot om-assistant：返回 asset_id。
- mac-cli media show <id>：大小、类型、来源、准备/解析状态及引用。
- mac-cli media export <id> --output ./downloads/chart.png：导出托管文件；已存在默认不覆盖。
- mac-cli media retry <id>：重试可恢复的下载或准备，不重跑完成的模型任务。
- mac-cli media send <id> --bot om-assistant --chat <chat-id>：显式发送，返回 delivery_id。
- mac-cli submit om-assistant-main "分析这张图" --attach ./chart.png（可重复）。
- mac-cli media gc --dry-run；显式执行才清理候选。

用 export 表示本地导出；远端下载由收到消息后的 download transfer 管理，避免同一个 download 命令混淆网络抓取与复制文件。

聊天：
- 用户直接发图片/文件，不要求先发命令。
- /media [list]：当前会话附件，紧凑分页。
- /media show <id>、/media retry <id>：详情和恢复。
- /task show <id>：输入附件、产物、发送状态。
- /bot doctor [id]：读配置和已知能力观察，不自动发测试图片或开权限。

Agent 工具：media_inspect、media_extract、media_publish；暂不增加任意目标发送工具。派发给其他 Agent 时由应用服务显式授予任务级资产引用，不将路径字符串当成共享授权。

mac 与 mac-cli 继续调用同一 entry；聊天与 CLI 同调用 application/media。图片附件后的 /命令解析只读取文字块，下载内容不能变成 Shell 命令。

## 11. 配置、容量和运维默认值（待确认）

先给可调的保守默认：每消息最多 10 个附件；单图片 20 MiB、单文件 100 MiB；单 Bot 托管存储软上限 2 GiB。
- 渠道或模型限制更低时取较小值；这些数值不是渠道保证。
- 全局下载/上传并发各 4、单 Bot 各 2；独立于 Codex worker 数。
- 网络传输单尝试超时 120 秒，临时失败最多 5 次；由错误分类决定能否重试。
- 普通资产最后有效引用释放后保留 7 天再回收；长期运行/交付引用阻止回收。用户显式 pin 的资产不自动清理。
- 登录二维码到期不再发送；终止后最多保留 1 小时用于诊断，随后清理资产及带 token 内容。
- 达到配额不驱逐活跃文件，返回 quota_exceeded 与清理入口。

默认不公开任意 URL 下载接口。渠道下载只允许其规定的域名、协议和重定向范围；下载以实际流量计费限额，检查 MIME 与文件魔数、尺寸和解压预算。图片派生过程限制像素与 CPU/内存；临时目录 0700、文件 0600。

长期历史消息只保留元数据不能伪装内容仍在。过期资产显示 expired；明确告知需要重传。

## 12. 诊断与错误契约

每个阶段输出稳定 error_code、phase、retryable、asset_id、可公开说明和修复动作。典型：download_expired / permission_denied / size_exceeded / mime_mismatch / model_vision_unsupported / parser_unavailable / upload_failed / delivery_unknown。

/bot doctor 将程序支持、权限验证、模型能力分开显示。例如“飞书原生图片：适配器支持，上传权限 denied，上次检查时间…”。预检无法保证当前权限，就显示 unknown；不能通过安装了 CLI 推断图片可发。

记录每阶段耗时、大小、重试、错误；不打印正文、base64、App Secret、下载解密材料。任务成功与附件发送失败独立显示。

## 13. 迁移与发布

1. 先备份数据库并建立前向迁移；新增表与 schema_version，旧 Message/Task/Delivery 有明确解码路径。
2. 旧文字任务映射为单 Text；保留 prompt/result 兼容读字段，业务逻辑统一从 ContentMessage 读取，避免两套内容分叉。
3. 旧 File(PathBuf) 在需要发送时导入快照；文件不存在返回可见错误，不清空旧队列。旧二维码缺少有效期且不能确认注册仍有效时不补发。
4. 保留 delivery_plan/part 的旧解码和幂等键；未知远端状态不重新上传后假装全新交付。
5. 增量发布：先飞书私聊图片，再微信图片，最后 CLI/Agent 产物与二维码迁移；整个第一期验收通过后才宣称完整能力。
6. 模式升级后旧二进制因 schema version 拒绝写库。回滚需停服务并按备份恢复，说明回滚期间新数据的处理；不承诺无损二进制降级。
7. 不修改已有 Bot 的 App 身份、owner、模型或全局沙箱；不重建 cron、会话或凭据。

## 14. 验收矩阵

自动化：
- 飞书 text/post/image/file，微信文字/图片/文件，图文顺序、纯附件、重复事件、非 owner、错误类型。
- 下载失败、超时、权限不足、过期、超量、恶意文件名、跨 Bot 资源引用、重定向、MIME 伪装。
- 崩溃点：接收事务后、下载 rename 后、上传成功后、发送成功后；恢复不能丢任务，未知交付必须显式呈现。
- 多图视觉编码、模型不支持、缺图不执行、steer 排队、取消与 lease fencing。
- 文件生成后修改/删除原路径，已导入资产保持不变；GC 不删除活动引用。
- 旧库、旧队列、旧文字任务与二维码迁移；跨 Bot 上传缓存隔离。

真实联调（模拟测试不能替代）：
- 用户向各渠道 Bot 发一张带已知内容的图片，模型回答正确；发图文、多图和普通文件。
- Agent 生成一张测试图并通过原会话返回，打开后内容一致；普通文件校验摘要一致。
- 明确验证飞书上传权限，微信原生图片或降级附件；不通过 mock 宣称权限可用。
- 重启后恢复未完成传输、二维码过期不再补发；至少一个权限不足场景收到明确文字反馈。

第一期不等于全文档/音视频解析；后续阶段在解析器、模型与外部服务选择明确后再启动。
