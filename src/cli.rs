use crate::application::{self, agents::ConfigPatch, Scope};
use crate::{channels, domain::*, service, store::Store};
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "mac-cli",
    version,
    about = "Durable personal multi-agent chat runtime"
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    Media {
        #[command(subcommand)]
        command: MediaCmd,
    },
    /// Manage one Agent's native Codex Skills.
    Skills {
        agent: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    Init,
    Serve {
        #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u8).range(1..=20))]
        workers: u8,
        #[arg(long)]
        mock: bool,
    },
    Bot {
        #[command(subcommand)]
        command: BotCmd,
    },
    Template {
        #[command(subcommand)]
        command: TemplateCmd,
    },
    Agent {
        #[command(subcommand)]
        command: AgentCmd,
    },
    Submit {
        agent: String,
        prompt: String,
        #[arg(long)]
        attach: Vec<PathBuf>,
        #[arg(long, default_value = "local")]
        scope: String,
    },
    Tasks {
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    Task {
        id: String,
    },
    Cancel {
        id: String,
    },
    Retry {
        id: String,
    },
    Chat {
        agent: String,
        message: String,
        #[arg(long, default_value = "local")]
        scope: String,
    },
    Query {
        #[arg(long)]
        source_task: String,
        #[arg(long)]
        target: String,
        #[arg(long)]
        collaboration: Option<String>,
        #[arg(long)]
        key: String,
        prompt: String,
    },
    Collaborations,
    Status,
    Stop,
    Doctor,
    Events {
        #[arg(long)]
        task: Option<String>,
        #[arg(long, default_value_t = 0)]
        after: i64,
        #[arg(long)]
        follow: bool,
    },
    Session {
        #[command(subcommand)]
        command: SessionCmd,
    },
    Collaboration {
        id: String,
    },
    Deliveries {
        #[arg(long)]
        retry: Option<String>,
    },
    Inspect {
        kind: String,
        #[arg(long)]
        id: Option<String>,
    },
    Cron {
        #[command(subcommand)]
        command: CronCmd,
    },
}
#[derive(Subcommand)]
enum MediaCmd {
    Extract {
        id: String,
        #[arg(long, default_value_t = 1)]
        start_page: u32,
        #[arg(long)]
        end_page: Option<u32>,
    },
    Status {
        id: String,
    },
    Result {
        id: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 4000)]
        limit: usize,
    },
    Cancel {
        id: String,
    },
    RetryExtraction {
        id: String,
    },

    Import {
        path: PathBuf,
        #[arg(long)]
        bot: String,
    },
    List {
        #[arg(long)]
        bot: Option<String>,
    },
    Show {
        id: String,
    },
    Export {
        id: String,
        #[arg(long)]
        output: PathBuf,
    },
    Send {
        id: String,
        #[arg(long)]
        bot: String,
        #[arg(long)]
        chat: String,
    },
    Retry {
        id: String,
    },
    Gc {
        #[arg(long, conflicts_with = "execute")]
        dry_run: bool,
        #[arg(long)]
        execute: bool,
    },
}
#[derive(Subcommand)]
enum SessionCmd {
    List,
    Show {
        id: String,
    },
    Clear {
        agent: String,
        #[arg(long, default_value = "local")]
        scope: String,
    },
    Close {
        id: String,
    },
}
#[derive(Subcommand)]
enum BotCmd {
    Add {
        channel: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        owner: Option<String>,
        /// Existing Lark application ID (cli_...). Requires a secret input option.
        #[arg(long, requires = "secret_input")]
        app_id: Option<String>,
        /// Pass App Secret directly to lark-cli through stdin; never a command argument.
        #[arg(
            long,
            requires = "app_id",
            group = "secret_input",
            conflicts_with = "app_secret"
        )]
        app_secret_stdin: bool,
        /// App Secret (visible in process arguments and potentially shell history).
        #[arg(
            long,
            requires = "app_id",
            group = "secret_input",
            conflicts_with = "app_secret_stdin"
        )]
        app_secret: Option<String>,
        /// Existing application platform; defaults to feishu.
        #[arg(long, value_parser = ["feishu", "lark"], requires = "app_id")]
        brand: Option<String>,
    },
    List,
    Show {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    Owner {
        id: String,
        principal: String,
    },
    /// Send a Lark connection message to the configured owner, even before serve starts.
    Notify {
        id: String,
    },
}
#[derive(Subcommand)]
enum TemplateCmd {
    Validate {
        file: PathBuf,
    },
    Register {
        file: PathBuf,
    },
    /// Save an Agent's current configuration as an immutable template (no history).
    FromAgent {
        agent: String,
        name: String,
        #[arg(long, default_value_t = 1)]
        version: u32,
        #[arg(long)]
        description: Option<String>,
    },
    List,
    Show {
        id: String,
    },
    Archive {
        id: String,
    },
}
#[derive(Subcommand)]
enum AgentCmd {
    Enable {
        id: String,
    },
    Disable {
        id: String,
        #[arg(long)]
        force: bool,
    },
    Create {
        name: String,
        #[arg(long)]
        bot: String,
        #[arg(long)]
        template: String,
        #[arg(long)]
        workspace: PathBuf,
    },
    List,
    Show {
        id: String,
    },
    Delete {
        id: String,
        #[arg(long)]
        force: bool,
    },
    Config {
        id: String,
        #[arg(long)]
        system: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        effort: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
}
#[derive(Subcommand)]
enum CronCmd {
    Add {
        #[arg(long)]
        agent: String,
        #[arg(long)]
        schedule: String,
        #[arg(long, default_value = "Asia/Shanghai")]
        tz: String,
        prompt: String,
    },
    Confirm {
        draft: String,
    },
    CancelDraft {
        draft: String,
    },
    Drafts,
    List,
    Show {
        id: String,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    Delete {
        id: String,
    },
    History {
        id: String,
    },
    Update {
        id: String,
        #[arg(long)]
        schedule: Option<String>,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        refresh_config: bool,
    },
}
fn print(v: impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}
fn root() -> PathBuf {
    std::env::var_os("MAC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".multi-agent-on-chat")
        })
}
pub async fn entry() {
    if let Err(e) = run().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open(cli.data_dir.unwrap_or_else(root))?;
    match cli.command {
        Cmd::Skills { agent, command } => {
            let command = shlex::try_join(command.iter().map(String::as_str))
                .map_err(|_| anyhow::anyhow!("invalid Skill argument"))?;
            print(crate::skills::command(&store, &agent, &command).await?)
        }
        Cmd::Init => {
            let created = store.initialize_defaults()?;
            print(
                json!({"data_dir":store.root,"schema":1,"default_template":"default@1","created_agents":created}),
            )
        }
        Cmd::Serve { workers, mock } => service::serve(store, workers as usize, mock).await,
        Cmd::Template { command } => match command {
            TemplateCmd::Validate { ref file } | TemplateCmd::Register { ref file } => {
                let t: Template = serde_yaml::from_str(&std::fs::read_to_string(file)?)?;
                t.validate()?;
                if matches!(command, TemplateCmd::Register { .. }) {
                    store.register_template(&t)?;
                }
                print(t)
            }
            TemplateCmd::FromAgent {
                agent,
                name,
                version,
                description,
            } => {
                print(store.template_from_agent(&agent, &name, version, description.as_deref())?)
            }
            TemplateCmd::List => print(store.list::<Template>("template")?),
            TemplateCmd::Show { id } => print(store.get::<Template>("template", &id)?),
            TemplateCmd::Archive { id } => {
                store.save("archived_template", &id, &true)?;
                print(json!({"archived":id}))
            }
        },
        Cmd::Bot { command } => match command {
            BotCmd::Add {
                channel,
                name,
                owner,
                app_id,
                app_secret_stdin: _,
                app_secret,
                brand,
            } => {
                if channel != "lark" && app_id.is_some() {
                    bail!("--app-id is supported only for lark bots");
                }
                let id = name.unwrap_or_else(|| new_id(&channel));
                identifier(&id)?;
                if store.get::<Bot>("bot", &id).is_ok() {
                    bail!("bot exists");
                }
                let mut b = Bot {
                    id: id.clone(),
                    channel: channel.clone(),
                    account_id: id.clone(),
                    credential_ref: None,
                    enabled: true,
                    owner,
                };
                match channel.as_str() {
                    "local" => {
                        b.owner = Some("local".into());
                    }
                    "wechat" => {
                        let creds = channels::wechat_login().await?;
                        b.account_id = creds.account_id.clone();
                        b.owner = Some(creds.owner.clone());
                        let p = store.root.join("credentials").join(format!("{id}.json"));
                        channels::private_write(&p, &serde_json::to_vec(&creds)?)?;
                        b.credential_ref = Some(p);
                    }
                    "lark" => {
                        let p = channels::lark_registration_directory(
                            &store.root.join("credentials").join(&id),
                        )?;
                        b.account_id = channels::lark_login_with_secret(
                            &p,
                            app_id.as_deref(),
                            brand.as_deref().unwrap_or("feishu"),
                            app_secret.as_deref(),
                        )
                        .await?;
                        b.credential_ref = Some(p);
                        if b.owner.is_none() {
                            b.enabled = false;
                            eprintln!("Bot registered disabled: set verified owner with mac bot owner, then enable.");
                        }
                    }
                    _ => bail!("channel must be local, wechat or lark"),
                }
                let default_agent = store.add_bot_with_defaults(&b)?;
                let notification = notify_lark_added(&store, &b).await?;
                let mut output = serde_json::to_value(&b)?;
                output["notification"] = notification;
                output["default_agent"] = serde_json::to_value(default_agent)?;
                print(output)
            }
            BotCmd::List => print(store.list::<Bot>("bot")?),
            BotCmd::Show { id } => print(store.get::<Bot>("bot", &id)?),
            BotCmd::Enable { ref id } | BotCmd::Disable { ref id } => {
                print(store.set_bot_enabled(id, matches!(command, BotCmd::Enable { .. }))?)
            }
            BotCmd::Notify { id } => {
                let b: Bot = store.get("bot", &id)?;
                if b.channel != "lark" {
                    bail!("bot notify currently supports Lark only");
                }
                if b.owner.as_deref().is_none_or(|x| x.trim().is_empty()) {
                    bail!("set an owner first: mac-cli bot owner <bot> <open_id>");
                }
                let result = notify_lark_added(&store, &b).await?;
                let sent = result["status"] == "sent";
                print(&result)?;
                if !sent {
                    bail!("connection message failed; bot remains registered; inspect JSON result");
                }
                Ok(())
            }
            BotCmd::Owner { id, principal } => {
                let mut b: Bot = store.get("bot", &id)?;
                b.owner = Some(principal);
                store.save("bot", &id, &b)?;
                print(b)
            }
        },
        Cmd::Agent { command } => match command {
            AgentCmd::Enable { id } => {
                print(application::agents::enable(&store, &Scope::Local, &id)?)
            }
            AgentCmd::Disable { id, force } => print(application::agents::delete(
                &store,
                &Scope::Local,
                &id,
                force,
            )?),
            AgentCmd::Create {
                name,
                bot,
                template,
                workspace,
            } => print(application::agents::create(
                &store,
                &Scope::Local,
                &name,
                &bot,
                &template,
                &workspace,
            )?),
            AgentCmd::List => print(application::agents::list(&store, &Scope::Local, true)?),
            AgentCmd::Show { id } => print(Scope::Local.agent(&store, &id)?),
            AgentCmd::Delete { id, force } => print(application::agents::delete(
                &store,
                &Scope::Local,
                &id,
                force,
            )?),
            AgentCmd::Config {
                id,
                system,
                model,
                effort,
                profile,
            } => print(application::agents::configure(
                &store,
                &Scope::Local,
                &id,
                ConfigPatch {
                    system,
                    model,
                    effort,
                    profile,
                },
            )?),
        },
        Cmd::Submit {
            agent,
            prompt,
            scope,
            attach,
        } => print(application::media::submit(
            &store,
            &Scope::Local,
            &agent,
            &scope,
            &prompt,
            &attach,
        )?),
        Cmd::Media { command } => match command {
            MediaCmd::Import { path, bot } => print(application::media::import_asset(
                &store,
                &Scope::Local,
                &path,
                &bot,
                "local",
            )?),
            MediaCmd::List { bot } => print(application::media::list(
                &store,
                &Scope::Local,
                bot.as_deref(),
            )?),
            MediaCmd::Show { id } => print(application::media::show(&store, &Scope::Local, &id)?),
            MediaCmd::Export { id, output } => print(application::media::export(
                &store,
                &Scope::Local,
                &id,
                &output,
            )?),
            MediaCmd::Send { id, bot, chat } => print(application::media::send(
                &store,
                &Scope::Local,
                &id,
                &bot,
                &chat,
            )?),
            MediaCmd::Extract {
                id,
                start_page,
                end_page,
            } => print(
                crate::media::extractions::request(
                    &store,
                    &id,
                    None,
                    None,
                    crate::media::extractors::Options {
                        start_page,
                        end_page,
                    },
                    None,
                    None,
                )
                .await?,
            ),
            MediaCmd::Status { id } => {
                print(crate::media::extractions::status(&store, &id, None, None)?)
            }
            MediaCmd::Result { id, offset, limit } => print(crate::media::extractions::result(
                &store, &id, None, None, offset, limit,
            )?),
            MediaCmd::Cancel { id } => {
                print(crate::media::extractions::cancel(&store, &id, None, None)?)
            }
            MediaCmd::RetryExtraction { id } => {
                print(crate::media::extractions::retry(&store, &id, None, None)?)
            }
            MediaCmd::Retry { id } => print(application::media::retry(&store, &Scope::Local, &id)?),
            MediaCmd::Gc {
                dry_run: _,
                execute,
            } => print(application::media::gc(&store, &Scope::Local, !execute)?),
        },
        Cmd::Tasks { limit } => {
            let mut tasks = store.list::<Task>("task")?;
            tasks.sort_by_key(|t| std::cmp::Reverse(t.created_at));
            tasks.truncate(limit);
            print(tasks)
        }
        Cmd::Task { id } => print(application::tasks::details(&store, &Scope::Local, &id)?),
        Cmd::Cancel { id } => print(store.cancel(&id)?),
        Cmd::Retry { id } => print(store.retry(&id)?),
        Cmd::Chat {
            agent,
            message,
            scope,
        } => {
            let result = match service::chat(&store, &agent, &scope, &message, None).await {
                Ok(result) => result,
                Err(error) => bail!(
                    "{}",
                    crate::presentation::error(&message, &error.to_string())
                ),
            };
            let result = service::wait_bot_add(&store, result).await?;
            println!("{}", crate::presentation::render(&message, &result));
            Ok(())
        }
        Cmd::Query {
            source_task,
            target,
            collaboration,
            key,
            prompt,
        } => print(store.query(
            &source_task,
            &target,
            &prompt,
            collaboration.as_deref(),
            &key,
        )?),
        Cmd::Collaborations => print(
            store
                .list::<Session>("session")?
                .into_iter()
                .filter(|s| s.kind == "collaboration")
                .collect::<Vec<_>>(),
        ),
        Cmd::Stop => {
            store.save("control", "shutdown", &true)?;
            print(json!({"requested":"graceful shutdown"}))
        }
        Cmd::Status => {
            let tasks = store.list::<Task>("task")?;
            let mut counts = std::collections::BTreeMap::new();
            for t in tasks {
                *counts.entry(t.state).or_insert(0usize) += 1;
            }
            let daemon = store
                .get::<serde_json::Value>("control", "daemon")
                .unwrap_or(json!(null));
            print(json!({"tasks":counts,"data_dir":store.root,"max_workers":20,"daemon":daemon}))
        }
        Cmd::Doctor => {
            let codex = tokio::process::Command::new("codex")
                .arg("--version")
                .output()
                .await;
            let lark = tokio::process::Command::new("lark-cli")
                .arg("--version")
                .output()
                .await;
            let c = store.conn()?;
            let check: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
            let computer_use = crate::engine::CodexEngine::computer_use_status(
                &std::env::current_dir()?,
            )
            .await
            .unwrap_or_else(|error| {
                json!({
                    "tools_loaded":false,"interaction_verified":false,"error":error.to_string()
                })
            });
            print(
                json!({"database":check,"media":crate::media::diagnostics(),"computer_use":computer_use,"codex":codex.map(|o|String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_else(|_|"unavailable".into()),"lark_cli":lark.map(|o|String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_else(|_|"unavailable".into()),"credentials":"not printed"}),
            )
        }
        Cmd::Events {
            task,
            mut after,
            follow,
        } => {
            loop {
                for event in store.events(task.as_deref(), after)? {
                    after = event["seq"].as_i64().unwrap_or(after);
                    println!("{}", serde_json::to_string(&event)?);
                }
                if !follow {
                    break;
                }
                tokio::select! {_=tokio::signal::ctrl_c()=>break,_=tokio::time::sleep(std::time::Duration::from_millis(250))=>{}}
            }
            Ok(())
        }
        Cmd::Session { command } => match command {
            SessionCmd::List => print(store.list::<Session>("session")?),
            SessionCmd::Show { id } => print(store.get::<Session>("session", &id)?),
            SessionCmd::Clear { agent, scope } => {
                store.clear(&agent, &scope)?;
                print(json!({"cleared":true}))
            }
            SessionCmd::Close { id } => {
                if store.list::<Task>("task")?.iter().any(|t| {
                    t.session_id == id
                        && matches!(
                            t.state.as_str(),
                            "running" | "queued" | "waiting_dependency"
                        )
                }) {
                    bail!("session has unfinished tasks");
                }
                let mut s: Session = store.get("session", &id)?;
                s.state = "closed".into();
                store.save("session", &id, &s)?;
                print(s)
            }
        },
        Cmd::Collaboration { id } => {
            let s: Session = store.get("session", &id)?;
            if s.kind != "collaboration" {
                bail!("not a collaboration");
            }
            let tasks = store
                .list::<Task>("task")?
                .into_iter()
                .filter(|t| t.session_id == id)
                .collect::<Vec<_>>();
            print(json!({"session":s,"requests":tasks}))
        }
        Cmd::Deliveries { retry } => {
            if let Some(id) = retry {
                let n=store.conn()?.execute("UPDATE deliveries SET state='pending',next_at=0,attempts=0 WHERE id=?1 AND state!='sent'",[id])?;
                if n == 0 {
                    bail!("delivery not found or already sent");
                }
            }
            print(store.deliveries()?)
        }
        Cmd::Inspect { kind, id } => {
            if ![
                "agent", "bot", "session", "task", "job", "template", "command", "draft", "control",
            ]
            .contains(&kind.as_str())
            {
                bail!("unsupported inspect kind");
            }
            match id {
                Some(id) => print(store.get::<serde_json::Value>(&kind, &id)?),
                None => print(store.list::<serde_json::Value>(&kind)?),
            }
        }
        Cmd::Cron { command } => match command {
            CronCmd::Add {
                agent,
                schedule,
                tz,
                prompt,
            } => print(application::cron::propose(
                &store,
                &Scope::Local,
                &agent,
                &schedule,
                &tz,
                &prompt,
                None,
            )?),
            CronCmd::Confirm { draft } => print(service::confirm_draft(&store, &draft)?),
            CronCmd::CancelDraft { draft } => print(service::cancel_draft(&store, &draft)?),
            CronCmd::Drafts => print(store.list::<serde_json::Value>("draft")?),
            CronCmd::List => print(store.visible_jobs()?),
            CronCmd::Show { id } => print(store.get::<Job>("job", &id)?),
            CronCmd::Pause { id } => print(application::cron::action(
                &store,
                &Scope::Local,
                &id,
                "pause",
            )?),
            CronCmd::Resume { id } => print(application::cron::action(
                &store,
                &Scope::Local,
                &id,
                "resume",
            )?),
            CronCmd::Delete { id } => print(application::cron::action(
                &store,
                &Scope::Local,
                &id,
                "delete",
            )?),
            CronCmd::History { id } => print(store.job_history(&id)?),
            CronCmd::Update {
                id,
                schedule,
                prompt,
                refresh_config,
            } => print(store.job_update(
                &id,
                schedule.as_deref(),
                prompt.as_deref(),
                refresh_config,
            )?),
        },
    }
}

async fn notify_lark_added(store: &Store, bot: &Bot) -> Result<serde_json::Value> {
    if bot.channel != "lark" {
        return Ok(json!({"status":"not_applicable"}));
    }
    let Some(owner) = bot.owner.as_deref() else {
        return Ok(
            json!({"status":"pending_owner","message":"Bot added; start mac-cli serve and send this bot a private text message to bind owner automatically. Or use bot owner followed by bot notify."}),
        );
    };
    let key = new_id("welcome");
    let target = ReplyTarget {
        chat_id: owner.into(),
        context_token: None,
        thread_id: None,
    };
    let text = format!(
        "接入成功：机器人 {} 已加入 multi-agent-on-chat。默认 Agent 已就绪，启动服务即可开始对话。",
        bot.id
    );
    let result = match channels::send(bot, &target, &text, &key).await {
        Ok(()) => json!({"status":"sent","recipient":owner}),
        Err(e) => {
            json!({"status":"failed","recipient":owner,"reason":e.to_string(),"bot_added":true})
        }
    };
    store.save("onboarding_notification", &bot.id, &result)?;
    Ok(result)
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    #[test]
    fn lark_secret_input_modes_are_exclusive() {
        assert!(Cli::try_parse_from([
            "mac-cli",
            "bot",
            "add",
            "lark",
            "--app-id",
            "cli_test",
            "--app-secret",
            "test-only"
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "mac-cli",
            "bot",
            "add",
            "lark",
            "--app-id",
            "cli_test",
            "--app-secret-stdin"
        ])
        .is_ok());
        assert!(
            Cli::try_parse_from(["mac-cli", "bot", "add", "lark", "--app-id", "cli_test"]).is_err()
        );
        assert!(Cli::try_parse_from([
            "mac-cli",
            "bot",
            "add",
            "lark",
            "--app-id",
            "cli_test",
            "--app-secret",
            "test-only",
            "--app-secret-stdin"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "mac-cli",
            "bot",
            "add",
            "lark",
            "--app-secret",
            "test-only"
        ])
        .is_err());
    }
}
