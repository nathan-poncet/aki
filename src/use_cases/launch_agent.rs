//! Opens a coding agent on a task: in the right place, in the right
//! directory, already told what there is to do.

use super::ports::{Checkouts, LaunchError, Launched, Launcher, OpenSession, Session};
use crate::entities::handoff::{AgentSettings, AgentState};
use crate::entities::task::Task;

pub struct LaunchAgent<'a> {
    pub settings: &'a AgentSettings,
    pub checkouts: &'a dyn Checkouts,
    pub launcher: &'a dyn Launcher,
}

impl LaunchAgent<'_> {
    /// What would be opened, without opening it. `brief` is what the agent
    /// is told first.
    pub fn plan(&self, task: &Task, brief: &str) -> Session {
        let placement = self.settings.placement(task);
        Session {
            workspace: placement.workspace,
            tab: placement.tab,
            // Without a local clone the agent starts where the launcher
            // does, and the brief still carries the link.
            directory: self.checkouts.find(&task.item),
            command: self.settings.command_line(brief),
            prompt: brief.to_string(),
            focus: self.settings.focus,
        }
    }

    /// The terminals the launcher has open.
    pub fn sessions(&self) -> Vec<OpenSession> {
        // Not being able to tell is the same as nothing open.
        self.launcher.sessions().unwrap_or_default()
    }

    /// What the agent opened on `task` is doing: the state of the terminal
    /// that bears exactly the name this task would be given.
    pub fn state_of(&self, task: &Task, sessions: &[OpenSession]) -> Option<AgentState> {
        let placement = self.settings.placement(task);
        sessions
            .iter()
            .find(|open| open.workspace == placement.workspace && open.tab == placement.tab)
            .map(|open| open.state)
    }

    pub fn run(&self, task: &Task, brief: &str) -> Result<(Session, Launched), LaunchError> {
        let session = self.plan(task, brief);
        let launched = self.launcher.launch(&session)?;
        Ok((session, launched))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::item::ItemRef;
    use crate::entities::task::{Kind, samples};
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct Clones(Option<&'static str>);

    impl Checkouts for Clones {
        fn find(&self, item: &ItemRef) -> Option<PathBuf> {
            self.0.map(|root| PathBuf::from(root).join(&item.container))
        }
    }

    #[derive(Default)]
    struct Desk {
        opened: RefCell<Vec<Session>>,
        refuses: bool,
    }

    impl Launcher for Desk {
        fn launch(&self, session: &Session) -> Result<Launched, LaunchError> {
            if self.refuses {
                return Err(LaunchError::Failed("no server".into()));
            }
            let mut opened = self.opened.borrow_mut();
            let already = opened
                .iter()
                .any(|open| open.workspace == session.workspace && open.tab == session.tab);
            opened.push(session.clone());
            Ok(if already {
                Launched::Briefed
            } else {
                Launched::Opened
            })
        }
    }

    fn review() -> Task {
        samples::task(
            "gitlab",
            "acme/web",
            "!916",
            Kind::ReviewRequested,
            Some("alex"),
        )
    }

    #[test]
    fn an_agent_opens_in_the_clone_named_after_the_work_and_briefed() {
        let (settings, desk) = (AgentSettings::default(), Desk::default());
        let launch = LaunchAgent {
            settings: &settings,
            checkouts: &Clones(Some("/code")),
            launcher: &desk,
        };
        let (session, launched) = launch.run(&review(), "Review acme/web!916").unwrap();

        assert_eq!(launched, Launched::Opened);
        assert_eq!(
            (session.workspace.as_str(), session.tab.as_str()),
            ("web", "!916")
        );
        assert_eq!(session.directory, Some(PathBuf::from("/code/acme/web")));
        assert_eq!(session.command, ["claude", "Review acme/web!916"]);
        assert_eq!(session.prompt, "Review acme/web!916");
        assert!(session.focus);
        assert_eq!(desk.opened.borrow().len(), 1);

        // Asked again, the launcher says it was there already.
        let (_, again) = launch.run(&review(), "Review acme/web!916").unwrap();
        assert_eq!(again, Launched::Briefed);
    }

    #[test]
    fn a_task_knows_its_agent_by_the_name_it_would_be_given() {
        let (settings, desk) = (AgentSettings::default(), Desk::default());
        let launch = LaunchAgent {
            settings: &settings,
            checkouts: &Clones(None),
            launcher: &desk,
        };
        let open = |workspace: &str, tab: &str, state| OpenSession {
            workspace: workspace.into(),
            tab: tab.into(),
            state,
        };
        let sessions = [
            open("web", "!915", AgentState::Working),
            open("api", "!916", AgentState::Idle),
            open("web", "!916", AgentState::Blocked),
        ];
        assert_eq!(
            launch.state_of(&review(), &sessions),
            Some(AgentState::Blocked)
        );
        assert_eq!(launch.state_of(&review(), &sessions[..2]), None);
        // This launcher cannot tell what is open.
        assert!(launch.sessions().is_empty());
    }

    #[test]
    fn without_a_clone_the_agent_still_opens() {
        let (settings, desk) = (AgentSettings::default(), Desk::default());
        let launch = LaunchAgent {
            settings: &settings,
            checkouts: &Clones(None),
            launcher: &desk,
        };
        let (session, _) = launch.run(&review(), "brief").unwrap();
        assert_eq!(session.directory, None);
    }

    #[test]
    fn a_plan_opens_nothing_and_a_refusal_is_reported() {
        let settings = AgentSettings::default();
        let desk = Desk {
            refuses: true,
            ..Desk::default()
        };
        let launch = LaunchAgent {
            settings: &settings,
            checkouts: &Clones(None),
            launcher: &desk,
        };
        assert_eq!(launch.plan(&review(), "brief").tab, "!916");
        assert!(desk.opened.borrow().is_empty());
        assert_eq!(
            launch.run(&review(), "brief"),
            Err(LaunchError::Failed("no server".into()))
        );
    }
}
