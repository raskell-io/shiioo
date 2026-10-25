//! `shiioo`: the office on this machine.
//!
//! `run`, `resume` and `log` work on single runs from a terminal; `serve`
//! (alias `up`) runs the office: the API and dashboard, the Telegram bot,
//! the workers and the issue intake; `install` makes it a login service;
//! `hook` is what the harnesses call before each tool call. Everything
//! lives in `<workspace>/.shiioo/` (see `docs/office-layout.md` in this
//! crate). Kill the process at any point and the runs continue from their
//! logs on the next start.

use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};
use shiioo_api::{
    ApiOptions, ApiState, CapacityTracker, Employee, IsolationPolicy, IssueCreator, Notifications,
    Role, Routine,
};
use shiioo_core::{Event, EventStore, RunId, Seq, verify_chain};
use shiioo_rules::RuleSet;
use shiioo_runtime::{
    AgentSpec, AnthropicProvider, ContainerSpec, Egress, HarnessBilling, HarnessSpec,
    HookCommand, HookEndpoint, LocalMachine, ModelProvider, Mount, OpenshellSpec, RunOutcome,
    RunSpec, RunStatus, Runtime, SecretSource, Toolbox, UnconfiguredProvider, Worker,
    anthropic::DEFAULT_MODEL, branch_for,
};
use shiioo_secrets::SecretStore;
use shiioo_store::{Notifying, RedbStore};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

#[derive(Parser)]
#[command(name = "shiioo", version, about = "Shiioo, the Virtual Company OS")]
struct Cli {
    /// Directory the agent works in. Shiioo keeps its data in `.shiioo/` inside it.
    #[arg(long, short, global = true, default_value = ".")]
    workspace: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a run and work on it until it finishes.
    Run {
        /// What the agent should do.
        task: String,
        /// Run the task with an external agent harness (`claude-code` or
        /// `codex`) instead of Shiioo's own loop. Uses the harness's own
        /// login, so no API key is needed.
        #[arg(long, value_parser = ["claude-code", "codex"])]
        harness: Option<String>,
        /// Git repository the harness works on. The run gets its own
        /// worktree under .shiioo/worktrees and a `shiioo/<run>` branch.
        #[arg(long, requires = "harness")]
        repo: Option<PathBuf>,
        /// Model. Native runs default to claude-opus-5-5; harness runs to
        /// the harness's own default.
        #[arg(long)]
        model: Option<String>,
        /// Tools the agent may use. Native: Shiioo tool names (default
        /// read_file,write_file,shell). Harness: its allow rules (default
        /// "Read,Edit,Write,Glob,Grep,Bash(git *)").
        #[arg(long, value_delimiter = ',')]
        tools: Option<Vec<String>>,
        /// Extra instructions for the agent.
        #[arg(long, default_value = "")]
        system: String,
        #[arg(long, default_value_t = 50)]
        max_turns: u32,
        /// Reasoning effort for native runs: low, medium, high, xhigh or max.
        #[arg(long, default_value = "high")]
        effort: String,
        /// Override the harness's permission mode (Claude Code: default,
        /// acceptEdits, plan, dontAsk, bypassPermissions; Codex: read-only,
        /// workspace-write, danger-full-access).
        #[arg(long, requires = "harness")]
        permission_mode: Option<String>,
        /// Bill the harness to API credits (ANTHROPIC_API_KEY) instead of
        /// the subscription login.
        #[arg(long, requires = "harness")]
        api_billing: bool,
        /// Stop a harness run at this estimated spend (Claude Code with API
        /// billing only; Codex ignores it).
        #[arg(long, requires = "harness")]
        max_budget_usd: Option<f64>,
        /// Fail a harness run that takes longer than this, across resumes.
        #[arg(long, requires = "harness")]
        max_minutes: Option<u64>,
        /// Where the harness runs: `worktree` (your user, no sandbox),
        /// `container` (Docker, read-only root, egress allow list) or
        /// `openshell`. Default: the office's policy in config.toml.
        #[arg(long, requires = "harness", value_parser = ["worktree", "container", "openshell"])]
        isolation: Option<String>,
        /// Run as a configured role ([[roles]] in config.toml): its
        /// instructions, tools, model and limits are laid over the
        /// harness's defaults.
        #[arg(long, requires = "harness")]
        role: Option<String>,
    },
    /// Continue a run, for example after the process was killed.
    Resume {
        run: String,
        #[arg(long, default_value = "high")]
        effort: String,
    },
    /// Print a run's events and verify its hash chain.
    Log { run: String },
    /// Called by an agent harness (not by people): answers one of its hook
    /// events by asking the Shiioo server. Reads the hook's JSON on stdin,
    /// prints the decision on stdout.
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        event: HookEvent,
    },
    /// Run the office: the API and dashboard, the Telegram bot, and the
    /// workers, all in this process. `shiioo up` is the same command.
    #[command(visible_alias = "up")]
    Serve {
        /// Address to listen on. Loopback by default: the API runs tools on
        /// this machine. Reach it from your phone over Tailscale.
        #[arg(long, default_value = "127.0.0.1:7878")]
        listen: String,
        /// Keep the machine from sleeping while the office runs (macOS
        /// `caffeinate`). The installed service turns this on.
        #[arg(long)]
        keep_awake: bool,
        /// Model for runs that don't name an agent.
        #[arg(long, default_value = DEFAULT_MODEL)]
        model: String,
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "read_file,write_file,shell"
        )]
        tools: Vec<String>,
        #[arg(long, default_value = "high")]
        effort: String,
    },
    /// Install the office as a background service that starts at login,
    /// restarts if it stops, and keeps the machine awake (macOS launchd;
    /// Linux systemd --user).
    Install {
        #[arg(long, default_value = "127.0.0.1:7878")]
        listen: String,
        /// Telegram bot token to store for the service (default: the
        /// SHIIOO_TELEGRAM_TOKEN variable, if set).
        #[arg(long)]
        telegram_token: Option<String>,
        /// Print the service definition instead of installing it.
        #[arg(long)]
        print: bool,
    },
    /// Remove the background service (the office's data stays).
    Uninstall,
    /// Secrets for the agents (tokens, keys), encrypted in .shiioo/.
    /// Refer to one as `secret:<name>` in config.toml's [env].
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
    /// The container level's images and leftovers.
    Sandbox {
        #[command(subcommand)]
        action: SandboxAction,
    },
}

