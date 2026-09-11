mod runtime;
mod state;

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand};
use silicon_browser::shared::*;
use silicon_browser::{Auth, Client};

use crate::state::{State, default_home, refresh_lock};

const REMOTE_HELP: &str = r#"Remote browser — use this for interaction-heavy work.

  sb profile ls
  sb session new {profileid} --name "..." --description "..." --ttl 30m
  # or: sb session new --incognito --name "..." --description "..." --ttl 15m
  sb run --help

Profiles keep one identity and one fixed proxy location. Incognito sessions have neither."#;

const DISCOVERY_HELP: &str = r#"Search & fetch — use this for read-heavy work. It needs no profile or session.

  sb search "{query}" --purpose "..."
  sb fetch https://one.example,https://two.example --purpose "..."
  sb search --help
  sb fetch --help

Search returns ranked URLs. Fetch renders pages and returns text; images are not fetched."#;

const MANAGED_RUN_HELP: &str = r#"Run browser commands in an active managed session.

  sb run {sessionid} "open https://example.com"
  sb run {sessionid} "snapshot -i"
  sb run {sessionid} "click @e1"
  sb run {sessionid} "fill @e2 'hello world'"
  sb run {sessionid} "get text @e3"
  sb run {sessionid} "wait --text 'Complete'"

Useful categories:
  navigate       open, back, forward, reload
  inspect        snapshot, get, is
  interact       click, fill, type, select, check, press, scroll
  wait/evaluate  wait, eval

Connection and session lifecycle are managed by Silicon Browser.
Screenshots, PDFs and local recordings write to your machine. Uploads transfer local file bytes.
Downloads support same-origin HTTP links and blob/data links. Button/script/POST downloads,
cross-origin frames, `wait --download` and `--download-path` are unsupported.
Use `sb session end {sessionid} --note "..."` instead of `close`.
Run `sb setup` to install the pinned runner and unlock its version-matched action help."#;

#[derive(Parser, Debug)]
#[command(
    name = "sb",
    version,
    about = "Managed remote browsers and fast web discovery",
    long_about = "Silicon Browser has two daily paths:\n  remote-browser   interaction-heavy work in an authenticated browser\n  search-and-fetch read-heavy research without a browser session\n\nRun `sb --help remote-browser` or `sb --help search-and-fetch` for the shortest useful flow.",
    disable_help_subcommand = true,
    arg_required_else_help = false
)]
struct Cli {
    /// Print structured JSON instead of concise text. Place before the subcommand.
    #[arg(long)]
    json: bool,
    /// Override the organization; setup saves it with newly exchanged credentials.
    #[arg(long = "org-id", global = true, env = "SB_ORG_ID", hide_env_values = true)]
    org_id: Option<String>,
    /// Override the backend URL; setup saves it with newly exchanged credentials.
    #[arg(long, global = true, env = "SB_BACKEND_URL", hide_env_values = true)]
    backend: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print this application's IAM identifier.
    Iam,
    /// Exchange an IAM short-lived token and store the resulting session.
    Login {
        token: Option<String>,
        #[command(subcommand)]
        command: Option<LoginCommand>,
    },
    /// Install/check the runner and authenticate with an IAM short-lived token.
    /// Recording delivery uses a separate fresh Browser oac_ token via SB_RECORDING_SLT
    /// or a masked prompt. An active backend authorization is reused.
    Setup {
        /// Select an organization already authorized by the short-lived token.
        #[arg(long)]
        org: Option<String>,
    },
    /// Manage persistent browser identities.
    Profile(Service<ProfileCommand>),
    /// List proxy locations available to new profiles.
    Proxy(Service<ProxyCommand>),
    /// Start, inspect, share, and end browser sessions.
    Session(Service<SessionCommand>),
    /// Run one browser command inside a managed session.
    Run(RunArgs),
    /// List and manage visual recordings.
    Recording(Service<RecordingCommand>),
    /// Inspect browser and proxy usage.
    Usage(Service<UsageCommand>),
    /// Find ranked URLs quickly with Silicon Browser.
    Search(SearchArgs),
    /// Render URLs and return extracted text with Silicon Browser.
    Fetch(FetchArgs),
}

#[derive(Subcommand, Debug)]
enum LoginCommand {
    /// Report whether the stored session is authenticated.
    Status,
}

#[derive(Args, Debug)]
struct Service<T: clap::Subcommand> {
    #[command(subcommand)]
    command: Option<T>,
}

#[derive(Subcommand, Debug)]
enum ProfileCommand {
    /// List profiles visible to you.
    Ls,
    /// Show one profile.
    Show { profile_id: String },
    /// Create a persistent profile; its location can never be changed.
    New {
        #[arg(long)]
        name: String,
        #[arg(long)]
        location: String,
        #[arg(long, default_value = "[]")]
        access: String,
    },
    /// Change only a profile's name or access list.
    Set {
        profile_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        access: Option<String>,
    },
    /// Retire a profile without deleting its history.
    End {
        profile_id: String,
        #[arg(long)]
        note: String,
    },
}

#[derive(Subcommand, Debug)]
enum ProxyCommand {
    /// List locations a profile can be pinned to.
    Ls,
}

#[derive(Subcommand, Debug)]
enum SessionCommand {
    /// Start a profile-backed or incognito session.
    New {
        profile_id: Option<String>,
        #[arg(long, conflicts_with = "profile_id")]
        incognito: bool,
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: String,
        /// Session duration: 15m, 30m, 45m, 60m, 120m, or 240m. Defaults to 15m for incognito; required for profiles.
        #[arg(long)]
        ttl: Option<SessionTtl>,
    },
    /// List visible sessions.
    Ls {
        #[arg(long)]
        filter: Option<String>,
    },
    /// Show session state and accrued usage.
    Show { session_id: String },
    /// Return a Silicon Browser link for a running session.
    Live { session_id: String },
    /// Show replayable sb commands for one day.
    Logs {
        session_id: String,
        #[arg(long)]
        date: Option<String>,
    },
    /// Retry locally queued command logs without repeating browser actions.
    Sync { session_id: String },
    /// Stop and store a session.
    End {
        session_id: String,
        #[arg(long)]
        note: String,
    },
}

#[derive(Args, Debug)]
struct RunArgs {
    session_id: String,
    /// One quoted browser command, passed without semantic rewriting.
    command: String,
    /// Additional browser command arguments.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    flags: Vec<String>,
}

#[derive(Subcommand, Debug)]
enum RecordingCommand {
    /// List recordings by session name and id.
    Ls {
        /// AND-separated discovery pipeline. Fields: `profile:<id>`, `for:@identity`,
        /// `name:<pattern>`, `description:<pattern>`, `contains:<text>`, and
        /// `is:incognito|mine|shared`. Text patterns accept `^prefix` and `*` wildcards.
        #[arg(long, value_name = "PIPELINE")]
        filter: Option<String>,
    },
    /// Show a recording or its honest pending state.
    Show { session_id: String },
    /// Retry eligible failed delivery artifacts as the initiating identity.
    /// Requires active recording authorization; completed Briefcase artifacts are preserved.
    /// A pending response means delivery was queued, not completed.
    Send { session_id: String },
    /// Hide a recording in Browser; its Briefcase files remain unchanged.
    Rm { session_id: String },
}

#[derive(Subcommand, Debug)]
enum UsageCommand {
    /// Read current browser capacity, shared across organizations.
    Limits,
    /// List per-session usage.
    Ls {
        #[arg(long)]
        filter: Option<String>,
    },
    /// Show one session, or the organization total with --org.
    Show {
        session_id: Option<String>,
        #[arg(long, conflicts_with = "session_id")]
        org: bool,
        /// Usage filter; organization totals accept only a between date window.
        #[arg(long, requires = "org", conflicts_with = "session_id")]
        filter: Option<String>,
    },
}

