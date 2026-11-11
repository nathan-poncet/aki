//! How the user lays their work out: a desk of spaces (pro, perso…), each
//! a set of panes, each pane a filtered, grouped view of the board.

use super::board::Board;
use super::filter::Filter;
use super::task::{Bucket, Kind, Role, Task};

/// How a pane packs its tasks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grouping {
    /// One flat list, most pressing first.
    #[default]
    None,
    Repo,
    /// By what there is to do: reviews, replies, merges…
    Kind,
    Source,
    /// By who is on the other side.
    Person,
    Role,
}

impl Grouping {
    pub const ALL: [Grouping; 6] = [
        Grouping::None,
        Grouping::Repo,
        Grouping::Kind,
        Grouping::Source,
        Grouping::Person,
        Grouping::Role,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Grouping::None => "none",
            Grouping::Repo => "repo",
            Grouping::Kind => "kind",
            Grouping::Source => "source",
            Grouping::Person => "person",
            Grouping::Role => "role",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|grouping| grouping.name() == name)
    }

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|other| *other == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

/// How a space tiles its panes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Side by side.
    #[default]
    Columns,
    /// One above the other.
    Rows,
    /// The first pane large on the left, the others stacked on the right.
    Master,
    /// As square a grid as the count allows.
    Grid,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::Columns, Layout::Rows, Layout::Master, Layout::Grid];

    pub fn name(self) -> &'static str {
        match self {
            Layout::Columns => "columns",
            Layout::Rows => "rows",
            Layout::Master => "master",
            Layout::Grid => "grid",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.name() == name)
    }

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|other| *other == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    /// Stable while the desk lives; lets an interface remember where the
    /// cursor was in a pane that moved.
    pub id: u32,
    /// Empty: named after its filter.
    pub title: String,
    pub filter: Filter,
    pub grouping: Grouping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Space {
    pub name: String,
    /// What belongs to this space at all; every pane filters further.
    pub scope: Filter,
    pub layout: Layout,
    pub panes: Vec<Pane>,
    /// The pane that has the keyboard.
    pub focus: usize,
}

/// What a group is a group of. Presenters choose the words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupKey {
    All,
    Repo(String),
    Kind(Kind),
    Source(String),
    Person(Option<String>),
    Role(Role),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<'a> {
    pub key: GroupKey,
    pub tasks: Vec<&'a Task>,
}

fn side(bucket: Bucket) -> Filter {
    Filter::all().with_bucket(Some(bucket))
}

impl Space {
    /// What a pane of this space shows: the board through the space's scope
    /// and the pane's filter, packed by the pane's grouping. Groups come in
    /// the order of their most pressing task.
    pub fn groups<'a>(&self, pane: &Pane, board: &'a Board) -> Vec<Group<'a>> {
        let mut groups: Vec<Group<'a>> = Vec::new();
        let tasks = board
            .todo
            .iter()
            .chain(&board.waiting)
            .filter(|task| self.scope.matches(task) && pane.filter.matches(task));
        for task in tasks {
            let key = match pane.grouping {
                Grouping::None => GroupKey::All,
                Grouping::Repo => GroupKey::Repo(task.item.container.clone()),
                Grouping::Kind => GroupKey::Kind(task.headline().kind),
                Grouping::Source => GroupKey::Source(task.item.source.clone()),
                Grouping::Person => GroupKey::Person(task.counterpart.clone()),
                Grouping::Role => GroupKey::Role(task.role),
            };
            match groups.iter_mut().find(|group| group.key == key) {
                Some(group) => group.tasks.push(task),
                None => groups.push(Group {
                    key,
                    tasks: vec![task],
                }),
            }
        }
        groups
    }

    /// How many things in this space wait on me.
    pub fn todo_count(&self, board: &Board) -> usize {
        board
            .todo
            .iter()
            .filter(|task| self.scope.matches(task))
            .count()
    }

    pub fn pane(&self) -> &Pane {
        &self.panes[self.focus]
    }
}