#[derive(Subcommand)]
enum SecretAction {
    /// Store a secret. The value is read from stdin (`echo -n … | shiioo
    /// secret set gh-token`), or from a variable or a file.
    Set {
        name: String,
        /// Take the value from this environment variable.
        #[arg(long)]
        from_env: Option<String>,
        /// Take the value from this file (trailing newline removed).
        #[arg(long)]
        from_file: Option<PathBuf>,
    },
    /// List the names (never the values).
    List,
    /// Remove a secret.
    Rm { name: String },
}

#[derive(Subcommand)]
enum SandboxAction {
    /// Build the harness and egress images now instead of on first use.
    Build,
    /// Remove containers and the network left by runs that did not clean
    /// up (names starting with `shiioo-`).
    Prune,
}

#[derive(Subcommand)]
enum HookEvent {
    /// The `PreToolUse` hook of Claude Code and Codex (same contract).
    /// Environment: SHIIOO_SERVER_URL, SHIIOO_API_TOKEN, SHIIOO_RUN_ID, set
    /// by the worker on the harness process.
    PreToolUse,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, Box<dyn std::error::Error>> {
    if let Command::Hook { event } = &cli.command {
        return hook(event).await;
    }
    let workspace = std::fs::canonicalize(&cli.workspace)?;
    let data = workspace.join(".shiioo");
    std::fs::create_dir_all(&data)?;
    let store = Arc::new(RedbStore::open(data.join("shiioo.redb"))?);

    match cli.command {
        Command::Log { run } => {
            let run = parse_run(&run)?;
            let log = store.read(run, Seq(0)).await?;
            log.iter().for_each(print_event);
            match verify_chain(&log) {
                Ok(()) => println!("chain verified: {} events", log.len()),
                Err(e) => println!("CHAIN BROKEN: {e}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Run {
            task,
            harness,
            repo,
            model,
            tools,
            system,
            max_turns,
            effort,
            permission_mode,
            api_billing,
            max_budget_usd,
            max_minutes,
            isolation,
            role,
        } => {
            let worker = worker(&store, &workspace, &data, &effort)?;
            let chosen_role = match &role {
                Some(name) => Some(
                    Config::load(&data.join("config.toml"))?
                        .roles
                        .into_iter()
                        .find(|r| &r.name == name)
                        .ok_or_else(|| format!("no role named {name} in config.toml"))?,
                ),
                None => None,
            };
            let runtime = match harness.as_deref() {
                Some(kind) => {
                    let mut h = if kind == "codex" {
                        HarnessSpec::codex()
                    } else {
                        HarnessSpec::claude_code()
                    };
                    h.repo = repo.as_ref().map(std::fs::canonicalize).transpose()?;
                    // The office's policy decides the level and the
                    // environment, as it does for runs from the phone.
                    let config = Config::load(&data.join("config.toml"))?;
                    let label = h.repo.as_ref().and_then(|path| {
                        config
                            .repos
                            .iter()
                            .find(|(_, p)| *p == path)
                            .map(|(l, _)| l.clone())
                    });
                    h.isolation = config.isolation_policy().resolve(
                        label.as_deref(),
                        isolation
                            .as_deref()
                            .or(chosen_role.as_ref().and_then(|r| r.isolation.as_deref())),
                    )?;
                    h.env = config.env.clone();
                    if let Some(mode) = permission_mode {
                        h.permission_mode = mode;
                    }
                    h.max_budget_usd = max_budget_usd;
                    h.max_minutes = max_minutes;
                    h.billing = if api_billing {
                        HarnessBilling::ApiKey
                    } else {
                        HarnessBilling::Subscription
                    };
                    Runtime::Harness(h)
                }
                None => Runtime::Native,
            };
            let is_harness = matches!(runtime, Runtime::Harness(_));
            let mut runtime = runtime;
            let mut agent = AgentSpec {
                    model: model.unwrap_or_else(|| {
                        if is_harness {
                            String::new()
                        } else {
                            DEFAULT_MODEL.into()
                        }
                    }),
                    system,
                    tools: tools.unwrap_or_else(|| {
                        if is_harness {
                            ["Read", "Edit", "Write", "Glob", "Grep", "Bash(git *)"]
                                .map(String::from)
                                .to_vec()
                        } else {
                            ["read_file", "write_file", "shell"]
                                .map(String::from)
                                .to_vec()
                        }
                    }),
                    max_turns,
            };
            if let Some(role) = &chosen_role {
                role.apply(&mut runtime, &mut agent);
            }
            let spec = RunSpec {
                task,
                agent,
                runtime,
            };
            let run = worker.create(&spec).await?;
            println!("run {run}");
            if repo.is_some() {
                println!(
                    "branch {} in {}",
                    branch_for(run),
                    data.join("worktrees").join(run.to_string()).display()
                );
            }
            drive(&worker, run).await
        }
        Command::Resume { run, effort } => {
            let worker = worker(&store, &workspace, &data, &effort)?;
            drive(&worker, parse_run(&run)?).await
        }
        Command::Serve {
            listen,
            keep_awake,
            model,
            tools,
            effort,
        } => {
            serve(
                &store, &workspace, &data, &listen, keep_awake, model, tools, &effort,
            )
            .await
        }
        Command::Install {
            listen,
            telegram_token,
            print,
        } => install(&workspace, &data, &listen, telegram_token, print),
        Command::Uninstall => uninstall(),
        Command::Secret { action } => secret_command(&data, action),
        Command::Sandbox { action } => sandbox_command(&data, action).await,
        // Answered before the store was opened: hooks run in the harness's
        // working directory, not in an office.
        Command::Hook { .. } => Ok(ExitCode::SUCCESS),
    }
}

/// `shiioo secret set|list|rm`.
fn secret_command(data: &Path, action: SecretAction) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let store = SecretStore::open(data)?;
    match action {
        SecretAction::Set {
            name,
            from_env,
            from_file,
        } => {
            shiioo_secrets::validate(&name)?;
            let value = match (from_env, from_file) {
                (Some(var), _) => std::env::var(&var)
                    .map_err(|_| format!("environment variable {var} is not set"))?,
                (None, Some(file)) => std::fs::read_to_string(&file)
                    .map_err(|e| format!("{}: {e}", file.display()))?
                    .trim_end_matches(['\n', '\r'])
                    .to_owned(),
                (None, None) => {
                    let mut value = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut value)?;
                    value.trim_end_matches(['\n', '\r']).to_owned()
                }
            };
            if value.is_empty() {
                return Err("refusing to store an empty secret".into());
            }
            store.set(&name, &value)?;
            println!(
                "stored secret {name} ({} bytes); refer to it as secret:{name}",
                value.len()
            );
            Ok(ExitCode::SUCCESS)
        }
        SecretAction::List => {
            let names = store.names()?;
            if names.is_empty() {
                println!("no secrets; `shiioo secret set <name>` stores one");
            }
            for name in names {
                println!("{name}");
            }
            Ok(ExitCode::SUCCESS)
        }
        SecretAction::Rm { name } => {
            if store.remove(&name)? {
                println!("removed secret {name}");
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("no secret named {name}");
                Ok(ExitCode::from(1))
            }
        }
    }
}

/// `shiioo sandbox build|prune`.
async fn sandbox_command(
    data: &Path,
    action: SandboxAction,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    use shiioo_runtime::sandbox::{container, egress};
    let config = Config::load(&data.join("config.toml"))?;
    let engine = if config.isolation.container.engine.is_empty() {
        "docker".to_owned()
    } else {
        config.isolation.container.engine.clone()
    };
    container::check_engine(&engine).await?;
    match action {
        SandboxAction::Build => {
            let dir = data.join("sandboxes").join("build");
            let image = if config.isolation.container.image.is_empty() {
                container::default_image()
            } else {
                config.isolation.container.image.clone()
            };
            if image == container::default_image() {
                container::ensure_image(&engine, &image, container::DOCKERFILE, &dir).await?;
            }
            container::ensure_image(&engine, &egress::image(), egress::DOCKERFILE, &dir).await?;
            egress::ensure_network(&engine).await?;
            println!("harness image: {image}\negress image: {}\nnetwork: {}", egress::image(), egress::NETWORK);
            Ok(ExitCode::SUCCESS)
        }
        SandboxAction::Prune => {
            let out = tokio::process::Command::new(&engine)
                .args(["ps", "-a", "--filter", "name=^shiioo-", "--format", "{{.Names}}"])
                .output()
                .await?;
            let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_owned)
                .filter(|n| !n.is_empty())
                .collect();
            if names.is_empty() {
                println!("nothing to prune");
            } else {
                tokio::process::Command::new(&engine)
                    .arg("rm")
                    .arg("-f")
                    .args(&names)
                    .status()
                    .await?;
                println!("removed {}", names.join(", "));
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Creates the issues of an approved plan through the `gh` CLI, run in the
/// repository (so per-repository account wrappers apply).
struct GhIssues(shiioo_intake::Gh);

impl IssueCreator for GhIssues {
    fn create_issue(
        &self,
        repo: &Path,
        title: &str,
        body: &str,
        label: &str,
    ) -> shiioo_api::plans::BoxFuture<'_, Result<String, String>> {
        let repo = repo.to_path_buf();
        let (title, body, label) = (title.to_owned(), body.to_owned(), label.to_owned());
        Box::pin(async move {
            self.0
                .create_issue(&repo, &title, &body, &label)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// The office's secret store as the workers see it: a missing or
/// unreadable store means no secrets, never a crash in a run.
struct StoreSecrets(SecretStore);

impl SecretSource for StoreSecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).ok().flatten()
    }

    fn all(&self) -> Vec<(String, String)> {
        self.0.all().unwrap_or_default()
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve(
    store: &Arc<RedbStore>,
    workspace: &Path,
    data: &Path,
    listen: &str,
    keep_awake: bool,
    model: String,
    tools: Vec<String>,
    effort: &str,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let _awake = if keep_awake { stay_awake() } else { None };
    let config = Config::load(&data.join("config.toml"))?;
    let rules = load_rules(&data.join("rules.toml"))?;
    let store = Arc::new(Notifying::new(RedbStore::clone(store)));
    let provider = provider(effort);
    let machine = Arc::new(
        LocalMachine::open(workspace, data.join("journal.jsonl"), Toolbox::builtin())?
            .with_worktrees(data.join("worktrees"))
            .with_sandboxes(data.join("sandboxes")),
    );
    let secrets: Arc<dyn SecretSource> = Arc::new(StoreSecrets(SecretStore::open(data)?));
    let token_path = data.join("api-token");
    let token = api_token(&token_path)?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let addr = listener.local_addr()?;

    // Harness hooks call back into this server; a wildcard bind is reached
    // on loopback.
    let hook_host = if addr.ip().is_unspecified() {
        "127.0.0.1".to_owned()
    } else {
        addr.ip().to_string()
    };
    let hooks = HookEndpoint {
        server_url: format!("http://{hook_host}:{}", addr.port()),
        token: token.clone(),
        command: HookCommand::Exe(std::env::current_exe()?),
    };
    let worker_store = Arc::clone(&store);
    let capacity = Arc::new(CapacityTracker::default());
    let observed = Arc::clone(&capacity);
    let worker_secrets = Arc::clone(&secrets);
    let workers = Arc::new(move || {
        let tracker = Arc::clone(&observed);
        Worker::new(
            Arc::clone(&worker_store),
            Arc::clone(&provider),
            Arc::clone(&machine),
            format!("local-{}", std::process::id()).as_str(),
        )
        .with_hooks(hooks.clone())
        .with_secrets(Arc::clone(&worker_secrets))
        .with_observer(Arc::new(move |ev| tracker.observe_event(ev)))
    });

    let telegram = config.telegram_bot(data);
    let notifications: Option<Arc<dyn Notifications>> = telegram.as_ref().map(|(bot, owner)| {
        Arc::new(TelegramNotifications::new(
            Arc::clone(bot),
            *owner,
            config.dashboard.url.clone(),
        )) as _
    });
    let state = ApiState::with_options(
        store,
        workers,
        token,
        AgentSpec {
            model,
            system: String::new(),
            tools,
            max_turns: 50,
        },
        ApiOptions {
            rules,
            employees: if config.employees.is_empty() {
                shiioo_api::default_employees()
            } else {
                config.employees.clone()
            },
            capacity,
            repos: config.repos.clone(),
            notifications,
            isolation: config.isolation_policy(),
            env: config.env.clone(),
            roles: config.roles.clone(),
            routines: config.routines.clone(),
            report: Some(config.report.schedule.clone()).filter(|s| !s.trim().is_empty()),
            data_dir: Some(data.to_path_buf()),
            issues: Some(Arc::new(GhIssues(shiioo_intake::Gh::new(None)))),
            intake_label: config.intake.label.clone(),
        },
    );
    tokio::spawn(Arc::clone(&state).run_scheduler());
    if !config.routines.is_empty() || !config.report.schedule.trim().is_empty() {
        println!("routines:\n{}", state.routines_text());
    }
    if !config.roles.is_empty() {
        println!(
            "roles: {}",
            config
                .roles
                .iter()
                .map(|r| format!("@{}", r.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if config.intake.enabled {
        let repos: Vec<shiioo_intake::RepoIntake> = config
            .repos
            .iter()
            .map(|(label, path)| shiioo_intake::RepoIntake {
                label: label.clone(),
                path: path.clone(),
            })
            .collect();
        let mut intake = shiioo_intake::IntakeConfig::new(repos, data.join("intake-state.json"));
        intake.issue_label = config.intake.label.clone();
        intake.interval = std::time::Duration::from_secs(config.intake.interval_secs.max(10));
        let sink = Arc::new(IssueSink {
            state: Arc::clone(&state),
        });
        tokio::spawn(shiioo_intake::run_intake(intake, sink));
        println!(
            "intake: watching {} repo(s) for issues labelled {}",
            config.repos.len(),
            config.intake.label
        );
    }
    if let Err(e) = state.seed_capacity().await {
        eprintln!("could not read capacity from the logs: {e}");
    }
    if let Some((bot, owner)) = telegram {
        let office = Arc::new(TelegramOffice {
            state: Arc::clone(&state),
        });
        tokio::spawn(shiioo_telegram::run_bot(bot, owner, office));
        println!("telegram bot: on, owner chat {owner}");
    } else {
        println!(
            "telegram bot: off (set SHIIOO_TELEGRAM_TOKEN and [telegram] owner_chat_id in {})",
            data.join("config.toml").display()
        );
    }
    println!(
        "listening on http://{addr}  ({} repo(s), rules: {})",
        config.repos.len(),
        data.join("rules.toml").display()
    );
    println!(
        "isolation: {} by default{}",
        if config.isolation.default.is_empty() {
            "worktree"
        } else {
            config.isolation.default.as_str()
        },
        if config.isolation.repos.is_empty() {
            String::new()
        } else {
            format!(
                "; {}",
                config
                    .isolation
                    .repos
                    .iter()
                    .map(|(r, l)| format!("{r}: {l}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    println!(
        "dashboard: http://{addr}/#token=<contents of {}>",
        token_path.display()
    );
    axum::serve(listener, shiioo_api::router(state))
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(ExitCode::SUCCESS)
}

/// `<data>/config.toml`: the office's settings that aren't secrets.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Config {
    telegram: TelegramConfig,
    dashboard: DashboardConfig,
    intake: IntakeSection,
    /// Who takes new tasks, in order. Empty = Claude Code, then Codex.
    employees: Vec<Employee>,
    /// Repository label → path. `/run <label> <task>` on the phone picks
    /// one; rules can be scoped to a label.
    repos: BTreeMap<String, PathBuf>,
    isolation: IsolationSection,
    egress: EgressSection,
    /// Environment every harness run gets. Values are literal or
    /// `secret:<name>`.
    env: BTreeMap<String, String>,
    /// Reusable agent configurations, named with `@` in `/run`.
    roles: Vec<Role>,
    /// Runs that start on a schedule.
    routines: Vec<Routine>,
    report: ReportSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ReportSection {
    /// When the morning report is sent (`weekdays 07:00`); empty = only
    /// on `/report`.
    schedule: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct IsolationSection {
    /// `worktree` (default), `container` or `openshell`.
    default: String,
    /// Repository label → level.
    repos: BTreeMap<String, String>,
    container: ContainerSection,
    openshell: OpenshellSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ContainerSection {
    image: String,
    engine: String,
    memory: Option<String>,
    cpus: Option<f64>,
    /// Host directories mounted at the same path: `"/path"` or `"/path:ro"`.
    mounts: Vec<String>,
    /// SSH remote prefixes rewritten to https://github.com/ inside.
    git_https: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OpenshellSection {
    image: String,
    cpus: Option<f64>,
    memory: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EgressSection {
    /// `allowlist` (default) or `open`.
    mode: String,
    /// Replaces the built-in list when set.
    allow: Vec<String>,
    /// Added to the list.
    also: Vec<String>,
}

impl Config {
    fn egress(&self) -> Egress {
        match self.egress.mode.as_str() {
            "open" => Egress::Open,
            _ => {
                let mut hosts = if self.egress.allow.is_empty() {
                    Egress::default_hosts()
                } else {
                    self.egress.allow.clone()
                };
                hosts.extend(self.egress.also.iter().cloned());
                Egress::Allowlist { hosts }
            }
        }
    }

    fn isolation_policy(&self) -> IsolationPolicy {
        let c = &self.isolation.container;
        let o = &self.isolation.openshell;
        IsolationPolicy {
            default: self.isolation.default.clone(),
            repos: self.isolation.repos.clone(),
            container: ContainerSpec {
                image: c.image.clone(),
                engine: c.engine.clone(),
                memory: c.memory.clone(),
                cpus: c.cpus,
                egress: self.egress(),
                mounts: c
                    .mounts
                    .iter()
                    .map(|m| match m.strip_suffix(":ro") {
                        Some(path) => Mount {
                            host: PathBuf::from(path),
                            readonly: true,
                        },
                        None => Mount {
                            host: PathBuf::from(m),
                            readonly: false,
                        },
                    })
                    .collect(),
                git_https: c.git_https.clone(),
            },
            openshell: OpenshellSpec {
                image: o.image.clone(),
                cpus: o.cpus,
                memory: o.memory.clone(),
                egress: self.egress(),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct IntakeSection {
    /// Watch the configured repos for issues labelled `label` (needs `gh`
    /// logged in). Off by default.
    enabled: bool,
    label: String,
    interval_secs: u64,
}

impl Default for IntakeSection {
    fn default() -> Self {
        Self {
            enabled: false,
            label: "shiioo".into(),
            interval_secs: 60,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DashboardConfig {
    /// Where the owner reaches the dashboard from the phone, for links in
    /// notifications (for example `https://mac-studio.tailnet.ts.net`).
    url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TelegramConfig {
    /// Prefer the SHIIOO_TELEGRAM_TOKEN environment variable.
    token: Option<String>,
    /// The owner's Telegram user id. Only this account is obeyed.
    owner_chat_id: Option<i64>,
}

const DEFAULT_CONFIG: &str = r#"# Shiioo office configuration.

[telegram]
# The bot token comes from the SHIIOO_TELEGRAM_TOKEN environment variable
# (or `token = "..."` here). Create a bot with @BotFather.
# Your own Telegram user id (the bot obeys nobody else). @userinfobot tells you yours.
# owner_chat_id = 123456789

[dashboard]
# Where you reach the dashboard from your phone; notifications link to it.
# After `tailscale serve --bg 7878`: url = "https://<machine>.<tailnet>.ts.net"
# url = ""

[repos]
# label = "/absolute/path/to/repository"
# shiioo = "/Users/me/code/shiioo"

[intake]
# Watch the repos above for issues labelled `shiioo` and work on them
# (comments and labels go through the `gh` CLI, which must be logged in).
enabled = false
label = "shiioo"
interval_secs = 60

# Who takes new tasks, in order. Skipped when their subscription window is
# above the limit. Default: claude-code, then codex.
# [[employees]]
# name = "claude"
# kind = "claude-code"
# max_five_hour = 0.95
# max_seven_day = 0.90
# [[employees]]
# name = "codex"
# kind = "codex"
# [[employees]]
# name = "local"
# kind = "native"
# model = "ollama/qwen3:32b"

[isolation]
# Where harness runs execute. `worktree`: as your user, no sandbox.
# `container`: in a Docker container per run (read-only root, no
# capabilities, the worktree mounted, egress through an allow list).
# `openshell`: an NVIDIA OpenShell sandbox (experimental).
default = "worktree"
# Per repository: [isolation.repos] myproject = "container"

[isolation.container]
# image = ""            # empty: Shiioo's own image, built on first use
# engine = "docker"     # or "podman"
# memory = "8g"
# cpus = 4.0
# mounts = ["/Users/me/data:ro"]
# SSH remote aliases to rewrite to https://github.com/ inside the container
# (it has no SSH keys; `gh` and GH_TOKEN handle pushes):
# git_https = ["git@github-work:"]

[egress]
# What sandboxed runs may reach: `allowlist` (default) or `open`.
mode = "allowlist"
# allow = [...]          # replaces the built-in list (model APIs, GitHub, registries)
# also = ["pypi.io"]     # adds to it

[env]
# Environment for every harness run. `secret:<name>` reads the secret store
# (`shiioo secret set <name>`); the log records the reference, not the value.
# GH_TOKEN = "secret:gh-token"

[report]
# The morning report on Telegram: what finished, what needs you, branches
# awaiting review, capacity left. Empty = only on /report.
# schedule = "weekdays 07:00"

# Roles: reusable agent configurations. `/run @reviewer <repo> <task>`.
# [[roles]]
# name = "reviewer"
# employee = "auto"
# system = "You review code and write findings to REVIEW.md. Never push."
# tools = ["Read", "Glob", "Grep", "Bash(git *)", "Write"]
# isolation = "container"
# max_minutes = 30

# Routines: runs that start on a schedule (hourly, every 6h, daily 02:00,
# weekdays 07:00, weekends 10:00, mon,thu 09:30). /routines lists them.
# [[routines]]
# name = "nightly-tests"
# schedule = "daily 02:00"
# repo = "myproject"
# role = "tester"            # or employee = "codex"
# task = "Run the full test suite; fix any flaky test you find and open a PR."
"#;

impl Config {
    fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.exists() {
            std::fs::write(path, DEFAULT_CONFIG)?;
        }
        let text = std::fs::read_to_string(path)?;
        let config: Config =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        for (label, repo) in &config.repos {
            if !repo.join(".git").exists() {
                eprintln!(
                    "warning: repo {label} at {} is not a git repository",
                    repo.display()
                );
            }
        }
        Ok(config)
    }

    /// The bot token comes from the environment, the config, or the
    /// `telegram-token` file `shiioo install` writes for the service.
    fn telegram_bot(&self, data: &Path) -> Option<(Arc<shiioo_telegram::Bot>, i64)> {
        let token = std::env::var("SHIIOO_TELEGRAM_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .or_else(|| self.telegram.token.clone())
            .or_else(|| {
                std::fs::read_to_string(data.join("telegram-token"))
                    .ok()
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty())
            })?;
        let owner = std::env::var("SHIIOO_TELEGRAM_OWNER")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(self.telegram.owner_chat_id);
        match owner {
            Some(owner) => Some((Arc::new(shiioo_telegram::Bot::new(token)), owner)),
            None => {
                eprintln!(
                    "telegram: token set but no owner; set [telegram] owner_chat_id in config.toml \
                     (or SHIIOO_TELEGRAM_OWNER). The bot stays off."
                );
                None
            }
        }
    }
}

/// `<data>/rules.toml`, created with the defaults on first start.
fn load_rules(path: &Path) -> Result<RuleSet, Box<dyn std::error::Error>> {
    if !path.exists() {
        std::fs::write(path, RuleSet::defaults().to_toml())?;
    }
    let text = std::fs::read_to_string(path)?;
    Ok(RuleSet::from_toml(&text).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// The phone's view of the office.
struct TelegramOffice {
    state: Arc<ApiState<RedbStore>>,
}

fn short(run: RunId) -> String {
    run.to_string()[..8].to_owned()
}

fn status_line(view: &shiioo_api::RunView) -> String {
    let (mark, detail) = match &view.status {
        RunStatus::Open => ("▶️", String::new()),
        RunStatus::WaitingApproval { summary, .. } => ("🔐", format!(" waiting: {summary}")),
        RunStatus::NeedsAttention { reason } => ("⚠️", format!(" {reason}")),
        RunStatus::Completed { .. } => ("✅", String::new()),
        RunStatus::Failed { reason } => ("❌", format!(" {reason}")),
        RunStatus::Cancelled { .. } => ("🛑", String::new()),
    };
    let task: String = view.task.chars().take(70).collect();
    format!("{mark} {} {task}{detail}", short(view.id))
}

impl shiioo_telegram::Office for TelegramOffice {
    fn repos(&self) -> Vec<String> {
        self.state.repos()
    }

    fn start_run(
        &self,
        repo: Option<String>,
        role: Option<String>,
        task: String,
    ) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let repos = self.state.repos();
            let label = match (repo, repos.as_slice()) {
                (Some(label), _) => Some(label),
                (None, [only]) => Some(only.clone()),
                (None, []) => None,
                (None, many) => {
                    return Err(format!(
                        "which repo? /run <repo> <task>, one of: {}",
                        many.join(", ")
                    ));
                }
            };
            let repo_path = match &label {
                Some(l) => Some(
                    self.state
                        .repo_path(l)
                        .ok_or_else(|| format!("unknown repo {l}; /repos lists them"))?,
                ),
                None => None,
            };
            let (runtime, agent, employee) = match &role {
                Some(role) => self.state.assign_role(role, repo_path, None),
                None => self.state.assign("auto", repo_path),
            }
            .map_err(|e| e.to_string())?;
            let employee = match &runtime {
                Runtime::Harness(h) if h.isolation.level() != "worktree" => {
                    format!("{employee}, {}", h.isolation.level())
                }
                _ => employee,
            };
            let spec = RunSpec {
                task: task.clone(),
                agent,
                runtime,
            };
            let run = self
                .state
                .start_run(spec)
                .await
                .map_err(|e| e.to_string())?;
            Ok(match label {
                Some(l) => format!(
                    "▶️ {} started on {l} ({employee}): {task}\nbranch {}",
                    short(run),
                    branch_for(run)
                ),
                None => format!("▶️ {} started ({employee}): {task}", short(run)),
            })
        })
    }

    fn roles(&self) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move { self.state.roles_text() })
    }

    fn routines(&self) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move { self.state.routines_text() })
    }

    fn fire_routine(&self, name: String) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            self.state
                .fire_routine(&name)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn report(&self) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move { self.state.report(None).await })
    }

    fn plan(
        &self,
        repo: Option<String>,
        goal: String,
    ) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let repos = self.state.repos();
            let label = match (repo, repos.as_slice()) {
                (Some(label), _) => label,
                (None, [only]) => only.clone(),
                (None, []) => return Err("no repos configured; a plan needs one".into()),
                (None, many) => {
                    return Err(format!(
                        "which repo? /plan <repo> <goal>, one of: {}",
                        many.join(", ")
                    ));
                }
            };
            let run = self
                .state
                .start_plan(&label, &goal, None)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "📋 planning {} for {label}: {goal}\nThe chief of staff is reading the repository; \
                 you get the plan here when it is done.",
                short(run)
            ))
        })
    }

    fn plan_approve(&self, plan: String) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            self.state
                .approve_plan(&plan)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn plan_show(&self, plan: String) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move { self.state.plan_details(&plan).map_err(|e| e.to_string()) })
    }

    fn plans(&self) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move { self.state.plans_text() })
    }

    fn status(&self) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move {
            let runs = match self.state.list_runs().await {
                Ok(runs) if runs.is_empty() => "No runs yet. Send me a task.".to_owned(),
                Ok(runs) => runs
                    .iter()
                    .take(15)
                    .map(status_line)
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(e) => format!("⚠️ {e}"),
            };
            format!("{runs}\n\n{}", self.state.capacity().text())
        })
    }

    fn capacity(&self) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move { self.state.capacity().text() })
    }

    fn log(&self, run: String) -> shiioo_telegram::BoxFuture<'_, String> {
        Box::pin(async move {
            let run = match self.state.resolve_run(&run).await {
                Ok(r) => r,
                Err(e) => return format!("⚠️ {e}"),
            };
            match self.state.log_text(run, 25).await {
                Ok(text) => text,
                Err(e) => format!("⚠️ {e}"),
            }
        })
    }

    fn cancel(&self, run: String) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let run = self
                .state
                .resolve_run(&run)
                .await
                .map_err(|e| e.to_string())?;
            self.state
                .cancel(run, "owner:telegram", "")
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("🛑 {} will stop at its next step", short(run)))
        })
    }

