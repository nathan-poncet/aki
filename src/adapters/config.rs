//! The configuration file: TOML in, typed settings out. Unknown keys and
//! unknown names are errors, so a typo never silently disables a rule.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use super::gateways::jira::DEFAULT_JQL;
use super::gateways::session::Credential;
use super::gateways::sound::Sounds;
use crate::entities::handoff::AgentSettings;
use crate::entities::rules::{OthersDrafts, OwnDrafts, Resolver, RuleOverride, Rules};
use crate::entities::task::{Bucket, Kind, Trigger};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceConfig {
    GitLab {
        name: String,
        host: String,
        credential: Credential,
    },
    GitHub {
        name: String,
        host: String,
        credential: Credential,
    },
    Jira {
        name: String,
        site: String,
        email: String,
        jql: String,
        credential: Credential,
    },
}

impl SourceConfig {
    pub fn name(&self) -> &str {
        match self {
            SourceConfig::GitLab { name, .. }
            | SourceConfig::GitHub { name, .. }
            | SourceConfig::Jira { name, .. } => name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub rules: Rules,
    pub refresh: Duration,
    /// How old a reading may be for `list` to reuse it instead of asking again.
    pub cache: Duration,
    pub notifications: bool,
    pub sources: Vec<SourceConfig>,
    pub agent: AgentConfig,
    pub sounds: Sounds,
    pub mcp: McpConfig,
}

/// What an assistant plugged into `aki mcp` may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct McpConfig {
    /// Whether the server answers at all.
    pub enabled: bool,
    pub set_aside: bool,
    pub start_agent: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            set_aside: true,
            start_agent: true,
        }
    }
}

/// Where a new Claude conversation opens, until told otherwise.
pub const CHAT_URL: &str = "https://claude.ai/new?q={prompt}";

