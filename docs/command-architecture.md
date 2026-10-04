# Command/application boundaries

Chat, CLI and dynamic tools are adapters over shared application operations; chat
does not launch the CLI. Both binaries call `cli::entry`, without source include.

Explicit `/sh` tasks use a separate four-slot scheduler lane. They can run alongside
Agent tasks in the same workspace, including when all Agent workers are occupied.
Shell tasks serialize with other shell tasks in their workspace; ordinary Agent
tasks retain their existing workspace/session serialization. Shutdown/recovery,
authorization, frozen shell cwd, cancellation and delivery use the shared task lifecycle.

- `application::Scope`: local administration or Bot-scoped ownership checks.
- `application::agents`: lifecycle/configuration reused by CLI/chat/profile tool.
- `application::cron`: draft creation/confirmation/cancellation and scoped actions.
  Jobs remain Agent-associated and retain their configuration snapshot.
- `application::tasks`: task details, result, events and delivery status.
- `application::commands`: argument validation and explicit pagination; free-text
  prompts/system/shell arguments are not parsed as pagination.
- `help::HELP`: shared help catalog and detail generator for `/help topic` and
  `/topic help`. Adapter parsers are still explicit, not generated from this catalog.

## Results

`application::output::Output` models Detail/Listing/Help with literal strings. Core
Agent/template/task views and help build RichText directly. Other existing formatter
branches remain as a compatibility path; this is not a complete typed-result migration.
`-r` serializes data before presentation. `/status` now returns a summary object;
`/tasks` retains the task array. Explicit pagination returns items/page/limit/total/
has_more/context; unpaginated raw lists keep their existing shape. Agent list adds
current, and cron history adds execution/delivery state alongside trigger status.

## Execution

Connect/set-thread errors are tagged `BeforeTurn` at the executor boundary and
finish failed. After turn submission, errors retain uncertain/interrupted semantics.
The scheduler no longer scans or matches error strings. Old interrupted tasks are
not automatically reclassified. Application errors expose stable codes where typed;
missing database records receive a readable not-found response.

## Delivery and persistence

`delivery` resolves destinations, persists the send plan, checkpoints each part and
retains uploaded file requests for retry. Adapters only encode/send. A crash between
remote acceptance and local checkpoint can still leave uncertain delivery; stable
request IDs depend on provider deduplication, not an exactly-once guarantee.

Cron transactions live in `store/cron.rs`; Store methods remain compatible. Existing
JSON snapshots and transactional boundaries are preserved without a schema migration.