    fn decide(
        &self,
        approval: String,
        allow: bool,
    ) -> shiioo_telegram::BoxFuture<'_, Result<String, String>> {
        Box::pin(async move {
            let run = self
                .state
                .decide(&approval, allow, "owner:telegram", None, false)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "{} for {}",
                if allow { "allowed" } else { "denied" },
                short(run)
            ))
        })
    }
}

/// What the office tells the owner on its own initiative.
struct TelegramNotifications {
    notifier: shiioo_telegram::Notifier,
    dashboard_url: Option<String>,
    /// approval id → (message id, text) of the question we sent, so the
    /// message can be marked when the request expires.
    asked: Mutex<HashMap<String, (i64, String)>>,
}

impl TelegramNotifications {
    fn new(bot: Arc<shiioo_telegram::Bot>, owner: i64, dashboard_url: Option<String>) -> Self {
        Self {
            notifier: shiioo_telegram::Notifier::new(bot, owner),
            dashboard_url,
            asked: Mutex::new(HashMap::new()),
        }
    }

    /// A link to the run in the dashboard, when the owner configured one.
    fn link(&self, run: RunId) -> String {
        match &self.dashboard_url {
            Some(url) => format!("\n{}/#run={run}", url.trim_end_matches('/')),
            None => String::new(),
        }
    }
}