#[derive(Args, Debug)]
struct SearchArgs {
    query: String,
    /// Audit intent recorded by Silicon Browser; it is not an upstream reranking instruction.
    #[arg(long)]
    purpose: String,
    /// Search currently supports only `web`; `news` and `research` are rejected.
    #[arg(long = "type", value_enum, default_value = "web")]
    search_type: SearchTypeArg,
    #[arg(long, value_delimiter = ',')]
    include_domains: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    exclude_domains: Vec<String>,
    #[arg(long)]
    location: Option<String>,
    #[arg(long)]
    language: Option<String>,
    /// Currently unsupported by the search service.
    #[arg(long = "recency")]
    recency_minutes: Option<u64>,
    /// Currently unsupported by the search service.
    #[arg(long, value_parser = cli_date)]
    after: Option<NaiveDate>,
    /// Currently unsupported by the search service.
    #[arg(long, value_parser = cli_date)]
    before: Option<NaiveDate>,
    /// Currently unsupported by the search service.
    #[arg(long)]
    pub_year_min: Option<i32>,
    /// Currently unsupported by the search service.
    #[arg(long)]
    pub_year_max: Option<i32>,
    #[arg(long, default_value_t = 0)]
    page: u8,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum SearchTypeArg {
    Web,
    News,
    Research,
}

impl From<SearchTypeArg> for SearchType {
    fn from(value: SearchTypeArg) -> Self {
        match value {
            SearchTypeArg::Web => Self::Web,
            SearchTypeArg::News => Self::News,
            SearchTypeArg::Research => Self::Research,
        }
    }
}

#[derive(Args, Debug)]
struct FetchArgs {
    /// URLs as separate arguments, comma-separated, or `[url,url]`.
    #[arg(required = true)]
    urls: Vec<String>,
    /// Audit intent recorded by Silicon Browser; it is not an upstream extraction instruction.
    #[arg(long)]
    purpose: String,
    #[arg(long, value_enum, default_value = "markdown")]
    format: FetchFormatArg,
    #[arg(long)]
    links: bool,
    #[arg(long)]
    image_links: bool,
    /// Currently unsupported by the search service.
    #[arg(long = "ttl")]
    ttl_seconds: Option<u64>,
    /// Currently unsupported by the search service.
    #[arg(long = "timeout")]
    timeout_ms: Option<u64>,
    /// Currently unsupported by the search service.
    #[arg(long, value_delimiter = ',')]
    include_selectors: Vec<String>,
    /// Currently unsupported by the search service.
    #[arg(long, value_delimiter = ',')]
    exclude_selectors: Vec<String>,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum FetchFormatArg {
    Markdown,
    Html,
    Json,
}

impl From<FetchFormatArg> for FetchFormat {
    fn from(value: FetchFormatArg) -> Self {
        match value {
            FetchFormatArg::Markdown => Self::Markdown,
            FetchFormatArg::Html => Self::Html,
            FetchFormatArg::Json => Self::Json,
        }
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();
    if let Some(code) = special_help(&raw) {
        return code;
    }
    let cli = Cli::parse_from(parse_arguments(&raw));
    match execute(cli, &raw[1..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sb: {error}");
            ExitCode::from(error_exit_code(error.as_ref()))
        }
    }
}

fn parse_arguments(raw: &[String]) -> Vec<String> {
    if raw.get(1).is_some_and(|command| command != "run")
        && let Some(index) = raw.iter().position(|argument| argument == "--json")
        && index > 1
    {
        let mut parsed = raw.to_vec();
        let json = parsed.remove(index);
        parsed.insert(1, json);
        parsed
    } else {
        raw.to_vec()
    }
}

#[derive(Debug)]
struct RunnerExit(i32);

impl std::fmt::Display for RunnerExit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "browser command exited with {}", self.0)
    }
}

impl std::error::Error for RunnerExit {}