/// What opens an agent's terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherChoice {
    /// Herdr when aki runs inside it, else the terminal itself.
    Auto,
    Herdr,
    Terminal,
    /// No terminal at all: a new Claude conversation, started on the brief.
    Claude,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    pub settings: AgentSettings,
    pub launcher: LauncherChoice,
    /// The user's own way to open a terminal window, for terminals aki
    /// does not know.
    pub terminal: Option<Vec<String>>,
    /// A named Herdr session to open agents in.
    pub herdr_session: Option<String>,
    /// The address a new conversation opens at; `{prompt}` is the brief.
    pub chat_url: String,
    /// Where clones are looked for. `None`: the usual places.
    pub roots: Option<Vec<PathBuf>>,
    /// Repository path → its folder, for clones that are found nowhere.
    pub repos: BTreeMap<String, PathBuf>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Syntax(String),
    #[error("{place}: unknown {what} \"{name}\" (expected one of: {expected})")]
    UnknownName {
        place: String,
        what: &'static str,
        name: String,
        expected: String,
    },
    #[error("sources: a {kind} source needs `{field}`")]
    MissingField { kind: String, field: &'static str },
    #[error("cannot read {path}: {reason}")]
    Unreadable { path: String, reason: String },
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
    refresh_seconds: Option<u64>,
    notifications: Option<bool>,
    cache_seconds: Option<u64>,
    sources: Option<Vec<SourceFile>>,
    threads: ThreadsFile,
    drafts: DraftsFile,
    re_review: ReReviewFile,
    tickets: TicketsFile,
    nudge: NudgeFile,
    urgency: UrgencyFile,
    rules: BTreeMap<String, RuleFile>,
    agent: AgentFile,
    sounds: SoundsFile,
    mcp: McpFile,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct McpFile {
    enabled: Option<bool>,
    set_aside: Option<bool>,
    start_agent: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SourceFile {
    kind: String,
    name: Option<String>,
    host: Option<String>,
    site: Option<String>,
    email: Option<String>,
    jql: Option<String>,
    token_env: Option<String>,
    token_command: Option<String>,
    token_renew_command: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThreadsFile {
    who_resolves: Option<String>,
    ignore_bots: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DraftsFile {
    mine: Option<String>,
    others: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ReReviewFile {
    triggers: Option<Vec<String>>,
    after_approval: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TicketsFile {
    waiting_statuses: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct NudgeFile {
    after_days: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SoundsFile {
    enabled: Option<bool>,
    new_task: Option<String>,
    rules: BTreeMap<String, String>,
}

fn read_sounds(file: SoundsFile) -> Result<Sounds, ConfigError> {
    let mut sounds = Sounds::default();
    sounds.enabled = file.enabled.unwrap_or(sounds.enabled);
    sounds.new_task = file.new_task.unwrap_or(sounds.new_task);
    for (name, sound) in file.rules {
        let Some(kind) = Kind::from_name(&name) else {
            return unknown("sounds.rules", "rule", &name, &Kind::ALL.map(Kind::name));
        };
        sounds.rules.insert(kind, sound);
    }
    Ok(sounds)
}

/// A command written as one line, or word by word.
#[derive(Deserialize)]
#[serde(untagged)]
enum Words {
    Line(String),
    List(Vec<String>),
}

impl Words {
    fn into_words(self) -> Vec<String> {
        match self {
            Words::Line(line) => line.split_whitespace().map(str::to_string).collect(),
            Words::List(words) => words,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AgentFile {
    command: Option<Words>,
    workspace: Option<String>,
    tab: Option<String>,
    instructions: Option<String>,
    focus: Option<bool>,
    launcher: Option<String>,
    terminal: Option<Words>,
    herdr_session: Option<String>,
    chat_url: Option<String>,
    roots: Option<Vec<String>>,
    repos: BTreeMap<String, String>,
}

/// `~/code` → the home folder's `code`.
fn expand(path: &str) -> PathBuf {
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => PathBuf::from(path),
    }
}

fn read_agent(file: AgentFile) -> Result<AgentConfig, ConfigError> {
    let mut settings = AgentSettings::default();
    if let Some(command) = file.command {
        let words = command.into_words();
        if words.is_empty() {
            return Err(ConfigError::Syntax("agent.command is empty".to_string()));
        }
        settings.command = words;
    }
    settings.workspace = file.workspace.unwrap_or(settings.workspace);
    settings.tab = file.tab.unwrap_or(settings.tab);
    settings.instructions = file.instructions.unwrap_or(settings.instructions);
    settings.focus = file.focus.unwrap_or(settings.focus);
    Ok(AgentConfig {
        settings,
        launcher: match file.launcher.as_deref() {
            None | Some("auto") => LauncherChoice::Auto,
            Some("herdr") => LauncherChoice::Herdr,
            Some("terminal") => LauncherChoice::Terminal,
            Some("claude") => LauncherChoice::Claude,
            Some(other) => {
                return unknown(
                    "agent.launcher",
                    "value",
                    other,
                    &["auto", "herdr", "terminal", "claude"],
                );
            }
        },
        chat_url: file.chat_url.unwrap_or_else(|| CHAT_URL.to_string()),
        terminal: file.terminal.map(Words::into_words),
        herdr_session: file.herdr_session,
        roots: file
            .roots
            .map(|roots| roots.iter().map(|root| expand(root)).collect()),
        repos: file
            .repos
            .iter()
            .map(|(repository, folder)| (repository.clone(), expand(folder)))
            .collect(),
    })
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UrgencyFile {
    per_day: Option<u32>,
    waiting_cap: Option<u32>,
    linked: Option<i32>,
    highest: Option<i32>,
    high: Option<i32>,
    medium: Option<i32>,
    low: Option<i32>,
    lowest: Option<i32>,
    warm: Option<u32>,
    hot: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RuleFile {
    enabled: Option<bool>,
    bucket: Option<String>,
    priority: Option<u32>,
}

fn unknown<T>(
    place: &str,
    what: &'static str,
    name: &str,
    expected: &[&str],
) -> Result<T, ConfigError> {
    Err(ConfigError::UnknownName {
        place: place.to_string(),
        what,
        name: name.to_string(),
        expected: expected.join(", "),
    })
}

fn gitlab_credential(host: &str) -> Credential {
    Credential {
        env: Some("GITLAB_TOKEN".to_string()),
        command: Some(format!("glab config get token --host {host}")),
        // glab keeps a short-lived OAuth token and renews it on any API call.
        renew: Some(format!("glab api user --hostname {host}")),
    }
}

fn github_credential(host: &str) -> Credential {
    Credential {
        env: Some("GITHUB_TOKEN".to_string()),
        command: Some(format!("gh auth token --hostname {host}")),
        renew: None,
    }
}

impl Config {
    /// `installed` tells whether a CLI is on the machine; without a
    /// `[[sources]]` list, the forges whose CLI is signed in are used.
    pub fn parse(text: &str, installed: &dyn Fn(&str) -> bool) -> Result<Self, ConfigError> {
        let file: File =
            toml::from_str(text).map_err(|error| ConfigError::Syntax(error.to_string()))?;

        let mut rules = Rules::default();
        if let Some(name) = &file.threads.who_resolves {
            rules.who_resolves = match Resolver::from_name(name) {
                Some(resolver) => resolver,
                None => {
                    return unknown(
                        "threads.who_resolves",
                        "value",
                        name,
                        &["reviewer", "author", "anyone"],
                    );
                }
            };
        }
        if let Some(ignore) = file.threads.ignore_bots {
            rules.ignore_bots = ignore;
        }
        if let Some(name) = &file.drafts.others {
            rules.others_drafts = match OthersDrafts::from_name(name) {
                Some(policy) => policy,
                None => {
                    return unknown(
                        "drafts.others",
                        "value",
                        name,
                        &["review", "wait", "ignore"],
                    );
                }
            };
        }
        if let Some(name) = &file.drafts.mine {
            rules.own_drafts = match OwnDrafts::from_name(name) {
                Some(policy) => policy,
                None => return unknown("drafts.mine", "value", name, &OwnDrafts::NAMES),
            };
        }
        if let Some(names) = &file.re_review.triggers {
            let expected = Trigger::ALL.map(Trigger::name);
            rules.re_review_triggers = names
                .iter()
                .map(|name| match Trigger::from_name(name) {
                    Some(trigger) => Ok(trigger),
                    None => unknown("re_review.triggers", "trigger", name, &expected),
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(after_approval) = file.re_review.after_approval {
            rules.re_review_after_approval = after_approval;
        }
        if let Some(statuses) = file.tickets.waiting_statuses {
            rules.ticket_waiting_statuses = statuses;
        }
        if let Some(days) = file.nudge.after_days {
            rules.nudge_after_days = days;
        }
        let weights = &mut rules.urgency;
        let urgency = &file.urgency;
        weights.per_day = urgency.per_day.unwrap_or(weights.per_day);
        weights.waiting_cap = urgency.waiting_cap.unwrap_or(weights.waiting_cap);
        weights.linked = urgency.linked.unwrap_or(weights.linked);
        weights.highest = urgency.highest.unwrap_or(weights.highest);
        weights.high = urgency.high.unwrap_or(weights.high);
        weights.medium = urgency.medium.unwrap_or(weights.medium);
        weights.low = urgency.low.unwrap_or(weights.low);
        weights.lowest = urgency.lowest.unwrap_or(weights.lowest);
        weights.warm = urgency.warm.unwrap_or(weights.warm);
        weights.hot = urgency.hot.unwrap_or(weights.hot);
        for (name, rule) in &file.rules {
            let Some(kind) = Kind::from_name(name) else {
                return unknown("rules", "rule", name, &Kind::ALL.map(Kind::name));
            };
            let bucket = match &rule.bucket {
                None => None,
                Some(bucket) => match Bucket::from_name(bucket) {
                    Some(bucket) => Some(bucket),
                    None => {
                        return unknown(
                            &format!("rules.{name}.bucket"),
                            "value",
                            bucket,
                            &["todo", "waiting"],
                        );
                    }
                },
            };
            rules.overrides.insert(
                kind,
                RuleOverride {
                    enabled: rule.enabled,
                    bucket,
                    priority: rule.priority,
                },
            );
        }

        let sources = match file.sources {
            Some(sources) => sources
                .into_iter()
                .map(read_source)
                .collect::<Result<_, _>>()?,
            None => detected_sources(installed),
        };

        Ok(Self {
            rules,
            refresh: Duration::from_secs(file.refresh_seconds.unwrap_or(30).max(5)),
            cache: Duration::from_secs(file.cache_seconds.unwrap_or(60)),
            notifications: file.notifications.unwrap_or(true),
            sources,
            agent: read_agent(file.agent)?,
            sounds: read_sounds(file.sounds)?,
            mcp: {
                let default = McpConfig::default();
                McpConfig {
                    enabled: file.mcp.enabled.unwrap_or(default.enabled),
                    set_aside: file.mcp.set_aside.unwrap_or(default.set_aside),
                    start_agent: file.mcp.start_agent.unwrap_or(default.start_agent),
                }
            },
        })
    }

    pub fn load(installed: &dyn Fn(&str) -> bool) -> Result<Self, ConfigError> {
        let path = path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(ConfigError::Unreadable {
                    path: path.display().to_string(),
                    reason: error.to_string(),
                });
            }
        };
        Self::parse(&text, installed)
    }
}

fn detected_sources(installed: &dyn Fn(&str) -> bool) -> Vec<SourceConfig> {
    let mut sources = Vec::new();
    if installed("glab") {
        sources.push(SourceConfig::GitLab {
            name: "gitlab".to_string(),
            host: "gitlab.com".to_string(),
            credential: gitlab_credential("gitlab.com"),
        });
    }
    if installed("gh") {
        sources.push(SourceConfig::GitHub {
            name: "github".to_string(),
            host: "github.com".to_string(),
            credential: github_credential("github.com"),
        });
    }
    sources
}

fn read_source(source: SourceFile) -> Result<SourceConfig, ConfigError> {
    let custom =
        (source.token_env.is_some() || source.token_command.is_some()).then(|| Credential {
            env: source.token_env.clone(),
            command: source.token_command.clone(),
            renew: source.token_renew_command.clone(),
        });
    let name = source.name.unwrap_or_else(|| source.kind.clone());
    match source.kind.as_str() {
        "gitlab" => {
            let host = source.host.unwrap_or_else(|| "gitlab.com".to_string());
            Ok(SourceConfig::GitLab {
                credential: custom.unwrap_or_else(|| gitlab_credential(&host)),
                name,
                host,
            })
        }
        "github" => {
            let host = source.host.unwrap_or_else(|| "github.com".to_string());
            Ok(SourceConfig::GitHub {
                credential: custom.unwrap_or_else(|| github_credential(&host)),
                name,
                host,
            })
        }
        "jira" => {
            let missing = |field| ConfigError::MissingField {
                kind: "jira".to_string(),
                field,
            };
            Ok(SourceConfig::Jira {
                name,
                site: source.site.ok_or_else(|| missing("site"))?,
                email: source.email.ok_or_else(|| missing("email"))?,
                jql: source.jql.unwrap_or_else(|| DEFAULT_JQL.to_string()),
                credential: custom.unwrap_or(Credential {
                    env: Some("JIRA_API_TOKEN".to_string()),
                    command: None,
                    renew: None,
                }),
            })
        }
        other => unknown("sources", "kind", other, &["gitlab", "github", "jira"]),
    }
}

/// `$AKI_CONFIG`, else `$XDG_CONFIG_HOME/aki/config.toml`, else
/// `~/.config/aki/config.toml`.
pub fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("AKI_CONFIG") {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("aki").join("config.toml")
}

/// The user's spaces, beside the configuration: `spaces.toml`.
pub fn spaces_path() -> PathBuf {
    path().with_file_name("spaces.toml")
}

/// Where gateways may keep what they already read: `$XDG_CACHE_HOME/aki`,
/// else `~/.cache/aki`.
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("aki")
}

/// What the user set aside: `$XDG_STATE_HOME/aki/snoozed.json`, else
/// `~/.local/state/aki/snoozed.json`.
pub fn snoozes_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("aki")
        .join("snoozed.json")
}

/// The SQLite file of everything already read.
pub fn cache_path() -> PathBuf {
    cache_dir().join("cache.db")
}

/// What `aki config init` writes: every setting, at its default.
pub const TEMPLATE: &str = include_str!("../../config/example.toml");

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(text, &|cli| cli == "glab")
    }

    #[test]
    fn an_empty_file_is_the_defaults_with_detected_sources() {
        let config = parse("").unwrap();
        assert_eq!(config.rules, Rules::default());
        assert_eq!(config.refresh, Duration::from_secs(30));
        assert_eq!(config.cache, Duration::from_secs(60));
        assert!(config.notifications);
        assert_eq!(config.sources.len(), 1);
        assert!(
            matches!(&config.sources[0], SourceConfig::GitLab { host, .. } if host == "gitlab.com")
        );
    }

    #[test]
    fn the_template_parses_to_the_defaults() {
        let config = parse(TEMPLATE).unwrap();
        assert_eq!(config.rules, Rules::default());
        assert_eq!(config.refresh, Duration::from_secs(30));
    }

    #[test]
    fn every_rule_can_be_tuned() {
        let config = parse(
            r#"
            refresh_seconds = 60
            notifications = false

            [threads]
            who_resolves = "author"
            ignore_bots = false

            [drafts]
            mine = "apart"
            others = "ignore"

            [re_review]
            triggers = ["re_requested", "all_threads_answered"]
            after_approval = true

            [tickets]
            waiting_statuses = ["QA"]

            [nudge]
            after_days = 5

            [urgency]
            per_day = 5
            highest = 80
            hot = 150

            [rules.draft_to_finish]
            enabled = false

            [rules.checks_failed]
            bucket = "waiting"
            priority = 5
            "#,
        )
        .unwrap();

        assert_eq!(config.refresh, Duration::from_secs(60));
        assert!(!config.notifications);
        let rules = config.rules;
        assert_eq!(rules.who_resolves, Resolver::Author);
        assert!(!rules.ignore_bots);
        assert_eq!(rules.others_drafts, OthersDrafts::Ignore);
        assert_eq!(rules.own_drafts, OwnDrafts::Apart);
        assert_eq!(
            rules.re_review_triggers,
            [Trigger::ReRequested, Trigger::AllThreadsAnswered]
        );
        assert!(rules.re_review_after_approval);
        assert!(rules.is_waiting_status("qa"));
        assert_eq!(rules.nudge_after_days, 5);
        assert_eq!((rules.urgency.per_day, rules.urgency.highest), (5, 80));
        assert_eq!((rules.urgency.hot, rules.urgency.warm), (150, 100));
        assert!(!rules.rule(Kind::DraftToFinish).enabled);
        let checks = rules.rule(Kind::ChecksFailed);
        assert_eq!((checks.bucket, checks.priority), (Bucket::Waiting, 5));
    }

    #[test]
    fn sources_are_read_with_their_defaults() {
        let config = parse(
            r#"
            [[sources]]
            kind = "gitlab"
            name = "work"
            host = "gitlab.acme.io"

            [[sources]]
            kind = "jira"
            site = "https://acme.atlassian.net"
            email = "me@acme.io"
            token_command = "pass jira"
            "#,
        )
        .unwrap();

        assert!(matches!(&config.sources[0],
            SourceConfig::GitLab { name, host, credential }
            if name == "work" && host == "gitlab.acme.io"
                && credential.command.as_deref().unwrap().contains("--host gitlab.acme.io")));
        assert!(matches!(&config.sources[1],
            SourceConfig::Jira { name, jql, credential, .. }
            if name == "jira" && jql == DEFAULT_JQL
                && credential.command.as_deref() == Some("pass jira") && credential.env.is_none()));
    }

    #[test]
    fn the_agent_is_claude_in_herdr_or_the_terminal_until_told_otherwise() {
        let agent = parse("").unwrap().agent;
        assert_eq!(agent.settings, AgentSettings::default());
        assert_eq!(agent.launcher, LauncherChoice::Auto);
        assert_eq!(
            (agent.terminal, agent.roots, agent.herdr_session),
            (None, None, None)
        );
    }

    #[test]
    fn the_agent_is_set_up_in_a_few_lines() {
        let agent = parse(
            r#"
            [agent]
            command = "codex --full-auto {prompt}"
            workspace = "{source}/{repo}"
            tab = "{ref}"
            instructions = "Answer in French."
            focus = false
            launcher = "terminal"
            terminal = ["alacritty", "-e", "{command}"]
            roots = ["~/code", "/srv/git"]

            [agent.repos]
            "acme/web" = "~/elsewhere/web"
            "#,
        )
        .unwrap()
        .agent;

        assert_eq!(agent.settings.command, ["codex", "--full-auto", "{prompt}"]);
        assert_eq!(agent.settings.workspace, "{source}/{repo}");
        assert_eq!(agent.settings.tab, "{ref}");
        assert_eq!(agent.settings.instructions, "Answer in French.");
        assert!(!agent.settings.focus);
        assert_eq!(agent.launcher, LauncherChoice::Terminal);
        assert_eq!(
            agent.terminal,
            Some(vec![
                "alacritty".to_string(),
                "-e".into(),
                "{command}".into()
            ])
        );
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        assert_eq!(
            agent.roots,
            Some(vec![home.join("code"), PathBuf::from("/srv/git")])
        );
        assert_eq!(agent.repos["acme/web"], home.join("elsewhere/web"));

        // Word by word, for arguments with spaces.
        let agent = parse("[agent]\ncommand = [\"claude\", \"--model\", \"a b\"]")
            .unwrap()
            .agent;
        assert_eq!(agent.settings.command, ["claude", "--model", "a b"]);
        assert!(matches!(
            parse("[agent]\nlauncher = \"screen\""),
            Err(ConfigError::UnknownName { name, .. }) if name == "screen"
        ));
        assert!(parse("[agent]\ncommand = \"\"").is_err());
    }

    #[test]
    fn an_assistant_is_let_in_and_can_be_held_back() {
        let config = parse("").unwrap();
        assert_eq!(config.mcp, McpConfig::default());
        assert!(config.mcp.enabled && config.mcp.set_aside && config.mcp.start_agent);
        assert_eq!(config.agent.chat_url, CHAT_URL);

        let config = parse(
            r#"
            [agent]
            launcher = "claude"
            chat_url = "claude://claude.ai/new?q={prompt}"

            [mcp]
            start_agent = false
            "#,
        )
        .unwrap();
        assert_eq!(config.agent.launcher, LauncherChoice::Claude);
        assert_eq!(config.agent.chat_url, "claude://claude.ai/new?q={prompt}");
        assert!(config.mcp.set_aside && !config.mcp.start_agent);
        assert!(!parse("[mcp]\nenabled = false").unwrap().mcp.enabled);
    }

    #[test]
    fn sounds_ring_by_default_and_follow_the_users_choices() {
        let sounds = parse("").unwrap().sounds;
        assert_eq!(sounds, Sounds::default());
        assert!(sounds.enabled && !sounds.new_task.is_empty());

        let sounds = parse(
            r#"
            [sounds]
            new_task = "~/sounds/ding.wav"

            [sounds.rules]
            review_requested = "Ping"
            draft_to_finish = "none"
            "#,
        )
        .unwrap()
        .sounds;
        assert_eq!(sounds.new_task, "~/sounds/ding.wav");
        assert_eq!(sounds.rules[&Kind::ReviewRequested], "Ping");
        assert_eq!(sounds.rules[&Kind::DraftToFinish], "none");

        assert!(!parse("[sounds]\nenabled = false").unwrap().sounds.enabled);
        assert!(matches!(
            parse("[sounds.rules]\nreviews = \"Ping\""),
            Err(ConfigError::UnknownName { name, .. }) if name == "reviews"
        ));
    }

    #[test]
    fn typos_are_errors_not_silence() {
        assert!(matches!(parse("[rules.re_reviw]\nenabled = false"),
            Err(ConfigError::UnknownName { name, .. }) if name == "re_reviw"));
        assert!(matches!(parse("[re_review]\ntriggers = [\"pushed\"]"),
            Err(ConfigError::UnknownName { name, .. }) if name == "pushed"));
        assert!(matches!(
            parse("[threads]\nwho = \"me\""),
            Err(ConfigError::Syntax(_))
        ));
        assert_eq!(
            parse("[[sources]]\nkind = \"jira\"\nemail = \"a@b.c\""),
            Err(ConfigError::MissingField {
                kind: "jira".into(),
                field: "site"
            })
        );
    }
}