impl Notifications for TelegramNotifications {
    fn approval_requested(
        &self,
        run: RunId,
        approval_id: String,
        summary: String,
    ) -> shiioo_api::BoxFuture<'_, ()> {
        Box::pin(async move {
            let text = format!(
                "🔐 {} wants to run\n{summary}{}",
                short(run),
                self.link(run)
            );
            match self.notifier.ask_approval(&approval_id, &text).await {
                Ok(message_id) => {
                    if let Ok(mut asked) = self.asked.lock() {
                        asked.insert(approval_id, (message_id, text));
                    }
                }
                Err(e) => eprintln!("telegram: could not ask for approval: {e}"),
            }
        })
    }

    fn approval_resolved(
        &self,
        approval_id: String,
        outcome: String,
    ) -> shiioo_api::BoxFuture<'_, ()> {
        Box::pin(async move {
            let asked = self
                .asked
                .lock()
                .ok()
                .and_then(|mut a| a.remove(&approval_id));
            if let Some((message_id, text)) = asked {
                self.notifier
                    .resolve_approval(message_id, &text, &outcome)
                    .await
                    .ok();
            }
        })
    }

    fn message(&self, text: String) -> shiioo_api::BoxFuture<'_, ()> {
        Box::pin(async move {
            self.notifier.notify(&text).await.ok();
        })
    }

    fn run_ended(
        &self,
        run: RunId,
        task: String,
        status: RunStatus,
    ) -> shiioo_api::BoxFuture<'_, ()> {
        Box::pin(async move {
            let task: String = task.chars().take(120).collect();
            let text = match status {
                RunStatus::Completed { output } => {
                    let output: String = output.chars().take(600).collect();
                    format!("✅ {} finished: {task}\n\n{output}", short(run))
                }
                RunStatus::Failed { reason } => {
                    format!("❌ {} failed: {task}\n{reason}", short(run))
                }
                RunStatus::Cancelled { reason } => format!("🛑 {} {reason}: {task}", short(run)),
                RunStatus::NeedsAttention { reason } => {
                    format!("⚠️ {} needs you: {task}\n{reason}", short(run))
                }
                RunStatus::Open | RunStatus::WaitingApproval { .. } => {
                    format!("⏸ {} stopped without finishing: {task}", short(run))
                }
            };
            self.notifier
                .notify(&format!("{text}{}", self.link(run)))
                .await
                .ok();
        })
    }
}