fn error_exit_code(error: &(dyn std::error::Error + 'static)) -> u8 {
    error.downcast_ref::<RunnerExit>().and_then(|exit| u8::try_from(exit.0).ok()).filter(|code| *code != 0).unwrap_or(1)
}

fn special_help(raw: &[String]) -> Option<ExitCode> {
    match raw.get(1..).unwrap_or_default() {
        [flag, topic] if flag == "--help" && topic == "remote-browser" => {
            println!("{REMOTE_HELP}");
            Some(ExitCode::SUCCESS)
        }
        [flag, topic] if flag == "--help" && topic == "search-and-fetch" => {
            println!("{DISCOVERY_HELP}");
            Some(ExitCode::SUCCESS)
        }
        [command, flag] if command == "run" && matches!(flag.as_str(), "--help" | "-h") => {
            match silicon_browser::setup::runner_help(runtime::controller_binary(), "{sessionid}") {
                Ok(help) => {
                    println!("{help}");
                }
                Err(error) => {
                    eprintln!("warning: {error}");
                    println!("{MANAGED_RUN_HELP}");
                }
            }
            Some(ExitCode::SUCCESS)
        }
        _ => None,
    }
}

fn execute(cli: Cli, arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(command) = &cli.command
        && print_service_verbs(command, cli.json)?
    {
        return Ok(());
    }
    let backend = cli.backend.as_deref().unwrap_or(silicon_browser::DEFAULT_BACKEND_URL);
    match &cli.command {
        Some(Command::Iam) => {
            let info = Client::iam(backend)?;
            if cli.json {
                print_json(&info)?;
            } else {
                println!("{}", info.app_id);
            }
            return Ok(());
        }
        Some(Command::Login { command: Some(LoginCommand::Status), .. }) => {
            return login_status(backend, cli.json, cli.org_id.as_deref());
        }
        Some(Command::Login { token: Some(token), command: None }) => {
            let root = default_home();
            let home = state::home_for_backend(&root, backend)?;
            let mut state = State::load(&home)?;
            let auth = Client::exchange(
                backend,
                &AuthExchangeRequest { short_lived_token: token.clone(), org_id: cli.org_id.clone() },
            )?;
            apply_auth_session(&mut state, &home, auth)?;
            if cli.json {
                print_json(
                    &serde_json::json!({"authenticated": true, "org": state.org_id, "identity": state.identity_id}),
                )?;
            } else {
                println!(
                    "authenticated as {} in {}",
                    state.identity_id.as_deref().unwrap_or("authenticated"),
                    state.org_id.as_deref().unwrap_or("unknown organization")
                );
            }
            return Ok(());
        }
        Some(Command::Login { token: None, command: None }) => return Err("usage: sb login <short-lived-token>".into()),
        _ => {}
    }
    let root = default_home();
    let home = state::home_for_backend(&root, backend)?;
    let mut state = State::load(&home)?;
    if state.stored_token().is_some() && state.credential_generation.is_none() {
        state = State::update(&home, |saved| {
            if saved.credential_generation.is_none() {
                saved.credential_generation = Some(uuid::Uuid::new_v4().to_string());
            }
        })?;
    }

    if let Some(Command::Setup { org }) = &cli.command {
        // Refresh stored credentials against their stored issuer and org before applying
        // one-command overrides. Setup is also the recovery path, so a failed refresh must not
        // prevent an interactive replacement with a new single-use IAM token. Any explicit
        // environment credential takes precedence, including an oac_ token that setup will
        // exchange, so it must not first rotate an unrelated stored credential.
        let refresh_failure = if environment_auth_token().is_some() {
            None
        } else {
            refresh_if_needed(&mut state, &home).err().map(|error| error.to_string())
        };
        apply_runtime_overrides(&mut state, None, cli.org_id.as_deref());
        if let Some(org) = org {
            state.org_id = Some(org.clone());
        }
        setup(&mut state, &home, cli.json, org.is_some() || cli.org_id.is_none(), refresh_failure.as_deref())?;
        return Ok(());
    }

    if let Some(command) = &cli.command
        && print_service_verbs(command, cli.json)?
    {
        return Ok(());
    }

    validate_runtime_environment_auth(&state)?;
    refresh_if_needed(&mut state, &home)?;
    apply_runtime_overrides(&mut state, None, cli.org_id.as_deref());
    resolve_org_if_missing(&mut state, &home)?;
    let Some(command) = cli.command else {
        let client = client(&state)?;
        let identity = client.me()?;
        let services = client.services()?;
        if cli.json {
            print_json(&serde_json::json!({
                "identity": identity,
                "org": state.org_id,
                "services": services
            }))?;
        } else {
            println!("{} ({:?})", identity.name, identity.kind);
            println!("org: {}", state.org_id.as_deref().unwrap_or("not selected"));
            if services.is_empty() {
                println!("services: not advertised for this credential");
            } else {
                println!("services: {}", services.join(", "));
            }
        }
        return Ok(());
    };

    state.remember(arguments);
    let previous_session_id = state.last_session_id.clone();
    let result = client(&state).and_then(|client| dispatch(&client, command, cli.json, &mut state, &home));
    let new_session_id =
        if state.last_session_id != previous_session_id { state.last_session_id.clone() } else { None };
    let saved = State::record_activity(
        &home,
        state.last_command.clone().unwrap_or_else(|| "[unknown command]".into()),
        new_session_id,
    );
    result?;
    if let Err(error) = saved {
        eprintln!("warning: local activity history could not be saved: {error}");
    }
    Ok(())
}

fn login_status(backend: &str, json: bool, org_override: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let root = default_home();
    let home = state::home_for_backend(&root, backend)?;
    let mut state = State::load(&home)?;
    if let Some(org) = org_override {
        state.org_id = Some(org.to_owned());
    }
    let authenticated = match state.token() {
        Some(token) => client_with_token(&state, &token).and_then(|client| Ok(client.me()?)).is_ok(),
        None => false,
    };
    if json {
        print_json(&serde_json::json!({"authenticated": authenticated}))?;
    } else {
        println!("{}", if authenticated { "authenticated" } else { "not authenticated" });
    }
    Ok(())
}

fn apply_runtime_overrides(state: &mut State, backend: Option<&str>, org_id: Option<&str>) {
    if let Some(backend) = backend {
        state.backend_url = backend.to_owned();
    }
    if let Some(org_id) = org_id {
        state.org_id = Some(org_id.to_owned());
    }
}

fn environment_auth_token() -> Option<String> {
    std::env::var("SB_AUTHTOKEN").ok().filter(|token| !token.trim().is_empty())
}

fn validate_runtime_environment_auth(state: &State) -> Result<(), Box<dyn std::error::Error>> {
    let Some(token) = environment_auth_token() else {
        return Ok(());
    };
    if token.starts_with("oat_") {
        return Ok(());
    }
    if token.starts_with("oac_") && state.stored_token().is_some() {
        // An exported single-use setup token must never shadow the OAT obtained
        // from it. State::token deliberately ignores this known family.
        return Ok(());
    }
    if token.starts_with("oac_") {
        return Err(
            "SB_AUTHTOKEN contains an IAM oac_ short-lived token; run `sb setup` to exchange it and select an organization"
                .into(),
        );
    }
    Err(
        "SB_AUTHTOKEN must be an IAM oat_ access token, or an oac_ short-lived token exchanged with `sb setup`"
            .into(),
    )
}

fn resolve_org_if_missing(state: &mut State, home: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    if state.org_id.is_some() {
        return Ok(());
    }
    let using_environment_access_token = State::has_environment_access_token();
    let expected_stored_token = state.stored_token().map(str::to_owned);
    let token = state.token().ok_or("not signed in: set SB_AUTHTOKEN or run `sb setup`")?;
    let org_id = sole_bound_org(Client::new(state.backend_url.clone(), Auth::new(token)?)?.orgs()?)?;
    state.org_id = Some(org_id.clone());

    // Persist only a selection derived from the same stored credential. An
    // environment override may belong to a different user and remains process-local.
    if !using_environment_access_token {
        State::update(home, move |latest| {
            if latest.org_id.is_none() && latest.stored_token() == expected_stored_token.as_deref() {
                latest.org_id = Some(org_id);
            }
        })?;
    }
    Ok(())
}

fn sole_bound_org(orgs: Vec<Org>) -> Result<String, Box<dyn std::error::Error>> {
    match orgs.as_slice() {
        [] => Err("this token has no accessible organization; run `sb setup` with an IAM token authorized for an organization".into()),
        [org] => Ok(org.id.clone()),
        _ => Err("this token can access multiple organizations; pass --org-id <id> explicitly".into()),
    }
}

fn print_service_verbs(command: &Command, json: bool) -> Result<bool, serde_json::Error> {
    let verbs: Option<&[&str]> = match command {
        Command::Profile(Service { command: None }) => Some(&["ls", "show", "new", "set", "end"]),
        Command::Proxy(Service { command: None }) => Some(&["ls"]),
        Command::Session(Service { command: None }) => Some(&["new", "ls", "show", "live", "logs", "sync", "end"]),
        Command::Recording(Service { command: None }) => Some(&["ls", "show", "send", "rm"]),
        Command::Usage(Service { command: None }) => Some(&["ls", "show"]),
        _ => None,
    };
    let Some(verbs) = verbs else {
        return Ok(false);
    };
    if json {
        print_json(&serde_json::json!({ "verbs": verbs }))?;
    } else {
        println!("verbs: {}", verbs.join(", "));
    }
    Ok(true)
}

fn setup(
    state: &mut State,
    home: &std::path::Path,
    json: bool,
    persist_org: bool,
    refresh_failure: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let explicit_env = environment_auth_token();
    let environment_kind = explicit_env.as_deref().map(setup_environment_token_kind).transpose()?;
    let can_replace_auth = explicit_env.is_none() && io::stdin().is_terminal();
    let mut exchanged = false;
    if environment_kind == Some(SetupEnvironmentTokenKind::ShortLived) {
        let org_id = state.org_id.clone();
        exchange_setup_token_value(state, home, explicit_env.expect("classified token exists"), org_id)?;
        exchanged = true;
    }
    if !exchanged
        && setup_needs_initial_exchange(
            environment_kind == Some(SetupEnvironmentTokenKind::Access),
            state.stored_token().is_some(),
            refresh_failure.is_some(),
        )
    {
        if !io::stdin().is_terminal() {
            if let Some(error) = refresh_failure {
                return Err(format!(
                    "stored authentication could not be refreshed: {error}; rerun `sb setup` interactively with a new IAM short-lived token"
                )
                .into());
            }
            return Err(
                "no auth token: set SB_AUTHTOKEN to an oat_ access token, or run `sb setup` with an IAM oac_ short-lived token"
                    .into(),
            );
        }
        exchange_setup_token(state, home)?;
        exchanged = true;
    }

    if state.org_id.is_none() {
        let auth = Auth::new(state.token().ok_or("no auth token")?)?;
        let client = Client::new(state.backend_url.clone(), auth)?;
        match client.orgs() {
            Ok(orgs) => state.org_id = Some(select_org(orgs)?),
            Err(_) if can_replace_auth && !exchanged => {
                exchange_setup_token(state, home)?;
                exchanged = true;
            }
            Err(error) => return Err(error.into()),
        }
    }

    // Possessing an OAT is not readiness: prove that this exact token can act in the selected
    // organization before installing anything or printing success.
    let mut setup_client = client_for_setup(state, exchanged)?;
    let identity = match validate_setup_client(&setup_client) {
        Ok(identity) => identity,
        Err(_) if can_replace_auth && !exchanged => {
            exchange_setup_token(state, home)?;
            setup_client = client_for_setup(state, true)?;
            validate_setup_client(&setup_client)?
        }
        Err(error) => return Err(error.into()),
    };
    let services = setup_client.services()?;
    let delivery = setup_delivery_authorization(&setup_client, &services)?;
    state.identity_id = Some(identity.id.clone());
    state.services.clone_from(&services);
    let selected_org = state.org_id.clone();
    let persisted_services = services.clone();
    // An environment OAT belongs only to this invocation. Its validated identity and org
    // must never be attached to a different stored access/refresh credential.
    if environment_kind != Some(SetupEnvironmentTokenKind::Access) {
        State::update(home, move |latest| {
            latest.identity_id = Some(identity.id);
            latest.services = persisted_services;
            if persist_org {
                latest.org_id = selected_org;
            }
        })?;
    }

    let mut setup_events = Vec::new();
    let controller_directory = default_home().join("bin");
    state::secure_home(&controller_directory, true)?;
    let controller_lock = state::open_lock_file(&controller_directory.join("setup.lock"))?;
    fs2::FileExt::lock_exclusive(&controller_lock)?;
    let mut report_setup_event = |event| {
        if !json {
            eprintln!("{}", setup_event(&event));
        }
        setup_events.push(format!("{event:?}"));
    };
    let status = if let Some(binary) = std::env::var_os("SB_CONTROLLER_BIN") {
        report_setup_event(silicon_browser::setup::SetupEvent::Checking);
        let status = silicon_browser::setup::runner_status(binary);
        if !status.ready {
            return Err(format!(
                "SB_CONTROLLER_BIN must point to browser controller {}",
                silicon_browser::setup::AGENT_BROWSER_VERSION
            )
            .into());
        }
        report_setup_event(silicon_browser::setup::SetupEvent::Ready(status.clone()));
        status
    } else {
        silicon_browser::setup::ensure_runner(&controller_directory, report_setup_event)?
    };
    if json {
        print_json(&serde_json::json!({
            "ready": status.ready,
            "runner_version": status.version,
            "org": state.org_id,
            "services": services,
            "events": setup_events,
            "recording_delivery": delivery
        }))?;
    } else {
        println!(
            "ready: @{} in {}",
            state.identity_id.as_deref().unwrap_or("authenticated"),
            state.org_id.as_deref().unwrap()
        );
    }
    Ok(())
}

fn setup_delivery_authorization(
    client: &Client,
    services: &[String],
) -> Result<Option<DeliveryAuthorization>, Box<dyn std::error::Error>> {
    if !services.iter().any(|service| service == "recording_delivery") {
        return Ok(None);
    }
    let current = client.delivery_authorization()?;
    if current.enabled
        && matches!(current.state, DeliveryAuthorizationState::Active | DeliveryAuthorizationState::Refreshing)
    {
        return Ok(Some(current));
    }
    if !current.configured || current.state == DeliveryAuthorizationState::Unavailable {
        return Err("recording delivery is unavailable on this backend; its configuration must be completed before setup is ready".into());
    }
    if matches!(current.state, DeliveryAuthorizationState::Pending | DeliveryAuthorizationState::Revoking) {
        return Err("recording authorization is being updated; retry `sb setup` after it finishes".into());
    }
    let token = match std::env::var("SB_RECORDING_SLT").ok().filter(|value| !value.trim().is_empty()) {
        Some(token) => token,
        None if io::stdin().is_terminal() => rpassword::prompt_password(
            "Fresh IAM oac_ token for background recording delivery (separate from CLI login): ",
        )?,
        None => return Err("recording delivery needs a separate fresh Browser IAM oac_ token; set SB_RECORDING_SLT and rerun `sb setup`, or rerun interactively. Do not reuse the CLI login token.".into()),
    };
    if !token.starts_with("oac_") {
        return Err("SB_RECORDING_SLT must be a fresh IAM oac_ short-lived token for Browser".into());
    }
    if environment_auth_token().as_deref() == Some(token.as_str()) {
        return Err("recording delivery requires a different fresh oac_ token from SB_AUTHTOKEN; the CLI login token cannot be reused".into());
    }
    let enrolled = client.authorize_delivery(&DeliveryAuthorizationRequest { short_lived_token: token })?;
    if !enrolled.enabled
        || !matches!(enrolled.state, DeliveryAuthorizationState::Active | DeliveryAuthorizationState::Refreshing)
    {
        return Err(format!("recording delivery authorization is {:?}; setup is not ready", enrolled.state).into());
    }
    Ok(Some(enrolled))
}

fn setup_needs_initial_exchange(has_environment_token: bool, has_stored_token: bool, refresh_failed: bool) -> bool {
    !has_environment_token && (!has_stored_token || refresh_failed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupEnvironmentTokenKind {
    ShortLived,
    Access,
}

fn setup_environment_token_kind(token: &str) -> Result<SetupEnvironmentTokenKind, Box<dyn std::error::Error>> {
    if token.starts_with("oac_") {
        Ok(SetupEnvironmentTokenKind::ShortLived)
    } else if token.starts_with("oat_") {
        Ok(SetupEnvironmentTokenKind::Access)
    } else {
        Err("SB_AUTHTOKEN must use IAM's oac_ short-lived-token or oat_ access-token form".into())
    }
}

fn client_for_setup(state: &State, prefer_stored_token: bool) -> Result<Client, Box<dyn std::error::Error>> {
    if !prefer_stored_token {
        return client(state);
    }
    let token = state.stored_token().ok_or("IAM exchange did not produce a stored access token")?;
    client_with_token(state, token)
}

fn exchange_setup_token(state: &mut State, home: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let org_id = state.org_id.clone();
    let short_lived_token =
        rpassword::prompt_password("IAM oac_ short-lived token (never your password or long-lived credential): ")?;
    exchange_setup_token_value(state, home, short_lived_token, org_id)
}

fn exchange_setup_token_value(
    state: &mut State,
    home: &std::path::Path,
    short_lived_token: String,
    org_id: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth = Client::exchange(state.backend_url.clone(), &AuthExchangeRequest { short_lived_token, org_id })?;
    apply_auth_session(state, home, auth)
}

fn validate_setup_client(client: &Client) -> Result<Identity, silicon_browser::Error> {
    client.me()
}

fn apply_auth_session(
    state: &mut State,
    home: &std::path::Path,
    auth: AuthSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let backend_url = state.backend_url.clone();
    let credential_generation = uuid::Uuid::new_v4().to_string();
    state.credential_generation = Some(credential_generation.clone());
    let access_token = auth.access_token;
    let refresh_token = auth.refresh_token;
    let expires_at = auth.expires_at.to_rfc3339();
    let identity_id = auth.identity.id;
    let org_id = auth.org.id;
    let services = auth.services;

    state.set_tokens(access_token.clone(), Some(refresh_token.clone()), Some(expires_at.clone()));
    state.identity_id = Some(identity_id.clone());
    state.org_id = Some(org_id.clone());
    state.services.clone_from(&services);
    State::update(home, move |latest| {
        latest.set_tokens(access_token, Some(refresh_token), Some(expires_at));
        latest.identity_id = Some(identity_id);
        latest.credential_generation = Some(credential_generation);
        latest.services = services;
        // Stored credentials and their issuer/organization are one indivisible binding.
        // Runtime overrides normally stay ephemeral, but setup has replaced the identity.
        latest.backend_url = backend_url;
        latest.org_id = Some(org_id);
    })?;
    Ok(())
}

fn dispatch(
    client: &Client,
    command: Command,
    json: bool,
    state: &mut State,
    home: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Command::Iam | Command::Login { .. } => unreachable!(),
        Command::Setup { .. } => unreachable!(),
        Command::Profile(service) => profile(client, service.command, json)?,
        Command::Proxy(service) => match service.command {
            None => println!("verbs: ls"),
            Some(ProxyCommand::Ls) => output(&client.proxy_locations()?, json, |locations| {
                for location in locations {
                    println!("{}\t{}", location.code, location.name);
                }
            })?,
        },
        Command::Session(service) => session(client, service.command, json, state, home)?,
        Command::Run(args) => {
            let stdout = io::stdout();
            let stderr = io::stderr();
            let mut stdout = stdout.lock();
            let mut stderr = stderr.lock();
            let mut output_error = None;
            let runtime = runtime::Runtime::new(home, state)?;
            let result = runtime.run(client, &args.session_id, &args.command, &args.flags, |event| {
                if output_error.is_none()
                    && let Err(error) = write_run_event(&mut stdout, &mut stderr, event, json)
                {
                    output_error = Some(error);
                }
            });
            if let Some(error) = output_error {
                return Err(error.into());
            }
            let result = result?.result;
            if !result.succeeded() {
                return Err(Box::new(RunnerExit(result.exit_code)));
            }
        }
        Command::Recording(service) => recording(client, service.command, json)?,
        Command::Usage(service) => usage(client, service.command, json)?,
        Command::Search(args) => {
            let request = SearchRequest {
                query: args.query,
                purpose: args.purpose,
                search_type: args.search_type.into(),
                include_domains: normalize_list(args.include_domains),
                exclude_domains: normalize_list(args.exclude_domains),
                location: args.location,
                language: args.language,
                recency_minutes: args.recency_minutes,
                after: args.after,
                before: args.before,
                pub_year_min: args.pub_year_min,
                pub_year_max: args.pub_year_max,
                page: args.page,
            };
            output(&client.search(&request)?, json, |response| {
                for result in &response.results {
                    println!("{}\t{}\t{}", result.rank, result.title, result.url);
                }
            })?;
        }
        Command::Fetch(args) => {
            let request = FetchRequest {
                urls: parse_urls(args.urls)?,
                purpose: args.purpose,
                format: args.format.into(),
                links: args.links,
                image_links: args.image_links,
                ttl_seconds: args.ttl_seconds,
                timeout_ms: args.timeout_ms,
                include_selectors: normalize_list(args.include_selectors),
                exclude_selectors: normalize_list(args.exclude_selectors),
            };
            output(&client.fetch(&request)?, json, |response| {
                for item in &response.items {
                    if let Some(content) = &item.content {
                        if response.items.len() > 1 {
                            println!("--- {} ---", item.url);
                        }
                        println!("{content}");
                    } else if let Some(error) = &item.error {
                        eprintln!("{}: {}", item.url, error.message);
                    }
                }
            })?;
        }
    }
    Ok(())
}

fn write_run_event(stdout: &mut impl Write, stderr: &mut impl Write, event: &RunEvent, json: bool) -> io::Result<()> {
    if json {
        serde_json::to_writer(&mut *stdout, event).map_err(io::Error::other)?;
        writeln!(stdout)?;
    } else {
        match event {
            RunEvent::Stdout { chunk } => write!(stdout, "{chunk}")?,
            RunEvent::Stderr { chunk } => write!(stderr, "{chunk}")?,
            RunEvent::Warning { message } => writeln!(stderr, "warning: {message}")?,
            _ => {}
        }
    }
    stdout.flush()?;
    stderr.flush()
}

fn profile(client: &Client, command: Option<ProfileCommand>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        None => println!("verbs: ls, show, new, set, end"),
        Some(ProfileCommand::Ls) => output(&client.profiles(None)?, json, |profiles| {
            for profile in profiles {
                println!("{}\t{}\t{}", profile.name, profile.id, profile.location.code);
            }
        })?,
        Some(ProfileCommand::Show { profile_id }) => output(&client.profile(&profile_id)?, json, print_profile)?,
        Some(ProfileCommand::New { name, location, access }) => {
            let made = client.create_profile(&ProfileCreate { name, location, access: parse_access(&access)? })?;
            output(&made, json, |profile| println!("{}", profile.id))?;
        }
        Some(ProfileCommand::Set { profile_id, name, access }) => {
            let request = ProfileUpdate { name, access: access.as_deref().map(parse_access).transpose()? };
            output(&client.update_profile(&profile_id, &request)?, json, print_profile)?;
        }
        Some(ProfileCommand::End { profile_id, note }) => {
            output(&client.end_profile(&profile_id, &ProfileEnd { note })?, json, print_profile)?;
        }
    }
    Ok(())
}

fn session(
    client: &Client,
    command: Option<SessionCommand>,
    json: bool,
    state: &mut State,
    home: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        None => println!("verbs: new, ls, show, live, logs, sync, end"),
        Some(SessionCommand::New { profile_id, incognito, name, description, ttl }) => {
            let ttl = match (incognito, ttl) {
                (true, None) => SessionTtl::Minutes15,
                (false, None) => return Err("--ttl is required for a profile session".into()),
                (_, Some(ttl)) => ttl,
            };
            let request = SessionCreate { profile_id, incognito, name, description, ttl };
            let made = client.create_session(&request)?;
            state.last_session_id = Some(made.id.clone());
            output(&made, json, |session| println!("{}", session.id))?;
        }
        Some(SessionCommand::Ls { filter }) => output(&client.sessions(filter.as_deref())?, json, |sessions| {
            for session in sessions {
                println!("{}", session.id);
            }
        })?,
        Some(SessionCommand::Show { session_id }) => output(&client.session(&session_id)?, json, print_session)?,
        Some(SessionCommand::Live { session_id }) => {
            output(&client.live(&session_id)?, json, |live| println!("{}", live.url))?
        }
        Some(SessionCommand::Logs { session_id, date }) => {
            output(&client.session_logs(&session_id, date.as_deref())?, json, |logs| {
                for log in logs {
                    println!("{}", replay_command(&session_id, &log.command));
                }
            })?
        }
        Some(SessionCommand::Sync { session_id }) => {
            let sent = runtime::Runtime::new(home, state)?.sync(client, Some(&session_id), 128)?;
            output(&serde_json::json!({"reports_delivered":sent}), json, |_| {
                println!("delivered {sent} command logs")
            })?;
        }
        Some(SessionCommand::End { session_id, note }) => {
            let runtime = runtime::Runtime::new(home, state)?;
            if runtime.sync(client, Some(&session_id), 128).is_err() {
                eprintln!(
                    "warning: some command logs remain queued locally; ending the session may close their upload window"
                );
            }
            let ended = client.end_session(&session_id, &SessionEnd { note })?;
            runtime.forget_connection(&session_id)?;
            output(&ended, json, print_session)?;
        }
    }
    Ok(())
}

