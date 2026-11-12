//! Keeps the board alive: look, show, announce what is new, wait, repeat.

use super::collect_board::CollectBoard;
use super::ports::{BoardView, Notifier, Pace, Pacer};
use crate::entities::board::Board;

pub struct WatchBoard<'a> {
    pub collect: CollectBoard<'a>,
    pub notifier: &'a dyn Notifier,
    pub pacer: &'a dyn Pacer,
}

impl WatchBoard<'_> {
    pub fn run(&self, view: &dyn BoardView) {
        // What was known last time shows at once; the real reading follows.
        // It is not a reference for news: it may be days old.
        if let Some(remembered) = self.collect.recalled() {
            view.show(remembered);
        }
        let mut previous: Option<Board> = None;
        loop {
            view.refreshing();
            let board = self.collect.run();

            // The first board is the state of the world, not news.
            if let Some(previous) = &previous {
                let news = board.news_since(previous);
                if !news.is_empty() {
                    self.notifier.announce(&news);
                }
            }
            view.show(board.clone());
            previous = Some(board);

            if self.pacer.wait() == Pace::Stop {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::rules::Rules;
    use crate::use_cases::ports::{SourceError, WorkSource};
    use crate::use_cases::testing::{
        FixedClock, NoSnoozes, Recorder, ScriptedSource, Turns, review_request,
    };

    fn watch(source: ScriptedSource, turns: usize) -> Recorder {
        let sources: Vec<Box<dyn WorkSource>> = vec![Box::new(source)];
        let recorder = Recorder::default();
        WatchBoard {
            collect: CollectBoard {
                sources: &sources,
                clock: &FixedClock(1_000),
                rules: &Rules::default(),
                snoozes: &NoSnoozes,
            },
            notifier: &recorder,
            pacer: &Turns::new(turns),
        }
        .run(&recorder);
        recorder
    }

    #[test]
    fn announces_only_what_appears_after_the_first_look() {
        let source = ScriptedSource::script(
            "gitlab",
            vec![
                Ok(vec![review_request("!1")]),
                Ok(vec![review_request("!1")]),
                Ok(vec![review_request("!1"), review_request("!2")]),
            ],
        );
        let recorder = watch(source, 3);

        assert_eq!(recorder.boards(), 3);
        assert_eq!(
            recorder.announced(),
            [vec!["gitlab:group/web!2".to_string()]]
        );
    }

    #[test]
    fn the_last_known_board_shows_before_the_first_reading() {
        let source = ScriptedSource::always("gitlab", vec![review_request("!1")])
            .remembering(vec![review_request("!0")], 400);
        let recorder = watch(source, 1);

        assert_eq!(recorder.boards(), 2);
        assert_eq!(recorder.shown(0), ["gitlab:group/web!0"]);
        assert_eq!(recorder.shown(1), ["gitlab:group/web!1"]);
        assert!(recorder.announced().is_empty(), "catching up is not news");
    }

    #[test]
    fn a_source_coming_back_is_not_news() {
        let source = ScriptedSource::script(
            "gitlab",
            vec![
                Ok(vec![review_request("!1")]),
                Err(SourceError::Unreachable("timeout".into())),
                Ok(vec![review_request("!1")]),
            ],
        );
        let recorder = watch(source, 3);

        assert_eq!(recorder.boards(), 3);
        assert!(recorder.announced().is_empty());
    }
}
