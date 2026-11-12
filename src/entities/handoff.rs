//! Handing a task to a coding agent: which command starts it, and where
//! its terminal goes. Work is laid out by repository, then by what is being
//! worked on: one workspace per repository, one tab per ticket or change.

use super::task::Task;

/// What an agent opened on a task is doing, as its terminal reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Working,
    /// It asked something and waits for the user.
    Blocked,
    /// It finished a turn the user has not looked at yet.
    Done,
    Idle,
    /// The terminal is open; what runs in it is not known.
    Unknown,
}

impl AgentState {
    pub const ALL: [AgentState; 5] = [
        AgentState::Working,
        AgentState::Blocked,
        AgentState::Done,
        AgentState::Idle,
        AgentState::Unknown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Done => "done",
            AgentState::Idle => "idle",
            AgentState::Unknown => "open",
        }
    }
}

/// How agents are started, as the user set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSettings {
    /// The agent's command, word by word. `{prompt}` is where the brief
    /// goes; without it, the brief is the last argument.
    pub command: Vec<String>,
    /// The workspace's name, with placeholders.
    pub workspace: String,
    /// The tab's name, with placeholders.
    pub tab: String,
    /// Added to every brief: how the user wants their agent to work.
    pub instructions: String,
    /// Whether opening an agent takes the user to it.
    pub focus: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            command: vec!["claude".to_string(), "{prompt}".to_string()],
            workspace: "{repo}".to_string(),
            tab: "{ticket} {ref}".to_string(),
            instructions: String::new(),
            focus: true,
        }
    }
}

/// Where a task's agent lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub workspace: String,
    pub tab: String,
}

/// What a name template may say:
/// `{repo}` the repository's own name, `{path}` its full path, `{ref}` the
/// item's reference (`!916`, `#12`, `PROJ-7`), `{short}` both (`web!916`),
/// `{ticket}` the first ticket the task is about (empty when there is none,
/// or when the task is that ticket), `{kind}` the rule that lists it,
/// `{source}` where it comes from.
fn fill(template: &str, task: &Task) -> String {
    let item = &task.item;
    let repo = item.container.rsplit('/').next().unwrap_or(&item.container);
    let ticket = task
        .tickets
        .first()
        .map(|ticket| ticket.key.as_str())
        .unwrap_or("");
    let filled = template
        .replace("{repo}", repo)
        .replace("{path}", &item.container)
        .replace("{ref}", &item.reference)
        .replace("{short}", &item.short())
        .replace("{ticket}", ticket)
        .replace("{kind}", task.headline().kind.name())
        .replace("{source}", &item.source);
    // An empty placeholder leaves no hole.
    filled.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl AgentSettings {
    /// A name is never empty: a template that says nothing for this task
    /// falls back on what always exists.
    pub fn placement(&self, task: &Task) -> Placement {
        let or = |name: String, fallback: &str| {
            if name.is_empty() {
                fallback.to_string()
            } else {
                name
            }
        };
        Placement {
            workspace: or(fill(&self.workspace, task), &task.item.container),
            tab: or(fill(&self.tab, task), &task.item.reference),
        }
    }

    /// The command to run, with the brief in its place, as one argument
    /// however long it is and whatever it contains.
    pub fn command_line(&self, prompt: &str) -> Vec<String> {
        let mut placed = false;
        let mut line: Vec<String> = self
            .command
            .iter()
            .map(|word| {
                if word.contains("{prompt}") {
                    placed = true;
                    word.replace("{prompt}", prompt)
                } else {
                    word.clone()
                }
            })
            .collect();
        if !placed {
            line.push(prompt.to_string());
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::task::{Kind, samples};
    use crate::entities::ticket::LinkedTicket;

    fn review() -> Task {
        let mut task = samples::task(
            "gitlab",
            "acme/platform/web",
            "!916",
            Kind::ReviewRequested,
            Some("alex"),
        );
        task.tickets = vec![LinkedTicket {
            key: "PROJ-580".into(),
            priority: None,
        }];
        task
    }

    #[test]
    fn work_is_laid_out_by_repository_then_by_ticket() {
        let placement = AgentSettings::default().placement(&review());
        assert_eq!(placement.workspace, "web");
        assert_eq!(placement.tab, "PROJ-580 !916");
    }

    #[test]
    fn a_change_without_a_ticket_is_named_by_its_reference() {
        let mut task = review();
        task.tickets.clear();
        assert_eq!(AgentSettings::default().placement(&task).tab, "!916");
    }

    #[test]
    fn a_ticket_is_named_once() {
        let ticket = samples::task("jira", "PROJ", "PROJ-7", Kind::TicketTodo, None);
        let placement = AgentSettings::default().placement(&ticket);
        assert_eq!(
            (placement.workspace.as_str(), placement.tab.as_str()),
            ("PROJ", "PROJ-7")
        );
    }

    #[test]
    fn names_follow_the_users_templates_and_are_never_empty() {
        let settings = AgentSettings {
            workspace: "{source}/{path}".into(),
            tab: "{kind}: {short}".into(),
            ..AgentSettings::default()
        };
        let placement = settings.placement(&review());
        assert_eq!(placement.workspace, "gitlab/acme/platform/web");
        assert_eq!(placement.tab, "review_requested: web!916");

        let mut task = review();
        task.tickets.clear();
        let silent = AgentSettings {
            workspace: " ".into(),
            tab: "{ticket}".into(),
            ..AgentSettings::default()
        };
        let placement = silent.placement(&task);
        assert_eq!(placement.workspace, "acme/platform/web");
        assert_eq!(placement.tab, "!916");
    }

    #[test]
    fn the_brief_is_one_argument_wherever_it_goes() {
        let brief = "Review this.\nIt's \"urgent\" $HOME";
        assert_eq!(
            AgentSettings::default().command_line(brief),
            ["claude", brief]
        );

        let custom = AgentSettings {
            command: vec!["agent".into(), "--prompt={prompt}".into(), "--yolo".into()],
            ..AgentSettings::default()
        };
        assert_eq!(
            custom.command_line("go"),
            ["agent", "--prompt=go", "--yolo"]
        );

        let bare = AgentSettings {
            command: vec!["codex".into()],
            ..AgentSettings::default()
        };
        assert_eq!(bare.command_line("go"), ["codex", "go"]);
    }
}