fn recording(client: &Client, command: Option<RecordingCommand>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        None => println!("verbs: ls, show, send, rm"),
        Some(RecordingCommand::Ls { filter }) => output(&client.recordings(filter.as_deref())?, json, |recordings| {
            for recording in recordings {
                println!("{}\t{}", recording.session_name, recording.session_id);
            }
        })?,
        Some(RecordingCommand::Show { session_id }) => output(&client.recording(&session_id)?, json, print_recording)?,
        Some(RecordingCommand::Send { session_id }) => {
            output(&client.retry_recording_delivery(&session_id)?, json, print_recording)?
        }
        Some(RecordingCommand::Rm { session_id }) => {
            output(&client.trash_recording(&session_id)?, json, print_recording)?
        }
    }
    Ok(())
}

fn usage(client: &Client, command: Option<UsageCommand>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        None => println!("verbs: ls, show, limits"),
        Some(UsageCommand::Limits) => output(&client.usage_limits()?, json, |limits| {
            println!("concurrent browser limit: {}", limits.concurrent_browser_limit);
            println!("scope: shared service account");
            println!("checked: {}", limits.checked_at.to_rfc3339());
        })?,
        Some(UsageCommand::Ls { filter }) => output(&client.usage_list(filter.as_deref())?, json, |items| {
            for item in items {
                println!(
                    "{}\t{:.2}m\t{} {}",
                    item.session_id,
                    item.browser_seconds as f64 / 60.0,
                    money_total(&item.cost),
                    item.cost.total.currency
                );
            }
        })?,
        Some(UsageCommand::Show { session_id: Some(id), org: false, .. }) => {
            output(&client.usage(&id)?, json, print_usage)?
        }
        Some(UsageCommand::Show { session_id: None, org: true, filter }) => {
            output(&client.org_usage(filter.as_deref())?, json, |total| {
                println!("scope: organization");
                println!("organization sessions: {}", total.sessions);
                println!("browser minutes: {:.2}", total.browser_minutes());
                println!("proxy GB in/out: {:.6}/{:.6}", total.proxy_gb_in(), total.proxy_gb_out());
                if total.proxy_bytes_unclassified > 0 {
                    println!("proxy GB unclassified: {:.6}", total.proxy_gb_unclassified());
                }
                println!("cost: {} {}", money_total(&total.cost), total.cost.total.currency);
            })?
        }
        Some(UsageCommand::Show { .. }) => return Err("pass a session id or --org".into()),
    }
    Ok(())
}