/// The spaces, and which one is shown. Every operation keeps the desk
/// usable: at least one space, at least one pane in each, focus in range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desk {
    spaces: Vec<Space>,
    current: usize,
    next_id: u32,
}

/// A space as written down, before the desk adopts it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpaceDraft {
    pub name: String,
    pub scope: Filter,
    pub layout: Layout,
    /// (title, filter, grouping)
    pub panes: Vec<(String, Filter, Grouping)>,
}

impl Desk {
    pub fn new(drafts: Vec<SpaceDraft>) -> Self {
        let mut desk = Self {
            spaces: Vec::new(),
            current: 0,
            next_id: 0,
        };
        for draft in drafts {
            desk.adopt(draft);
        }
        if desk.spaces.is_empty() {
            desk.adopt(SpaceDraft {
                name: "All".to_string(),
                ..SpaceDraft::default()
            });
        }
        desk
    }

    /// The desk of a first run: everything, then one space per source when
    /// there are several, each with what is mine beside what waits.
    pub fn starting(sources: &[String]) -> Self {
        let mut drafts = vec![SpaceDraft {
            name: "All".to_string(),
            ..SpaceDraft::default()
        }];
        if sources.len() > 1 {
            for source in sources {
                let mut name = source.clone();
                if let Some(first) = name.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                drafts.push(SpaceDraft {
                    name,
                    // A source name with a space or a colon cannot be scoped; keep all.
                    scope: Filter::parse(&format!("source:{source}")).unwrap_or_default(),
                    ..SpaceDraft::default()
                });
            }
        }
        Self::new(drafts)
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn adopt(&mut self, draft: SpaceDraft) {
        let mut panes = draft.panes;
        if panes.is_empty() {
            panes = vec![
                (String::new(), side(Bucket::Todo), Grouping::None),
                (String::new(), side(Bucket::Waiting), Grouping::None),
            ];
        }
        let panes = panes
            .into_iter()
            .map(|(title, filter, grouping)| Pane {
                id: self.id(),
                title,
                filter,
                grouping,
            })
            .collect();
        self.spaces.push(Space {
            name: draft.name,
            scope: draft.scope,
            layout: draft.layout,
            panes,
            focus: 0,
        });
    }

    pub fn spaces(&self) -> &[Space] {
        &self.spaces
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn space(&self) -> &Space {
        &self.spaces[self.current]
    }

    pub fn space_mut(&mut self) -> &mut Space {
        &mut self.spaces[self.current]
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        let space = self.space_mut();
        &mut space.panes[space.focus]
    }

    /// Shows a space; false when there is none at that index.
    pub fn show(&mut self, index: usize) -> bool {
        let exists = index < self.spaces.len();
        if exists {
            self.current = index;
        }
        exists
    }

    /// Shows the next (`+1`) or previous (`-1`) space, wrapping around.
    pub fn cycle_space(&mut self, step: isize) {
        let count = self.spaces.len() as isize;
        self.current = (self.current as isize + step).rem_euclid(count) as usize;
    }

    /// Adds a space after the others and shows it.
    pub fn add_space(&mut self, name: &str) {
        self.adopt(SpaceDraft {
            name: name.to_string(),
            ..SpaceDraft::default()
        });
        self.current = self.spaces.len() - 1;
    }

    /// Removes the shown space; false when it is the last one.
    pub fn remove_space(&mut self) -> bool {
        if self.spaces.len() == 1 {
            return false;
        }
        self.spaces.remove(self.current);
        self.current = self.current.min(self.spaces.len() - 1);
        true
    }

    /// Moves the shown space among the others, and stays on it.
    pub fn shift_space(&mut self, step: isize) {
        let target = self.current as isize + step;
        if (0..self.spaces.len() as isize).contains(&target) {
            self.spaces.swap(self.current, target as usize);
            self.current = target as usize;
        }
    }

    /// Focuses the next (`+1`) or previous (`-1`) pane, wrapping around.
    pub fn cycle_pane(&mut self, step: isize) {
        let space = self.space_mut();
        let count = space.panes.len() as isize;
        space.focus = (space.focus as isize + step).rem_euclid(count) as usize;
    }

    pub fn focus(&mut self, pane: usize) {
        let space = self.space_mut();
        if pane < space.panes.len() {
            space.focus = pane;
        }
    }

    /// Opens a pane beside the focused one, showing the same thing, and
    /// focuses it: the quickest start for "the same, but…".
    pub fn split_pane(&mut self) {
        let id = self.id();
        let space = self.space_mut();
        let mut pane = space.pane().clone();
        pane.id = id;
        pane.title.clear();
        space.panes.insert(space.focus + 1, pane);
        space.focus += 1;
    }

    /// Closes the focused pane; false when it is the space's last one.
    pub fn close_pane(&mut self) -> bool {
        let space = self.space_mut();
        if space.panes.len() == 1 {
            return false;
        }
        space.panes.remove(space.focus);
        space.focus = space.focus.min(space.panes.len() - 1);
        true
    }

    /// Swaps the focused pane with its neighbour, and keeps it focused.
    pub fn shift_pane(&mut self, step: isize) {
        let space = self.space_mut();
        let target = space.focus as isize + step;
        if (0..space.panes.len() as isize).contains(&target) {
            space.panes.swap(space.focus, target as usize);
            space.focus = target as usize;
        }
    }

    /// Sends the focused pane to another space and follows it. False when
    /// the space does not exist, is this one, or would be left empty.
    pub fn send_pane(&mut self, to: usize) -> bool {
        if to >= self.spaces.len() || to == self.current || self.space().panes.len() == 1 {
            return false;
        }
        let from = self.space_mut();
        let pane = from.panes.remove(from.focus);
        from.focus = from.focus.min(from.panes.len() - 1);
        let target = &mut self.spaces[to];
        target.panes.push(pane);
        target.focus = target.panes.len() - 1;
        self.current = to;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::item::ItemRef;
    use crate::entities::task::{Detail, Reason};
    use crate::entities::time::Timestamp;

    fn task(source: &str, container: &str, reference: &str, kind: Kind) -> Task {
        Task {
            item: ItemRef {
                source: source.into(),
                container: container.into(),
                reference: reference.into(),
                title: "Title".into(),
                url: "https://example.test".into(),
                updated_at: Timestamp(0),
            },
            role: Role::Author,
            counterpart: None,
            reasons: vec![Reason {
                kind,
                bucket: kind.default_bucket(),
                priority: kind.default_priority(),
                detail: Detail::None,
                url: None,
            }],
            since: Timestamp(0),
        }
    }

    fn board() -> Board {
        Board::new(
            vec![
                task("gitlab", "g/api", "!1", Kind::ChecksFailed),
                task("gitlab", "g/web", "!2", Kind::ReplyNeeded),
                task("gitlab", "g/api", "!3", Kind::ReplyNeeded),
                task("github", "o/notes", "#4", Kind::ReadyToMerge),
                task("gitlab", "g/web", "!5", Kind::AwaitingReview),
            ],
            vec!["gitlab".into(), "github".into()],
            Vec::new(),
            Timestamp(0),
        )
    }

    fn references(group: &Group) -> Vec<String> {
        group
            .tasks
            .iter()
            .map(|task| task.item.reference.clone())
            .collect()
    }

    #[test]
    fn a_first_desk_has_everything_then_one_space_per_source() {
        let desk = Desk::starting(&["gitlab".into(), "github".into()]);
        let names: Vec<&str> = desk
            .spaces()
            .iter()
            .map(|space| space.name.as_str())
            .collect();
        assert_eq!(names, ["All", "Gitlab", "Github"]);
        assert_eq!(desk.spaces()[1].scope.query(), "source:gitlab");
        let panes: Vec<&str> = desk
            .space()
            .panes
            .iter()
            .map(|pane| pane.filter.query())
            .collect();
        assert_eq!(panes, ["is:todo", "is:waiting"]);

        // One source: nothing to separate.
        assert_eq!(Desk::starting(&["gitlab".into()]).spaces().len(), 1);
        // Never empty, whatever was written down.
        assert_eq!(Desk::new(Vec::new()).spaces().len(), 1);
    }

    #[test]
    fn a_pane_shows_its_space_through_its_filter_packed_by_its_grouping() {
        let mut desk = Desk::starting(&["gitlab".into(), "github".into()]);
        let board = board();

        desk.show(1);
        desk.pane_mut().grouping = Grouping::Repo;
        let space = desk.space();
        let groups = space.groups(space.pane(), &board);
        // Gitlab only, to do only; groups in the order of their most pressing task.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, GroupKey::Repo("g/api".into()));
        assert_eq!(references(&groups[0]), ["!3", "!1"]);
        assert_eq!(references(&groups[1]), ["!2"]);
        assert_eq!(space.todo_count(&board), 3);

        desk.show(0);
        desk.pane_mut().grouping = Grouping::Kind;
        let space = desk.space();
        let groups = space.groups(space.pane(), &board);
        let keys: Vec<&GroupKey> = groups.iter().map(|group| &group.key).collect();
        assert_eq!(
            keys,
            [
                &GroupKey::Kind(Kind::ReplyNeeded),
                &GroupKey::Kind(Kind::ChecksFailed),
                &GroupKey::Kind(Kind::ReadyToMerge)
            ]
        );
    }

    #[test]
    fn panes_split_move_travel_and_close() {
        let mut desk = Desk::starting(&["gitlab".into(), "github".into()]);
        let todo = desk.space().panes[0].id;

        desk.split_pane();
        assert_eq!(desk.space().panes.len(), 3);
        assert_eq!(desk.space().focus, 1);
        let copy = desk.space().pane().id;
        assert_ne!(copy, todo);
        assert_eq!(desk.space().pane().filter.query(), "is:todo");

        desk.shift_pane(1);
        assert_eq!(desk.space().focus, 2);
        assert_eq!(desk.space().pane().id, copy);
        desk.shift_pane(1); // already last: stays
        assert_eq!(desk.space().focus, 2);

        assert!(desk.send_pane(2));
        assert_eq!(desk.current(), 2);
        assert_eq!(desk.space().pane().id, copy);
        assert_eq!(desk.spaces()[0].panes.len(), 2);
        assert!(!desk.send_pane(2), "already there");
        assert!(!desk.send_pane(9), "no such space");

        assert!(desk.close_pane());
        assert!(desk.close_pane());
        assert!(!desk.close_pane(), "a space keeps one pane");
        assert!(!desk.send_pane(0), "and does not give its last one away");
    }

    #[test]
    fn spaces_are_added_shown_moved_and_removed() {
        let mut desk = Desk::starting(&["gitlab".into()]);
        desk.add_space("Perso");
        assert_eq!(desk.current(), 1);
        assert_eq!(desk.space().name, "Perso");

        desk.shift_space(-1);
        assert_eq!(desk.current(), 0);
        assert_eq!(desk.spaces()[1].name, "All");

        desk.cycle_space(-1);
        assert_eq!(desk.current(), 1);
        assert!(!desk.show(5));

        assert!(desk.remove_space());
        assert_eq!(desk.space().name, "Perso");
        assert!(!desk.remove_space(), "a desk keeps one space");
    }

    #[test]
    fn names_round_trip() {
        for grouping in Grouping::ALL {
            assert_eq!(Grouping::from_name(grouping.name()), Some(grouping));
        }
        for layout in Layout::ALL {
            assert_eq!(Layout::from_name(layout.name()), Some(layout));
        }
        assert_eq!(Layout::Grid.next(), Layout::Columns);
        assert_eq!(Grouping::Role.next(), Grouping::None);
    }
}
