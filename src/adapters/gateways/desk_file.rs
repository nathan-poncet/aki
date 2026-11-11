//! The desk on disk: `spaces.toml`, beside the configuration. Written by
//! the board each time the layout changes, and readable enough to edit by
//! hand.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::entities::desk::{Desk, Grouping, Layout, SpaceDraft};
use crate::entities::filter::Filter;
use crate::use_cases::ports::{DeskError, DeskStore};

const HEADER: &str = "\
# Your spaces. myturn rewrites this file when you rearrange the board, and
# reads it at start; edit it by hand if you like.
#
#   scope / filter   is:todo|waiting  source:  repo:  kind:  top:  role:  by:  words
#                    values: a,b (any)   negation: -repo:api
#                    kind: any reason of the task   top: its most pressing one
#   layout           columns | rows | master | grid
#   group            none | repo | kind | source | person | role
";

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeskFile {
    #[serde(default)]
    spaces: Vec<SpaceFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceFile {
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    scope: String,
    #[serde(default = "columns")]
    layout: String,
    #[serde(default)]
    panes: Vec<PaneFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaneFile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(default)]
    filter: String,
    #[serde(default = "none")]
    group: String,
}

fn columns() -> String {
    Layout::Columns.name().to_string()
}

fn none() -> String {
    Grouping::None.name().to_string()
}

pub fn parse(text: &str) -> Result<Desk, String> {
    let file: DeskFile = toml::from_str(text).map_err(|error| error.to_string())?;
    let mut drafts = Vec::new();
    for space in file.spaces {
        let place = format!("space \"{}\"", space.name);
        let filter =
            |query: &str| Filter::parse(query).map_err(|error| format!("{place}: {error}"));
        let mut panes = Vec::new();
        for pane in &space.panes {
            let grouping = Grouping::from_name(&pane.group).ok_or_else(|| {
                format!(
                    "{place}: unknown group \"{}\" (expected: {})",
                    pane.group,
                    Grouping::ALL.map(Grouping::name).join(", ")
                )
            })?;
            panes.push((pane.title.clone(), filter(&pane.filter)?, grouping));
        }
        drafts.push(SpaceDraft {
            scope: filter(&space.scope)?,
            layout: Layout::from_name(&space.layout).ok_or_else(|| {
                format!(
                    "{place}: unknown layout \"{}\" (expected: {})",
                    space.layout,
                    Layout::ALL.map(Layout::name).join(", ")
                )
            })?,
            name: space.name,
            panes,
        });
    }
    Ok(Desk::new(drafts))
}

pub fn render(desk: &Desk) -> String {
    let file = DeskFile {
        spaces: desk
            .spaces()
            .iter()
            .map(|space| SpaceFile {
                name: space.name.clone(),
                scope: space.scope.query().to_string(),
                layout: space.layout.name().to_string(),
                panes: space
                    .panes
                    .iter()
                    .map(|pane| PaneFile {
                        title: pane.title.clone(),
                        filter: pane.filter.query().to_string(),
                        group: pane.grouping.name().to_string(),
                    })
                    .collect(),
            })
            .collect(),
    };
    let body = toml::to_string(&file).expect("a desk always serializes");
    format!("{HEADER}\n{body}")
}

pub struct DeskFileStore {
    pub path: PathBuf,
}

impl DeskStore for DeskFileStore {
    fn load(&self) -> Result<Option<Desk>, DeskError> {
        let place = self.path.display();
        match std::fs::read_to_string(&self.path) {
            Ok(text) => parse(&text)
                .map(Some)
                .map_err(|error| DeskError::Unreadable(format!("{place}: {error}"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(DeskError::Unreadable(format!("{place}: {error}"))),
        }
    }

    fn save(&self, desk: &Desk) -> Result<(), DeskError> {
        // Written aside then renamed: a crash never leaves half a desk.
        let aside = self.path.with_extension("toml.tmp");
        self.path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&aside, render(desk)))
            .and_then(|()| std::fs::rename(&aside, &self.path))
            .map_err(|error| DeskError::Unwritable(format!("{}: {error}", self.path.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::desk::Pane;

    #[test]
    fn a_desk_survives_the_file() {
        let mut desk = Desk::starting(&["gitlab".into(), "github".into()]);
        desk.show(1);
        desk.space_mut().name = "Pro".to_string();
        desk.space_mut().layout = Layout::Master;
        desk.split_pane();
        let pane = desk.pane_mut();
        pane.title = "Reviews".to_string();
        pane.filter = Filter::parse("is:todo kind:review -repo:api").unwrap();
        pane.grouping = Grouping::Repo;

        let text = render(&desk);
        assert!(text.starts_with("# Your spaces."));
        assert!(text.contains("[[spaces.panes]]"));
        // Pane ids and what is shown are not part of the file; all the rest is.
        let back = parse(&text).unwrap();
        assert_eq!(render(&back), text);
        let pro = &back.spaces()[1];
        assert_eq!((pro.name.as_str(), pro.layout), ("Pro", Layout::Master));
        assert_eq!(
            pro.panes[1],
            Pane {
                id: pro.panes[1].id,
                ..desk.space().pane().clone()
            }
        );
    }

    #[test]
    fn a_hand_written_file_needs_very_little() {
        let desk = parse(
            r#"
            [[spaces]]
            name = "Pro"
            scope = "source:gitlab,jira"

            [[spaces]]
            name = "Perso"
            layout = "rows"

            [[spaces.panes]]
            filter = "is:todo"
            group = "repo"
            "#,
        )
        .unwrap();
        let pro = &desk.spaces()[0];
        assert_eq!(pro.layout, Layout::Columns);
        assert_eq!(pro.panes.len(), 2, "no pane written: the two usual ones");
        let perso = &desk.spaces()[1];
        assert_eq!(perso.panes.len(), 1);
        assert_eq!(perso.panes[0].grouping, Grouping::Repo);
    }

    #[test]
    fn mistakes_name_their_space() {
        let error = parse("[[spaces]]\nname = \"Pro\"\nscope = \"repos:web\"").unwrap_err();
        assert!(
            error.starts_with("space \"Pro\": unknown filter"),
            "{error}"
        );
        let error = parse("[[spaces]]\nname = \"Pro\"\nlayout = \"spiral\"").unwrap_err();
        assert!(error.contains("unknown layout \"spiral\""), "{error}");
        assert!(parse("[[spaces]]\nname = \"Pro\"\ncolour = 1").is_err());
    }
}