fn client(state: &State) -> Result<Client, Box<dyn std::error::Error>> {
    let token = state.token().ok_or("not signed in: set SB_AUTHTOKEN or run `sb setup`")?;
    client_with_token(state, &token)
}

fn client_with_token(state: &State, token: &str) -> Result<Client, Box<dyn std::error::Error>> {
    let org = state.org_id.clone().ok_or("no organization selected: run `sb setup` or pass --org-id")?;
    let mut transport = silicon_browser::HttpTransport::default();
    if !State::has_environment_access_token() && state.stored_token() == Some(token) {
        let home = state::home_for_backend(&default_home(), &state.backend_url)?;
        let backend = state.backend_url.clone();
        let bound_org = org.clone();
        let identity = state.identity_id.clone();
        transport = transport.with_unauthorized_recovery(move |request| {
            recover_rejected_access(&home, &backend, &bound_org, identity.as_deref(), request).map_err(|error| {
                silicon_browser::Error::Local(format!(
                    "access was rejected and automatic refresh could not complete: {error}"
                ))
            })
        });
    }
    Ok(Client::with_transport(state.backend_url.clone(), Auth::new(token)?, std::sync::Arc::new(transport))?
        .org(org)?)
}

fn recover_rejected_access(
    home: &std::path::Path,
    backend: &str,
    org: &str,
    identity: Option<&str>,
    request: &silicon_browser::Request,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let _guard = refresh_lock(home)?;
    let current = State::load(home)?;
    let matches_binding = |state: &State| {
        identity.is_some()
            && state.identity_id.as_deref() == identity
            && state.backend_url == backend
            && state.org_id.as_deref() == Some(org)
    };
    if !matches_binding(&current) || request.org.as_deref() != Some(org) {
        return Err(
            "stored identity, backend, or organization changed; run the command again with the intended identity"
                .into(),
        );
    }
    if current.stored_token() != request.bearer.as_deref() {
        // A public identity string cannot distinguish another refresh from a concurrent
        // setup with a different IAM principal. Never transfer queued intent to that token.
        return Err("credentials changed after the rejected request; run the command again".into());
    }
    let refresh =
        current.refresh_token().ok_or("no owned refresh token; run `sb setup` with a fresh Browser token")?.to_owned();
    let session = Client::refresh(backend, &AuthRefreshRequest { refresh_token: refresh.clone(), org_id: org.into() })?;
    if Some(session.identity.id.as_str()) != identity {
        return Err("refresh returned a different identity; sign in again".into());
    }
    let mut applied = false;
    let latest = State::update(home, |latest| {
        if matches_binding(latest) && latest.refresh_token() == Some(refresh.as_str()) {
            latest.set_tokens(session.access_token, Some(session.refresh_token), Some(session.expires_at.to_rfc3339()));
            latest.services = session.services;
            applied = true;
        }
    })?;
    if !applied {
        return Err("credentials changed during refresh; run the command again".into());
    }
    Ok(latest.stored_token().map(str::to_owned))
}