/// Labelled GitHub issues become runs.
struct IssueSink {
    state: Arc<ApiState<RedbStore>>,
}

impl shiioo_intake::IntakeSink for IssueSink {
    fn start_run(
        &self,
        repo: &str,
        task: String,
    ) -> shiioo_intake::BoxFuture<'_, Result<shiioo_intake::Started, String>> {
        let repo = repo.to_owned();
        Box::pin(async move {
            let path = self
                .state
                .repo_path(&repo)
                .ok_or_else(|| format!("unknown repo {repo}"))?;
            let (runtime, agent, _) = self
                .state
                .assign("auto", Some(path))
                .map_err(|e| e.to_string())?;
            // The branch name derives from the run id, which doesn't exist
            // yet; the worktree is checked out on it, so the agent can
            // refer to the current branch.
            let task = shiioo_intake::with_branch(&task, "$(git branch --show-current)");
            let run = self
                .state
                .start_run(RunSpec {
                    task,
                    agent,
                    runtime,
                })
                .await
                .map_err(|e| e.to_string())?;
            Ok(shiioo_intake::Started {
                run_id: run.to_string(),
                branch: branch_for(run),
            })
        })
    }

    fn run_outcome(
        &self,
        run_id: &str,
    ) -> shiioo_intake::BoxFuture<'_, Option<Result<String, String>>> {
        let run_id = run_id.to_owned();
        Box::pin(async move {
            let run = uuid::Uuid::parse_str(&run_id).ok().map(RunId)?;
            let view = self.state.run_view(run).await.ok()?;
            match view.status {
                RunStatus::Open | RunStatus::WaitingApproval { .. } => None,
                RunStatus::Completed { output } => Some(Ok(output)),
                RunStatus::Failed { reason } => Some(Err(reason)),
                RunStatus::Cancelled { reason } => Some(Err(reason)),
                RunStatus::NeedsAttention { reason } => Some(Err(reason)),
            }
        })
    }
}

