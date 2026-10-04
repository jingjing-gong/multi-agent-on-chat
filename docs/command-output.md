# Command output contract

All command/subcommand results now produce channel-independent rich-text spans (raw mode stays literal JSON). Escaped Markdown punctuation and HTML entities are decoded in the common formatter before channel delivery; emphasis is represented as span metadata. Template delete uses `/template delete name@version`, removes only that registered version and leaves existing Agent snapshots intact. The required bootstrap template `default@1` cannot be deleted.

Presentation produces channel-independent `Message::Plain`, `Markdown` or `RichText` (rows of text spans with emphasis). Structured command messages are saved atomically with their outbox rows under `delivery_payload`; retries reuse that payload. Existing text-only deliveries remain readable. Help is built as rich-text rows in the presentation layer. Lark translates spans to native post elements; text-only channels flatten the same rows. Adapters do not import the help catalog, recognize command names, or compare message bodies to identify commands.

Lark delivery uses a rich `post` with a Markdown element, for both new messages and replies. GFM tables become labelled records because Lark post rendering does not consistently support pipe tables. Raw JSON remains a literal text message. The raw flag is now `-r`; `--raw` is no longer an output flag.

Business handlers retain JSON values. Chat delivery, local `mac-cli chat`, and dynamic-tool responses apply deterministic presentation; no extra model call is needed. A bare final `-r` requests original JSON; quoted/escaped tokens remain arguments. Tools use `raw: true`. Raw changes output only, never ownership checks or credential redaction. Errors preserve failure status and display a readable message.

| Command / branches | Default Markdown fields |
| --- | --- |
| `/help` | Two lines per command: usage with merged options, then a basic description. Aliases share an entry; no heading prefix or truncation. `/help -r` returns the same catalog as JSON |
| `/bot`, `show [id]`, `enable [id]`, `disable [id]` | Bot ID, channel, enabled, App ID |
| `/bot list` | Same fields in a table |
| `/bot help` | All actions and cross-Bot targeting note |
| `/bot add local|lark|wechat` | Accepted name, registration status, status command; final Bot, enabled, pending owner, default Agent |
| `/bot add-status name` | Running/completed/failed state, final result or explicit error |
| `/profile`, `name`, `default` | Agent, profile, explicit model/effort, next-boundary notice on update |
| `/profile list` | Profile names only |
| `/model`, `id [effort]`, `effort value`, `default` | Model and effort |
| `/model list` | Model ID, display name, description, default/supported efforts |
| `/system`, `text`, `default` | Prompt preview; raw gives full configuration |
| `/cd`, `path` | Current/new canonical workspace |
| `/skills` | Skill names, explicit empty state |
| `/agents` | Agent/Bot, enabled, template, profile/model/effort |
| `/agent create`, `delete [--force]` | Resulting Agent state |
| `/agent use` | Selected Agent |
| `/agent` or unknown action | Usage |
| `/template`, `/template list` | Template/version, description, profile/model/effort/skills |
| `/template help` | List/show/from-agent usage; `/templates` removed in favor of `/template list` |
| `/template show`, `from-agent [agent] name@version` | Same configuration fields; export completion |
| `/tasks`, `/status` | Task/Agent, type, state, request/result previews |
| `/cancel [id]`, `/retry id`, `/compact`, `/sh command` | Task ID, Agent, type/state, request/result |
| `/clear` | New-conversation acknowledgement |
| `/ask agent prompt` | Child task and state |
| `/collaborations` | Session, Agent, state, recent activity |
| `/collaboration show id` | Session, Agent, state, configuration revision |
| `/inbox`, `/recv` | Delivery task, state, message preview |
| `/cron add` | No-job-created notice, draft, schedule/timezone, next fire, expiry, prompt, profile/model/effort, confirmation instructions |
| `/cron list` | Job/Agent, state, schedule/timezone, next fire, prompt |
| `/cron show`, `pause`, `resume`, `delete` | Resulting job fields |
| `/cron history` | Scheduled time, task, occurrence status |
| `/cron` or unknown action | Usage |
| `确认`/`confirm`, `取消`/`cancel` | Created job or cancelled draft |
| Invalid command/arguments/ownership | Explicit failure; no success-style acknowledgement |

Agent tools `agent_profile`, `bot_command`, `bot_add`, `template_from_agent` reuse the corresponding command formatter. `agent_list` presents Agent/Bot/description; `agent_query` presents accepted/request/session/state. Tool success flags remain unchanged.

Times are readable Beijing timestamps, alongside each job's configured timezone. Normal lists show up to 20 rows, fields up to 240 characters, with raw hints. Help is not row-limited. Markdown metacharacters and multiline values are escaped to avoid breaking tables.

Validation: `tests/command_presentation.rs` executes handler branches against temporary stores and checks default/raw results, parser quoting, empty/long values, dynamic tools, async states and actual history/inbox field shapes. `service` tests cover inbound receipt deduplication, durable command replies, onboarding and tool identity. Engine tests cover model-list/profile protocol; smoke scripts exercise service lifecycle, fake Lark credentials, owner binding and cross-Bot enable/disable. No new real Bot or real scheduled job is created for testing.