fn refresh_if_needed(state: &mut State, home: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    if State::has_environment_access_token() || !needs_refresh(state) {
        return Ok(());
    }
    let _guard = refresh_lock(home)?;
    let current = State::load(home)?;
    if !needs_refresh(&current) {
        *state = current;
        return Ok(());
    }
    let refresh_token = current
        .refresh_token()
        .ok_or("the auth token expired and has no refresh token; run `sb setup` with a new IAM short-lived token")?
        .to_owned();
    let org_id = current.org_id.clone().ok_or("the auth token expired without an organization; run `sb setup`")?;
    // Exactly one attempt while holding the process-wide refresh lock. An ambiguous transport
    // failure is deliberately not retried because IAM refresh tokens rotate on use.
    let session = Client::refresh(
        current.backend_url.clone(),
        &AuthRefreshRequest { refresh_token: refresh_token.clone(), org_id },
    )?;
    let mut applied = false;
    let latest = State::update(home, |latest| {
        // A concurrent setup may have replaced the credential while this HTTP request was in
        // flight. Never overwrite that newer credential with this response.
        if latest.refresh_token() == Some(refresh_token.as_str()) {
            latest.set_tokens(session.access_token, Some(session.refresh_token), Some(session.expires_at.to_rfc3339()));
            latest.identity_id = Some(session.identity.id);
            latest.org_id = Some(session.org.id);
            latest.services = session.services;
            applied = true;
        }
    })?;
    if !applied && needs_refresh(&latest) {
        return Err("credentials changed while refreshing; retry the command".into());
    }
    *state = latest;
    Ok(())
}

fn needs_refresh(state: &State) -> bool {
    if state.stored_token().is_none() && state.refresh_token().is_none() {
        return false;
    }
    state.stored_token().is_none()
        || state
            .token_expires_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .is_none_or(|expires| expires <= chrono::Utc::now() + chrono::Duration::seconds(60))
}

fn output<T: serde::Serialize>(value: &T, json: bool, text: impl FnOnce(&T)) -> Result<(), serde_json::Error> {
    if json {
        print_json(value)
    } else {
        text(value);
        Ok(())
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), serde_json::Error> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_profile(profile: &Profile) {
    println!("name: {}", profile.name);
    println!("id: {}", profile.id);
    println!("fingerprint: {}", profile.fingerprint);
    println!("location: {}", profile.location.code);
    println!("access: {}", profile.access.iter().collect::<Vec<_>>().join(", "));
    println!("owner: @{}", profile.owner_id);
    println!("sessions: {}", profile.sessions_run);
    println!("status: {:?}", profile.status);
    println!("created: {}", profile.created_at.to_rfc3339());
}

fn print_session(session: &Session) {
    println!("id: {}", session.id);
    println!("profile: {}", session.profile_id.as_deref().unwrap_or("incognito"));
    println!("location: {}", session.location.as_ref().map_or("none", |location| location.code.as_str()));
    println!("name: {}", session.name);
    println!("description: {}", session.description);
    println!("status: {:?}", session.status);
    println!("started: {}", session.started_at.to_rfc3339());
    println!("ttl left: {}s", session.ttl_left_seconds_at(chrono::Utc::now()));
    println!("cost: {} {}", money_total(&session.usage.cost), session.usage.cost.total.currency);
}

fn replay_command(session_id: &str, command: &str) -> String {
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
    format!("sb run {} {}", quote(session_id), quote(command))
}

fn print_recording(recording: &Recording) {
    println!("session: {}", recording.session_id);
    println!("profile: {}", recording.profile_id.as_deref().unwrap_or("incognito"));
    println!("name: {}", recording.session_name);
    println!("description: {}", recording.session_description);
    println!("owner: {}", recording.owner_id);
    println!("actors: {}", recording.participant_ids.join(", "));
    println!("status: {:?}", recording.status);
    println!(
        "briefcase: {}",
        recording.briefcase_link.as_deref().unwrap_or(match recording.status {
            RecordingStatus::Pending => "pending OBO storage",
            RecordingStatus::Available => "link unavailable",
            RecordingStatus::Trashed => "trashed",
            RecordingStatus::Failed => "delivery failed",
        })
    );
    if let Some(link) = &recording.command_log_link {
        println!("command log: {link}");
    }
    if let Some(error) = &recording.delivery_error {
        println!("delivery error: {error}");
    }
    println!("duration: {}s", recording.duration_seconds);
    println!("size: {} bytes", recording.size_bytes);
}

fn print_usage(usage: &Usage) {
    println!("browser minutes: {:.2}", usage.browser_seconds as f64 / 60.0);
    println!("proxy GB in: {:.6}", usage.proxy_bytes_in as f64 / 1_000_000_000.0);
    println!("proxy GB out: {:.6}", usage.proxy_bytes_out as f64 / 1_000_000_000.0);
    if usage.proxy_bytes_unclassified > 0 {
        println!(
            "proxy GB unclassified: {:.6} (provider does not report direction)",
            usage.proxy_bytes_unclassified as f64 / 1_000_000_000.0
        );
    }
    println!("browser cost: {} {}", money(&usage.cost.browser), usage.cost.browser.currency);
    println!("proxy in cost: {} {}", money(&usage.cost.proxy_in), usage.cost.proxy_in.currency);
    println!("proxy out cost: {} {}", money(&usage.cost.proxy_out), usage.cost.proxy_out.currency);
    if usage.cost.proxy_unclassified.micros > 0 {
        println!(
            "proxy unclassified cost: {} {}",
            money(&usage.cost.proxy_unclassified),
            usage.cost.proxy_unclassified.currency
        );
    }
    println!("total cost: {} {}", money_total(&usage.cost), usage.cost.total.currency);
}

fn money_total(cost: &UsageCost) -> String {
    money(&cost.total)
}

fn money(value: &Money) -> String {
    format!("{:.6}", value.micros as f64 / 1_000_000.0)
}

fn parse_access(value: &str) -> Result<AccessList, ValidationError> {
    let value = value.trim().strip_prefix('[').and_then(|value| value.strip_suffix(']')).unwrap_or(value.trim());
    if value.trim().is_empty() {
        return AccessList::new::<[&str; 0], &str>([]);
    }
    AccessList::new(value.split(',').map(str::trim))
}

fn parse_urls(values: Vec<String>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let urls: Vec<String> = values
        .into_iter()
        .flat_map(|value| {
            let trimmed = value.trim();
            let trimmed = trimmed.strip_prefix('[').and_then(|list| list.strip_suffix(']')).unwrap_or(trimmed);
            // Commas are legal URL characters. Only a comma introducing another absolute
            // HTTP URL separates entries; paired list brackets must not eat an IPv6 suffix.
            let mut start = 0;
            let mut urls = Vec::new();
            for (index, _) in trimmed.match_indices(',') {
                let next = trimmed[index + 1..].trim_start();
                if next.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
                    || next.get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
                {
                    urls.push(trimmed[start..index].trim().to_owned());
                    start = index + 1;
                }
            }
            if !trimmed[start..].trim().is_empty() {
                urls.push(trimmed[start..].trim().to_owned());
            }
            urls
        })
        .collect();
    if urls.is_empty() { Err("at least one URL is required".into()) } else { Ok(urls) }
}

fn normalize_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().trim_start_matches('[').trim_end_matches(']').trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

fn cli_date(value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%d-%m-%Y").map_err(|_| "expected a valid DD-MM-YYYY date".into())
}

fn prompt(label: &str) -> io::Result<String> {
    eprint!("{label}: ");
    io::stderr().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{label} is required")))
    } else {
        Ok(value)
    }
}