/// Keeps a Mac from sleeping while the office runs: `caffeinate` tied to
/// this process. Dropping the child ends it.
fn stay_awake() -> Option<tokio::process::Child> {
    if !cfg!(target_os = "macos") {
        eprintln!("--keep-awake is only implemented on macOS; ignoring");
        return None;
    }
    match tokio::process::Command::new("caffeinate")
        .args(["-s", "-i", "-w", &std::process::id().to_string()])
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => Some(child),
        Err(e) => {
            eprintln!("could not start caffeinate: {e}");
            None
        }
    }
}

const SERVICE_LABEL: &str = "io.raskell.shiioo";

/// Installs the office as a login service and prints how to reach it.
fn install(
    workspace: &Path,
    data: &Path,
    listen: &str,
    telegram_token: Option<String>,
    print: bool,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let path_var = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME")?;
    let logs = data.join("logs");
    let log_file = logs.join("serve.log");
    let definition = if cfg!(target_os = "macos") {
        launchd_plist(&exe, workspace, listen, &path_var, &home, &log_file)
    } else {
        systemd_unit(&exe, workspace, listen, &path_var)
    };
    if print {
        println!("{definition}");
        return Ok(ExitCode::SUCCESS);
    }

    std::fs::create_dir_all(&logs)?;
    let token_source = telegram_token.or_else(|| {
        std::env::var("SHIIOO_TELEGRAM_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
    });
    if let Some(token) = token_source {
        let path = data.join("telegram-token");
        std::fs::write(&path, token.trim())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        println!("telegram token stored in {}", path.display());
    }
    let api_token = api_token(&data.join("api-token"))?;

    if cfg!(target_os = "macos") {
        let plist = PathBuf::from(&home)
            .join("Library/LaunchAgents")
            .join(format!("{SERVICE_LABEL}.plist"));
        std::fs::create_dir_all(plist.parent().unwrap_or(Path::new("/")))?;
        std::fs::write(&plist, &definition)?;
        let domain = format!("gui/{}", uid()?);
        // Replace any earlier version, then start.
        run_quiet(
            "launchctl",
            &["bootout", &format!("{domain}/{SERVICE_LABEL}")],
        );
        run_checked(
            "launchctl",
            &["bootstrap", &domain, &plist.to_string_lossy()],
        )?;
        println!("installed {} and started it", plist.display());
    } else {
        let unit_dir = PathBuf::from(&home).join(".config/systemd/user");
        std::fs::create_dir_all(&unit_dir)?;
        let unit = unit_dir.join("shiioo.service");
        std::fs::write(&unit, &definition)?;
        run_checked("systemctl", &["--user", "daemon-reload"])?;
        run_checked(
            "systemctl",
            &["--user", "enable", "--now", "shiioo.service"],
        )?;
        println!("installed {} and started it", unit.display());
    }

    println!("logs: {}", log_file.display());
    println!("dashboard on this machine: http://{listen}/#token={api_token}");
    let port = listen.rsplit(':').next().unwrap_or("7878");
    match tailscale_dns_name() {
        Some(host) => {
            println!("from your phone, once: tailscale serve --bg {port}");
            println!("then open: https://{host}/#token={api_token}");
            println!(
                "and set [dashboard] url = \"https://{host}\" in {} for links in Telegram messages",
                data.join("config.toml").display()
            );
        }
        None => println!(
            "to reach it from your phone, install Tailscale on this machine and the phone, then: \
             tailscale serve --bg {port}"
        ),
    }
    Ok(ExitCode::SUCCESS)
}

fn uninstall() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    if cfg!(target_os = "macos") {
        let domain = format!("gui/{}", uid()?);
        run_quiet(
            "launchctl",
            &["bootout", &format!("{domain}/{SERVICE_LABEL}")],
        );
        let plist = PathBuf::from(&home)
            .join("Library/LaunchAgents")
            .join(format!("{SERVICE_LABEL}.plist"));
        if plist.exists() {
            std::fs::remove_file(&plist)?;
        }
        println!("removed the launchd service");
    } else {
        run_quiet(
            "systemctl",
            &["--user", "disable", "--now", "shiioo.service"],
        );
        let unit = PathBuf::from(&home).join(".config/systemd/user/shiioo.service");
        if unit.exists() {
            std::fs::remove_file(&unit)?;
        }
        run_quiet("systemctl", &["--user", "daemon-reload"]);
        println!("removed the systemd service");
    }
    Ok(ExitCode::SUCCESS)
}

fn launchd_plist(
    exe: &Path,
    workspace: &Path,
    listen: &str,
    path_var: &str,
    home: &str,
    log_file: &Path,
) -> String {
    let x = xml_escape;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>--workspace</string><string>{ws}</string>
    <string>serve</string>
    <string>--listen</string><string>{listen}</string>
    <string>--keep-awake</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>{path}</string>
    <key>HOME</key><string>{home}</string>
  </dict>
  <key>WorkingDirectory</key><string>{ws}</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Standard</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        exe = x(&exe.to_string_lossy()),
        ws = x(&workspace.to_string_lossy()),
        listen = x(listen),
        path = x(path_var),
        home = x(home),
        log = x(&log_file.to_string_lossy()),
    )
}

fn systemd_unit(exe: &Path, workspace: &Path, listen: &str, path_var: &str) -> String {
    format!(
        "[Unit]\nDescription=Shiioo office\nAfter=network-online.target\n\n[Service]\n\
         ExecStart={} --workspace {} serve --listen {listen}\nRestart=always\nRestartSec=3\n\
         Environment=PATH={path_var}\nWorkingDirectory={}\n\n[Install]\nWantedBy=default.target\n",
        exe.display(),
        workspace.display(),
        workspace.display()
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn uid() -> Result<String, Box<dyn std::error::Error>> {
    let out = std::process::Command::new("id").arg("-u").output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn run_quiet(program: &str, args: &[&str]) {
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok();
}

fn run_checked(program: &str, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let out = std::process::Command::new(program).args(args).output()?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(())
}

/// This machine's MagicDNS name, if Tailscale is installed and up.
fn tailscale_dns_name() -> Option<String> {
    let out = std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let status: Value = serde_json::from_slice(&out.stdout).ok()?;
    let name = status["Self"]["DNSName"].as_str()?.trim_end_matches('.');
    (!name.is_empty()).then(|| name.to_owned())
}

/// Reads the API token, creating a random one (readable only by this user)
/// on first use.
fn api_token(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(token) = std::fs::read_to_string(path) {
        return Ok(token.trim().to_owned());
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    std::fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

fn worker(
    store: &Arc<RedbStore>,
    workspace: &Path,
    data: &Path,
    effort: &str,
) -> Result<Worker<RedbStore>, Box<dyn std::error::Error>> {
    let provider = provider(effort);
    let machine = LocalMachine::open(workspace, data.join("journal.jsonl"), Toolbox::builtin())?
        .with_worktrees(data.join("worktrees"))
        .with_sandboxes(data.join("sandboxes"));
    let secrets: Arc<dyn SecretSource> = Arc::new(StoreSecrets(SecretStore::open(data)?));
    Ok(Worker::new(
        Arc::clone(store),
        provider,
        Arc::new(machine),
        format!("local-{}", std::process::id()).as_str(),
    )
    .with_secrets(secrets)
    .with_observer(Arc::new(print_event)))
}

/// Answers a harness hook by asking the server. Fails closed: if the
/// server can't be reached or doesn't answer, the tool call is denied with
/// the reason, and the harness tells the model.
async fn hook(event: &HookEvent) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let HookEvent::PreToolUse = event;
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
    let request: Value = serde_json::from_str(&input)?;
    let decision = match ask_server(&request).await {
        Ok(d) => d,
        Err(e) => json!({ "decision": "deny", "reason": format!("shiioo could not decide: {e}") }),
    };
    let allow = decision["decision"] == "allow";
    println!(
        "{}",
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": if allow { "allow" } else { "deny" },
                "permissionDecisionReason": decision["reason"].as_str().unwrap_or("decided by shiioo"),
            }
        })
    );
    Ok(ExitCode::SUCCESS)
}

async fn ask_server(request: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let server = std::env::var("SHIIOO_SERVER_URL")?;
    let token = std::env::var("SHIIOO_API_TOKEN")?;
    let run = std::env::var("SHIIOO_RUN_ID")?;
    // The request may wait hours for the owner; the harness's own hook
    // timeout is the limit, not ours.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            shiioo_runtime::harness::claude_code::HOOK_TIMEOUT_SECS + 60,
        ))
        .build()?;
    let response = http
        .post(format!("{server}/v1/runs/{run}/permissions"))
        .bearer_auth(token)
        .json(request)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