fn select_org(orgs: Vec<Org>) -> Result<String, Box<dyn std::error::Error>> {
    match orgs.as_slice() {
        [] => Err("this token has no accessible organization; mint an IAM short-lived token authorized for an organization".into()),
        [org] => Ok(org.id.clone()),
        many if io::stdin().is_terminal() => {
            for (index, org) in many.iter().enumerate() {
                eprintln!("{}: {} ({})", index + 1, org.name, org.id);
            }
            let chosen =
                prompt("organization number")?.parse::<usize>().map_err(|_| "organization must be a number")?;
            many.get(chosen.saturating_sub(1))
                .map(|org| org.id.clone())
                .ok_or_else(|| "organization number is out of range".into())
        }
        _ => Err("more than one organization is available; rerun setup with --org <id>".into()),
    }
}

fn setup_event(event: &silicon_browser::setup::SetupEvent) -> &'static str {
    match event {
        silicon_browser::setup::SetupEvent::Checking => "checking the local browser controller",
        silicon_browser::setup::SetupEvent::InstallingCli => "installing the local browser controller",
        silicon_browser::setup::SetupEvent::Ready(_) => "the local browser controller is ready",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use silicon_browser::{Request, Response, Transport};

    use super::*;

    #[derive(Default)]
    struct FlushWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for FlushWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct SetupMeTransport(Mutex<Vec<Request>>);

    impl Transport for SetupMeTransport {
        fn send(&self, request: Request) -> Result<Response, silicon_browser::Error> {
            let allowed = request.org.as_deref() == Some("org-allowed") && request.url.ends_with("/api/v1/me");
            self.0.lock().unwrap().push(request);
            if allowed {
                Ok(Response {
                    status: 200,
                    body: serde_json::to_vec(&Envelope::new(Identity {
                        id: "public-silicon".into(),
                        name: "Silicon".into(),
                        kind: IdentityKind::Silicon,
                        tags: vec![],
                        verified_aliases: Vec::new(),
                    }))
                    .unwrap(),
                })
            } else {
                Ok(Response {
                    status: 403,
                    body: serde_json::to_vec(&ApiErrorEnvelope::new("org_mismatch", "token cannot access that org"))
                        .unwrap(),
                })
            }
        }
    }

    #[test]
    fn rejected_request_does_not_adopt_concurrent_setup_with_the_same_public_name() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("state");
        State::update(&home, |state| {
            state.backend_url = "http://127.0.0.1:1".into();
            state.org_id = Some("org".into());
            state.identity_id = Some("reused-name".into());
            state.set_tokens(
                "oat_other_principal".into(),
                Some("ort_other_principal".into()),
                Some("2099-01-01T00:00:00Z".into()),
            );
        })
        .unwrap();
        let request = silicon_browser::Request {
            method: silicon_browser::Method::Post,
            url: "http://127.0.0.1:1/api/v1/sessions".into(),
            bearer: Some("oat_old_principal".into()),
            org: Some("org".into()),
            body: None,
        };
        let error =
            recover_rejected_access(&home, "http://127.0.0.1:1", "org", Some("reused-name"), &request).unwrap_err();
        assert!(error.to_string().contains("credentials changed after"));
        assert_eq!(State::load(&home).unwrap().stored_token(), Some("oat_other_principal"));
    }

    /// Test group: access arguments accept both documented brackets and plain comma lists.
    #[test]
    fn access_list_cli_shape_is_normalized() {
        assert_eq!(parse_access("[@alice,@bot:tos,growth]").unwrap().as_slice(), ["@alice", "@bot:tos", "growth"]);
        assert_eq!(parse_access("@alice,growth").unwrap().as_slice(), ["@alice", "growth"]);
        assert!(parse_access("[@alice,]").is_err());
    }

    /// Test group: fetch accepts bracketed, comma-separated, and repeated URL inputs in order.
    #[test]
    fn fetch_urls_preserve_cli_order() {
        let urls = parse_urls(vec!["[https://a.test,https://b.test]".into(), "https://c.test".into()]).unwrap();
        assert_eq!(urls, ["https://a.test", "https://b.test", "https://c.test"]);
    }

    /// Test group: all documented top-level grammar branches parse.
    /// Test group: URL-list convenience preserves legal URL punctuation and IPv6 hosts.
    #[test]
    fn fetch_urls_preserve_commas_and_ipv6_literals() {
        let urls = parse_urls(vec![
            "https://example.test/a,b?fields=name,id".into(),
            "http://[::1]".into(),
            "[http://[::1], https://example.test/a,b]".into(),
            "https://a.test,HTTPS://b.test".into(),
        ])
        .unwrap();
        assert_eq!(
            urls,
            [
                "https://example.test/a,b?fields=name,id",
                "http://[::1]",
                "http://[::1]",
                "https://example.test/a,b",
                "https://a.test",
                "HTTPS://b.test",
            ]
        );
    }

    #[test]
    fn documented_commands_parse() {
        for args in [
            vec!["sb", "profile", "ls"],
            vec!["sb", "proxy", "ls"],
            vec!["sb", "session", "new", "p1", "--name", "n", "--description", "d", "--ttl", "30m"],
            vec!["sb", "session", "new", "--incognito", "--name", "n", "--description", "d"],
            vec!["sb", "session", "live", "s1"],
            vec!["sb", "session", "logs", "s1", "--date", "04-09-2026"],
            vec![
                "sb",
                "recording",
                "ls",
                "--filter",
                "profile:p1 -> for:@silicon-1 -> name:market* -> description:^research -> is:shared",
            ],
            vec!["sb", "recording", "rm", "s1"],
            vec!["sb", "usage", "show", "--org"],
            vec!["sb", "search", "query", "--purpose", "reason"],
            vec!["sb", "fetch", "https://example.com", "--purpose", "reason"],
            vec!["sb", "run", "s1", "fill @e1 'hello world'", "--json"],
        ] {
            Cli::try_parse_from(args).unwrap();
        }
    }

    /// Test group: flags after the run command belong byte-for-byte to agent-browser, while
    /// Silicon Browser's JSON mode remains available before the subcommand.
    #[test]
    fn run_flags_are_not_consumed_as_global_cli_flags() {
        let cli = Cli::try_parse_from(["sb", "run", "s1", "snapshot", "--json", "--full-page"]).unwrap();
        assert!(!cli.json);
        let Some(Command::Run(args)) = cli.command else {
            panic!("expected run arguments");
        };
        assert_eq!(args.session_id, "s1");
        assert_eq!(args.command, "snapshot");
        assert_eq!(args.flags, ["--json", "--full-page"]);

        let cli = Cli::try_parse_from(["sb", "--json", "run", "s1", "snapshot"]).unwrap();
        assert!(cli.json);
        let Some(Command::Run(args)) = cli.command else {
            panic!("expected run arguments");
        };
        assert!(args.flags.is_empty());
    }

    /// Test group: every streamed event flushes both terminal streams before the callback returns.
    #[test]
    fn run_events_are_flushed_immediately() {
        let mut stdout = FlushWriter::default();
        let mut stderr = FlushWriter::default();
        write_run_event(&mut stdout, &mut stderr, &RunEvent::Stdout { chunk: "ready".into() }, false).unwrap();
        assert_eq!(stdout.bytes, b"ready");
        assert_eq!(stdout.flushes, 1);
        assert_eq!(stderr.flushes, 1);
    }

    /// Test group: `sb run` preserves ordinary agent-browser process status
    /// codes while invalid/out-of-range statuses remain a generic failure.
    #[test]
    fn runner_exit_status_is_preserved() {
        assert_eq!(error_exit_code(&RunnerExit(7)), 7);
        assert_eq!(error_exit_code(&RunnerExit(255)), 255);
        assert_eq!(error_exit_code(&RunnerExit(0)), 1);
        assert_eq!(error_exit_code(&RunnerExit(256)), 1);
    }

    /// Test group: exchanging under setup overrides stores the issuer and org with the
    /// rotating credential so the next process can refresh against the correct endpoint.
    #[test]
    fn exchanged_credentials_persist_their_backend_and_org_binding() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("sb");
        let mut state = State::update(&home, |state| {
            state.backend_url = "https://old.example".into();
            state.org_id = Some("old-org".into());
        })
        .unwrap();
        apply_runtime_overrides(&mut state, Some("https://new.example"), Some("new-org"));
        let auth = serde_json::from_value(serde_json::json!({
            "access_token": "oat_new", "refresh_token": "ort_new",
            "expires_at": "2030-03-17T17:46:40Z",
            "identity": {"id":"silicon-1","name":"Silicon","kind":"silicon"},
            "org": {"id":"new-org","name":"New"}, "services": ["session"]
        }))
        .unwrap();
        apply_auth_session(&mut state, &home, auth).unwrap();
        let saved = State::load(&home).unwrap();
        assert_eq!(saved.backend_url, "https://new.example");
        assert_eq!(saved.org_id.as_deref(), Some("new-org"));
        assert_eq!(saved.stored_token(), Some("oat_new"));
        assert_eq!(saved.refresh_token(), Some("ort_new"));
    }

    /// Test group: endpoint and org overrides only affect the in-memory invocation snapshot.
    #[test]
    fn runtime_overrides_do_not_change_persisted_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("sb");
        State::update(&home, |state| {
            state.backend_url = "https://stored.example".into();
            state.org_id = Some("stored-org".into());
        })
        .unwrap();
        let mut runtime = State::load(&home).unwrap();
        apply_runtime_overrides(&mut runtime, Some("https://once.example"), Some("once-org"));
        State::record_activity(&home, "profile ls".into(), None).unwrap();
        let persisted = State::load(&home).unwrap();
        assert_eq!(runtime.backend_url, "https://once.example");
        assert_eq!(runtime.org_id.as_deref(), Some("once-org"));
        assert_eq!(persisted.backend_url, "https://stored.example");
        assert_eq!(persisted.org_id.as_deref(), Some("stored-org"));
    }

    /// Test group: missing or malformed stored expiry is treated as unsafe whenever refresh state
    /// exists, rather than silently using an access token forever.
    #[test]
    fn malformed_or_missing_expiry_requires_refresh() {
        let mut state = State::default();
        state.set_tokens("oat_old".into(), Some("ort_old".into()), None);
        assert!(needs_refresh(&state));
        state.token_expires_at = Some("not-a-timestamp".into());
        assert!(needs_refresh(&state));
        state.token_expires_at = Some((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
        assert!(!needs_refresh(&state));
    }

    /// Test group: setup remains the credential-recovery path after a stored refresh fails, while
    /// an explicit environment credential is always left authoritative for that invocation.
    #[test]
    fn setup_replaces_missing_or_unrefreshable_stored_auth() {
        assert!(setup_needs_initial_exchange(false, false, false));
        assert!(setup_needs_initial_exchange(false, true, true));
        assert!(!setup_needs_initial_exchange(false, true, false));
        assert!(!setup_needs_initial_exchange(true, false, true));
    }

    /// Test group: setup distinguishes the two IAM token families locally so
    /// an unknown credential is never tried as either bearer or exchange input.
    #[test]
    fn setup_environment_token_families_are_explicit() {
        assert_eq!(setup_environment_token_kind("oac_single-use").unwrap(), SetupEnvironmentTokenKind::ShortLived);
        assert_eq!(setup_environment_token_kind("oat_access").unwrap(), SetupEnvironmentTokenKind::Access);
        assert!(setup_environment_token_kind("sat_not-supported-here").is_err());
        assert!(setup_environment_token_kind("unknown").is_err());
    }

    /// Test group: implicit organization resolution is deterministic and never
    /// guesses when an access token is unbound or has multiple choices.
    #[test]
    fn automatic_org_resolution_requires_exactly_one_org() {
        assert_eq!(sole_bound_org(vec![Org { id: "tos".into(), name: "TOS".into() }]).unwrap(), "tos");
        assert!(sole_bound_org(Vec::new()).unwrap_err().to_string().contains("no accessible organization"));
        assert!(
            sole_bound_org(vec![
                Org { id: "one".into(), name: "One".into() },
                Org { id: "two".into(), name: "Two".into() },
            ])
            .unwrap_err()
            .to_string()
            .contains("multiple organizations")
        );
    }

    /// Test group: offline managed-run guidance retains the session placeholder
    /// and useful command categories independently of local runner state.
    #[test]
    fn managed_run_fallback_is_actionable() {
        assert!(MANAGED_RUN_HELP.contains("sb run {sessionid}"));
        assert!(MANAGED_RUN_HELP.contains("navigate"));
        assert!(MANAGED_RUN_HELP.contains("interact"));
        assert!(MANAGED_RUN_HELP.contains("sb setup"));
    }

    /// Test group: setup readiness comes from a real scoped `me` request, not from merely finding
    /// an OAT string; an invalid org is rejected without contacting external providers.
    #[test]
    fn setup_identity_validation_is_scoped_to_the_selected_org() {
        let transport = Arc::new(SetupMeTransport::default());
        let allowed =
            Client::with_transport("https://backend.example", Auth::new("oat_test").unwrap(), transport.clone())
                .unwrap()
                .org("org-allowed")
                .unwrap();
        assert_eq!(validate_setup_client(&allowed).unwrap().id, "public-silicon");

        let denied =
            Client::with_transport("https://backend.example", Auth::new("oat_test").unwrap(), transport.clone())
                .unwrap()
                .org("org-denied")
                .unwrap();
        let error = validate_setup_client(&denied).unwrap_err();
        assert!(error.to_string().contains("token cannot access that org"));

        let requests = transport.0.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].org.as_deref(), Some("org-allowed"));
        assert_eq!(requests[1].org.as_deref(), Some("org-denied"));
        assert!(requests.iter().all(|request| request.bearer.as_deref() == Some("oat_test")));
    }

    /// Test group: service discovery is local and preserves the compact documented verbs.
    #[test]
    fn service_verbs_do_not_need_authentication() {
        let cli = Cli::try_parse_from(["sb", "session"]).unwrap();
        assert!(print_service_verbs(cli.command.as_ref().unwrap(), false).unwrap());
        let cli = Cli::try_parse_from(["sb", "search", "q", "--purpose", "p"]).unwrap();
        assert!(!print_service_verbs(cli.command.as_ref().unwrap(), false).unwrap());
    }
}