/// The native runtime's model providers: Anthropic for plain model names
/// (when an API key is configured), and local or OpenAI-compatible servers
/// behind prefixes (`ollama/qwen3:32b`, `lmstudio/…`, `openai/…`). Without
/// any credentials, native runs on cloud models fail with a clear message
/// while harness and local-model runs still work.
fn provider(effort: &str) -> Arc<dyn ModelProvider> {
    let default: Arc<dyn ModelProvider> = match AnthropicProvider::from_env() {
        Ok(mut p) => {
            p.effort = effort.to_owned();
            Arc::new(p)
        }
        Err(_) => Arc::new(UnconfiguredProvider),
    };
    Arc::new(shiioo_providers::from_env(default))
}

async fn drive(
    worker: &Worker<RedbStore>,
    run: RunId,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    // A lost lease on one machine means it expired (for example the laptop
    // slept); taking it again is safe, the fence protects the log.
    for _ in 0..3 {
        match worker.drive(run).await? {
            RunOutcome::Completed(text) => {
                println!("\n{text}");
                return Ok(ExitCode::SUCCESS);
            }
            RunOutcome::Failed(reason) => {
                eprintln!("run failed: {reason}");
                return Ok(ExitCode::from(1));
            }
            RunOutcome::NeedsAttention(reason) => {
                eprintln!("run needs attention: {reason}");
                return Ok(ExitCode::from(2));
            }
            RunOutcome::Cancelled(reason) => {
                eprintln!("run {reason}");
                return Ok(ExitCode::from(4));
            }
            RunOutcome::LostLease => eprintln!("lease lost, taking the run again"),
        }
    }
    Ok(ExitCode::from(3))
}

fn parse_run(s: &str) -> Result<RunId, uuid::Error> {
    Ok(RunId(uuid::Uuid::parse_str(s)?))
}

fn print_event(ev: &Event) {
    println!("#{:<3} {}", ev.seq.0, shiioo_api::describe(ev));
}
