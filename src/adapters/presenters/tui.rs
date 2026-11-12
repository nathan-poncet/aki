//! The live board: spaces along the top, tiled panes below, each a
//! filtered and grouped view of the same board. A background watch sends
//! boards through a channel; this draws the latest one and turns keys and
//! clicks into changes of the desk.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout as Split, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use unicode_width::UnicodeWidthStr;

use super::wording::{self, age, column, plain};
use crate::entities::board::Board;
use crate::entities::desk::{Desk, GroupKey, Grouping, Layout, Pane, Sort, Space};
use crate::entities::filter::Filter;
use crate::entities::handoff::AgentState;
use crate::entities::rules::Rules;
use crate::entities::snooze::{Snooze, Until};
use crate::entities::task::{Bucket, Task};
use crate::entities::time::Timestamp;
use crate::entities::urgency::Heat;
use crate::use_cases::ports::{BoardView, Clock, OpenSession, Pace};

pub enum Update {
    Refreshing,
    Board(Board),
    /// The terminals the launcher has open, when that changed.
    Agents(Vec<OpenSession>),
}

/// The watch's side of the channel.
pub struct ChannelView(pub Sender<Update>);

impl BoardView for ChannelView {
    fn refreshing(&self) {
        let _ = self.0.send(Update::Refreshing);
    }

    fn show(&self, board: Board) {
        let _ = self.0.send(Update::Board(board));
    }
}

/// Remembers that an item was set aside, or brought back (`None`).
pub type Remember = dyn Fn(&str, Option<Snooze>) -> Result<(), String>;

/// What the interface needs from outside.
pub struct Wiring<'a> {
    pub updates: Receiver<Update>,
    pub requests: Sender<Pace>,
    pub clock: &'a dyn Clock,
    pub open: &'a dyn Fn(&str),
    /// Keeps the desk for the next run; the error is shown, not fatal.
    pub save: &'a dyn Fn(&Desk) -> Result<(), String>,
    /// Opens the user's agent on a task; says what it did, or why not.
    pub agent: &'a dyn Fn(&Task) -> Result<String, String>,
    /// What the agent opened on a task is doing, among the open terminals.
    pub agent_of: &'a dyn Fn(&Task, &[OpenSession]) -> Option<AgentState>,
    /// Remembers that an item was set aside (`None`: brought back).
    pub snooze: &'a Remember,
    /// The user's rules, to explain what the board shows with them.
    pub rules: &'a Rules,
    /// Where the spaces are kept, to say so.
    pub kept_in: String,
}

/// What a key or a click asks of the outside.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    None,
    Quit,
    Refresh,
    Open(String),
    /// Hand this task to an agent.
    Agent(Box<Task>),
    /// Set this item aside, or bring it back.
    Postpone(String, Postpone),
}

/// How long a task is set aside for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Postpone {
    Days(u32),
    UntilItMoves,
    /// Not any more.
    BringBack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    PaneFilter,
    PaneTitle,
    SpaceName,
    NewSpace,
    SpaceScope,
}

impl Ask {
    fn label(self) -> &'static str {
        match self {
            Ask::PaneFilter => "filter",
            Ask::PaneTitle => "pane title",
            Ask::SpaceName => "space name",
            Ask::NewSpace => "new space",
            Ask::SpaceScope => "space scope",
        }
    }
}

struct Prompt {
    ask: Ask,
    text: String,
    /// What was there before, to put back on Esc.
    original: String,
    error: Option<String>,
}

enum Mode {
    Normal,
    Prompt(Prompt),
    /// Waiting for the number of the space to send the pane to.
    Send,
    /// Waiting for how long to set the selected task aside.
    Postpone,
    ConfirmRemoveSpace,
    Help,
    /// Why the selected task is here, and where its place comes from.
    Explain,
    /// How the shown space is made, and what it holds.
    AboutSpace,
}

/// Where things were drawn, for the mouse.
enum Hit {
    Space {
        area: Rect,
        index: usize,
    },
    Pane {
        area: Rect,
        index: usize,
        /// The first list line on screen, and what each line of the whole
        /// list is: a task (by position) or a group heading.
        top: u16,
        offset: usize,
        lines: Vec<Option<usize>>,
    },
}

struct Screen {
    board: Option<Board>,
    desk: Desk,
    /// Pane id → selected task, by position in the pane.
    cursors: HashMap<u32, usize>,
    mode: Mode,
    zoomed: bool,
    detail: bool,
    refreshing: bool,
    /// One line of feedback, until the next key: something refused…
    notice: Option<String>,
    /// …or something done.
    done: Option<String>,
    /// The desk changed and must be saved.
    unsaved: bool,
    hits: Vec<Hit>,
    /// The terminals open on tasks, as last heard.
    sessions: Vec<OpenSession>,
    rules: Rules,
    kept_in: String,
}

/// The tasks a pane lists, in order.
fn listing<'a>(board: Option<&'a Board>, space: &Space, pane: &Pane) -> Vec<&'a Task> {
    board
        .map(|board| {
            space
                .groups(pane, board)
                .into_iter()
                .flat_map(|group| group.tasks)
                .collect()
        })
        .unwrap_or_default()
}

impl Screen {
    fn new(desk: Desk) -> Self {
        Self {
            board: None,
            desk,
            cursors: HashMap::new(),
            mode: Mode::Normal,
            zoomed: false,
            detail: true,
            refreshing: true,
            notice: None,
            done: None,
            unsaved: false,
            hits: Vec::new(),
            sessions: Vec::new(),
            rules: Rules::default(),
            kept_in: String::new(),
        }
    }

    fn tasks(&self) -> Vec<&Task> {
        let space = self.desk.space();
        listing(self.board.as_ref(), space, space.pane())
    }

    fn cursor(&self) -> usize {
        let pane = self.desk.space().pane().id;
        self.cursors.get(&pane).copied().unwrap_or(0)
    }

    fn selected(&self) -> Option<&Task> {
        self.tasks().get(self.cursor()).copied()
    }

    /// Moves the focused pane's cursor to `to`, kept in range.
    fn select(&mut self, to: usize) {
        let last = self.tasks().len().saturating_sub(1);
        let pane = self.desk.space().pane().id;
        self.cursors.insert(pane, to.min(last));
    }

    fn step(&mut self, by: isize) {
        self.select(self.cursor().saturating_add_signed(by));
    }

    /// Takes a fresh board, keeping every pane's cursor on the same item
    /// when it is still there.
    fn receive(&mut self, board: Board) {
        let mut followed: Vec<(u32, String)> = Vec::new();
        for space in self.desk.spaces() {
            for pane in &space.panes {
                let cursor = self.cursors.get(&pane.id).copied().unwrap_or(0);
                if let Some(task) = listing(self.board.as_ref(), space, pane).get(cursor) {
                    followed.push((pane.id, task.item.id()));
                }
            }
        }
        self.board = Some(board);
        self.refreshing = false;
        for space in self.desk.spaces() {
            for pane in &space.panes {
                let tasks = listing(self.board.as_ref(), space, pane);
                let wanted = followed.iter().find(|(id, _)| *id == pane.id);
                let row = wanted
                    .and_then(|(_, item)| tasks.iter().position(|task| task.item.id() == *item))
                    .unwrap_or_else(|| self.cursors.get(&pane.id).copied().unwrap_or(0));
                self.cursors
                    .insert(pane.id, row.min(tasks.len().saturating_sub(1)));
            }
        }
    }

    /// Says of every task on the board what its agent is doing.
    fn mark_agents(&mut self, of: &dyn Fn(&Task, &[OpenSession]) -> Option<AgentState>) {
        let sessions = std::mem::take(&mut self.sessions);
        if let Some(board) = &mut self.board {
            board.mark_agents(|task| of(task, &sessions));
        }
        self.sessions = sessions;
        // A pane filtered on agents just changed under the cursor.
        self.select(self.cursor());
    }

    fn changed(&mut self) {
        self.unsaved = true;
        self.select(self.cursor());
    }

    fn refuse(&mut self, why: &str) {
        self.notice = Some(why.to_string());
    }

    fn ask(&mut self, ask: Ask) {
        let text = match ask {
            Ask::PaneFilter => self.desk.space().pane().filter.query().to_string(),
            Ask::PaneTitle => self.desk.space().pane().title.clone(),
            Ask::SpaceName => self.desk.space().name.clone(),
            Ask::SpaceScope => self.desk.space().scope.query().to_string(),
            Ask::NewSpace => String::new(),
        };
        self.mode = Mode::Prompt(Prompt {
            ask,
            original: text.clone(),
            text,
            error: None,
        });
    }

    /// Applies what is typed so far. Filters apply as you type, so the pane
    /// narrows under your fingers; a query that does not parse yet only
    /// shows why.
    fn apply(&mut self, ask: Ask, text: &str) -> Result<(), String> {
        match ask {
            Ask::PaneFilter => {
                self.desk.pane_mut().filter =
                    Filter::parse(text).map_err(|error| error.to_string())?;
            }
            Ask::SpaceScope => {
                self.desk.space_mut().scope =
                    Filter::parse(text).map_err(|error| error.to_string())?;
            }
            Ask::PaneTitle => self.desk.pane_mut().title = text.trim().to_string(),
            Ask::SpaceName | Ask::NewSpace => {}
        }
        self.select(self.cursor());
        Ok(())
    }

    fn prompt_key(&mut self, key: KeyEvent) {
        let Mode::Prompt(prompt) = &mut self.mode else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                let (ask, original) = (prompt.ask, prompt.original.clone());
                self.mode = Mode::Normal;
                let _ = self.apply(ask, &original);
                return;
            }
            KeyCode::Enter => {
                let (ask, text) = (prompt.ask, prompt.text.trim().to_string());
                if let Err(error) = self.apply(ask, &text) {
                    if let Mode::Prompt(prompt) = &mut self.mode {
                        prompt.error = Some(error);
                    }
                    return;
                }
                match ask {
                    Ask::SpaceName if !text.is_empty() => self.desk.space_mut().name = text,
                    Ask::NewSpace if !text.is_empty() => self.desk.add_space(&text),
                    _ => {}
                }
                self.mode = Mode::Normal;
                self.changed();
                return;
            }
            KeyCode::Backspace => {
                prompt.text.pop();
            }
            KeyCode::Char(character) => prompt.text.push(character),
            _ => return,
        }
        let (ask, text) = (prompt.ask, prompt.text.clone());
        let error = self.apply(ask, &text).err();
        if let Mode::Prompt(prompt) = &mut self.mode {
            prompt.error = error;
        }
    }

    fn show_space(&mut self, index: usize) {
        if self.desk.show(index) {
            self.zoomed = false;
        } else {
            self.refuse(&format!("there is no space {} (N makes one)", index + 1));
        }
    }

    fn key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        self.notice = None;
        self.done = None;
        match self.mode {
            Mode::Prompt(_) => {
                self.prompt_key(key);
                return Action::None;
            }
            Mode::Help | Mode::Explain | Mode::AboutSpace => {
                self.mode = Mode::Normal;
                return Action::None;
            }
            Mode::Send => {
                self.mode = Mode::Normal;
                if let KeyCode::Char(digit @ '1'..='9') = key.code {
                    if self.desk.send_pane(digit as usize - '1' as usize) {
                        self.zoomed = false;
                        self.changed();
                    } else {
                        self.refuse("cannot send it there: no such space, or its last pane");
                    }
                }
                return Action::None;
            }
            Mode::Postpone => {
                self.mode = Mode::Normal;
                let choice = match key.code {
                    KeyCode::Char('t') => Some(Postpone::Days(1)),
                    KeyCode::Char('w') => Some(Postpone::Days(7)),
                    KeyCode::Char('m') => Some(Postpone::UntilItMoves),
                    KeyCode::Char(digit @ '1'..='9') => {
                        Some(Postpone::Days(digit as u32 - '0' as u32))
                    }
                    _ => None,
                };
                return match (choice, self.selected()) {
                    (Some(choice), Some(task)) => Action::Postpone(task.item.id(), choice),
                    _ => Action::None,
                };
            }
            Mode::ConfirmRemoveSpace => {
                self.mode = Mode::Normal;
                if key.code == KeyCode::Char('y') {
                    if self.desk.remove_space() {
                        self.zoomed = false;
                        self.changed();
                    } else {
                        self.refuse("the last space stays");
                    }
                }
                return Action::None;
            }
            Mode::Normal => {}
        }

        let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('r') => return Action::Refresh,
            KeyCode::Enter | KeyCode::Char('o') => {
                if let Some(task) = self.selected() {
                    return Action::Open(task.target_url().to_string());
                }
            }
            KeyCode::Char('a') => {
                if let Some(task) = self.selected() {
                    return Action::Agent(Box::new(task.clone()));
                }
            }
            KeyCode::Char('p') => match self.selected() {
                // Already aside: the same key brings it back.
                Some(task) if task.snoozed.is_some() => {
                    return Action::Postpone(task.item.id(), Postpone::BringBack);
                }
                Some(_) => self.mode = Mode::Postpone,
                None => {}
            },
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('i') => match self.selected() {
                Some(_) => self.mode = Mode::Explain,
                None => self.refuse("no task selected to explain (I explains the space)"),
            },
            KeyCode::Char('I') => self.mode = Mode::AboutSpace,
            KeyCode::Esc => self.zoomed = false,

            // Within the pane.
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::PageDown => self.step(10),
            KeyCode::PageUp => self.step(-10),
            KeyCode::Home => self.select(0),
            KeyCode::End => self.select(usize::MAX),

            // Between panes; with Shift, the pane itself moves.
            KeyCode::Left if shifted => self.move_pane(-1),
            KeyCode::Right if shifted => self.move_pane(1),
            KeyCode::Char('H') => self.move_pane(-1),
            KeyCode::Char('L') => self.move_pane(1),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => self.desk.cycle_pane(-1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => self.desk.cycle_pane(1),

            // Between spaces.
            KeyCode::Char(digit @ '1'..='9') => self.show_space(digit as usize - '1' as usize),
            KeyCode::Char(',') => {
                self.desk.cycle_space(-1);
                self.zoomed = false;
            }
            KeyCode::Char('.') => {
                self.desk.cycle_space(1);
                self.zoomed = false;
            }
            KeyCode::Char('<') => {
                self.desk.shift_space(-1);
                self.changed();
            }
            KeyCode::Char('>') => {
                self.desk.shift_space(1);
                self.changed();
            }

            // The focused pane.
            KeyCode::Char('/') => self.ask(Ask::PaneFilter),
            KeyCode::Char('e') => self.ask(Ask::PaneTitle),
            KeyCode::Char('b') => {
                let pane = self.desk.pane_mut();
                let next = match pane.filter.bucket() {
                    Some(Bucket::Todo) => Some(Bucket::Waiting),
                    Some(Bucket::Waiting) => None,
                    None => Some(Bucket::Todo),
                };
                pane.filter = pane.filter.with_bucket(next);
                self.changed();
            }
            KeyCode::Char('g') => {
                let pane = self.desk.pane_mut();
                pane.grouping = pane.grouping.next();
                // The order changed under the cursor: start from the top.
                self.unsaved = true;
                self.select(0);
            }
            KeyCode::Char('s') => {
                let pane = self.desk.pane_mut();
                pane.sort = pane.sort.next();
                pane.reversed = false;
                self.unsaved = true;
                self.select(0);
            }
            KeyCode::Char('v') => {
                let pane = self.desk.pane_mut();
                pane.reversed = !pane.reversed;
                self.unsaved = true;
                self.select(0);
            }
            KeyCode::Char('n') => {
                self.desk.split_pane();
                self.zoomed = false;
                self.changed();
                self.ask(Ask::PaneFilter);
            }
            KeyCode::Char('x') => {
                if self.desk.close_pane() {
                    self.zoomed = false;
                    self.changed();
                } else {
                    self.refuse("a space keeps one pane (X removes the space)");
                }
            }
            KeyCode::Char('m') => self.mode = Mode::Send,
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
            KeyCode::Char('d') => self.detail = !self.detail,

            // The space.
            KeyCode::Char('t') => {
                let space = self.desk.space_mut();
                space.layout = space.layout.next();
                self.zoomed = false;
                self.changed();
            }
            KeyCode::Char('N') => self.ask(Ask::NewSpace),
            KeyCode::Char('R') => self.ask(Ask::SpaceName),
            KeyCode::Char('S') => self.ask(Ask::SpaceScope),
            KeyCode::Char('X') => self.mode = Mode::ConfirmRemoveSpace,
            _ => {}
        }
        Action::None
    }

    /// Sets an item aside on the board at hand, or brings it back, and
    /// says what there is to remember.
    fn postpone(&mut self, item: &str, choice: Postpone, now: Timestamp) -> Option<Snooze> {
        let until = match choice {
            Postpone::Days(days) => Some(Until::At(Timestamp(now.0 + i64::from(days) * 86_400))),
            Postpone::UntilItMoves => Some(Until::Moves),
            Postpone::BringBack => None,
        };
        let snooze = self.board.as_mut()?.snooze(item, until);
        self.done = Some(match until {
            Some(until) => wording::set_aside(until, now),
            None => "brought back".to_string(),
        });
        // The list under the cursor just lost or gained a line.
        self.select(self.cursor());
        snooze
    }

    fn move_pane(&mut self, step: isize) {
        self.desk.shift_pane(step);
        self.changed();
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        let mut show = None;
        let mut focus = None;
        for hit in &self.hits {
            match hit {
                Hit::Space { area, index } if area.contains(at) => show = Some(*index),
                Hit::Pane {
                    area,
                    index,
                    top,
                    offset,
                    lines,
                } if area.contains(at) => {
                    let line = (mouse.row >= *top)
                        .then(|| lines.get(offset + (mouse.row - top) as usize))
                        .flatten();
                    focus = Some((*index, line.copied().flatten()));
                }
                _ => {}
            }
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = show {
                    self.show_space(index);
                } else if let Some((pane, task)) = focus {
                    self.desk.focus(pane);
                    if let Some(task) = task {
                        self.select(task);
                    }
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                if let Some((pane, _)) = focus {
                    self.desk.focus(pane);
                    let down = mouse.kind == MouseEventKind::ScrollDown;
                    self.step(if down { 3 } else { -3 });
                }
            }
            _ => {}
        }
    }
}

// ── geometry ───────────────────────────────────────────────────────────

/// Where each of `count` panes goes in `area`.
fn tile(layout: Layout, count: usize, area: Rect) -> Vec<Rect> {
    let even = |count: usize| vec![Constraint::Ratio(1, count as u32); count];
    match (layout, count) {
        (_, 0) => Vec::new(),
        (_, 1) => vec![area],
        (Layout::Columns, _) => Split::horizontal(even(count)).split(area).to_vec(),
        (Layout::Rows, _) => Split::vertical(even(count)).split(area).to_vec(),
        (Layout::Master, _) => {
            let [master, stack]: [Rect; 2] =
                Split::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(area);
            let mut rects = vec![master];
            rects.extend(Split::vertical(even(count - 1)).split(stack).iter());
            rects
        }
        (Layout::Grid, _) => {
            let columns = (count as f64).sqrt().ceil() as usize;
            let rows = count.div_ceil(columns);
            let mut rects = Vec::with_capacity(count);
            for (row, band) in Split::vertical(even(rows)).split(area).iter().enumerate() {
                // The last row shares the width among what is left.
                let here = columns.min(count - row * columns);
                rects.extend(Split::horizontal(even(here)).split(*band).iter());
            }
            rects
        }
    }
}

// ── drawing ────────────────────────────────────────────────────────────

const DIM: Style = Style::new().fg(Color::DarkGray);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const FOCUS: Color = Color::Magenta;

fn accent(bucket: Bucket) -> Color {
    match bucket {
        Bucket::Todo => Color::Yellow,
        Bucket::Waiting => Color::Cyan,
    }
}

fn agent_color(state: AgentState) -> Color {
    match state {
        AgentState::Working => Color::Blue,
        // The one that wants the user stands out.
        AgentState::Blocked => Color::Magenta,
        AgentState::Done => Color::Green,
        AgentState::Idle | AgentState::Unknown => Color::DarkGray,
    }
}

fn heat_color(heat: Heat) -> Color {
    match heat {
        Heat::Hot => Color::Red,
        Heat::Warm | Heat::Calm => Color::Yellow,
    }
}

fn layout_glyph(layout: Layout) -> &'static str {
    match layout {
        Layout::Columns => "▥",
        Layout::Rows => "▤",
        Layout::Master => "◧",
        Layout::Grid => "▦",
    }
}

/// The dock: spaces, then the state of the world.
fn dock(screen: &mut Screen, area: Rect, now: Timestamp) -> Line<'static> {
    let mut spans = vec![Span::styled(" aki ", BOLD)];
    let mut x = area.x + 5;
    for (index, space) in screen.desk.spaces().iter().enumerate() {
        let todo = screen
            .board
            .as_ref()
            .map(|board| space.todo_count(board))
            .unwrap_or(0);
        let text = format!(" {} {} {todo} ", index + 1, space.name);
        let width = text.width() as u16;
        let style = if index == screen.desk.current() {
            Style::new()
                .bg(FOCUS)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else if todo > 0 {
            Style::new().fg(Color::Gray)
        } else {
            DIM
        };
        screen.hits.push(Hit::Space {
            area: Rect::new(x, area.y, width, 1),
            index,
        });
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
        x += width + 1;
    }

    let space = screen.desk.space();
    spans.push(Span::styled(
        format!(" {} {}", layout_glyph(space.layout), space.layout.name()),
        DIM,
    ));
    if !space.scope.is_all() {
        spans.push(Span::styled(format!(" · {}", space.scope.query()), DIM));
    }
    let aside = screen.board.as_ref().map(Board::snoozed_count).unwrap_or(0);
    if aside > 0 {
        spans.push(Span::styled(format!(" · {aside} set aside"), DIM));
    }
    let apart = screen.board.as_ref().map(Board::apart_count).unwrap_or(0);
    if apart > 0 {
        spans.push(Span::styled(format!(" · {apart} drafts apart"), DIM));
    }
    spans.push(Span::raw("   "));

    match &screen.board {
        Some(board) => {
            for source in &board.sources {
                spans.push(Span::styled(
                    format!("{source} ✓ "),
                    Style::new().fg(Color::Green),
                ));
            }
            for failure in &board.failures {
                spans.push(Span::styled(
                    format!("{} ✗ ", failure.source),
                    Style::new().fg(Color::Red),
                ));
            }
            let state = if screen.refreshing {
                "refreshing…".to_string()
            } else {
                match age(board.refreshed_at, now).as_str() {
                    "now" => "live".to_string(),
                    ago => format!("{ago} ago"),
                }
            };
            spans.push(Span::styled(format!(" {state}"), DIM));
        }
        None => spans.push(Span::styled("reading your sources…", DIM)),
    }
    Line::from(spans)
}

/// One task on one line, as wide as the pane allows.
/// `agents`: whether the pane has a column for agents at all.
/// `scored`: whether the pane is ordered by urgency, and shows the score
/// its order comes from.
fn row(
    task: &Task,
    grouping: Grouping,
    width: usize,
    (agents, scored): (bool, bool),
    now: Timestamp,
) -> ListItem<'static> {
    // In a pane grouped by repository, the heading already names it.
    let (reference, reference_width) = match grouping {
        Grouping::Repo => (task.item.reference.clone(), 10),
        _ => (task.item.short(), 17),
    };
    let extra = match task.reasons.len() {
        1 => String::new(),
        more => format!("+{}", more - 1),
    };
    let color = Style::new().fg(accent(task.bucket()));
    let with_label = width >= 72 && grouping != Grouping::Kind;
    let with_who = width >= 110 && grouping != Grouping::Person;
    let with_score = scored && width >= 72;

    // The heat mark, the reference, its gap, the title's gap, "+n" and the
    // age are always there.
    let mut title = width.saturating_sub(reference_width + 11);
    if agents {
        title = title.saturating_sub(2);
    }
    let mut spans = vec![match task.snoozed {
        // Set aside: it sleeps, whatever its heat.
        Some(_) => Span::styled("z ", DIM),
        None => Span::styled(
            format!("{} ", wording::heat(task.urgency.heat)),
            Style::new().fg(heat_color(task.urgency.heat)),
        ),
    }];
    if agents {
        spans.push(match task.agent {
            Some(state) => Span::styled(
                format!("{} ", wording::agent(state)),
                Style::new()
                    .fg(agent_color(state))
                    .add_modifier(Modifier::BOLD),
            ),
            None => Span::raw("  "),
        });
    }
    if with_label {
        spans.push(Span::styled(
            format!("{} ", column(wording::headline(task), 15)),
            color,
        ));
        title = title.saturating_sub(16);
    } else {
        spans.push(Span::styled("● ", color));
        title = title.saturating_sub(2);
    }
    if with_who {
        title = title.saturating_sub(15);
    }
    if with_score {
        title = title.saturating_sub(4);
    }
    spans.push(Span::styled(
        format!("{} ", column(&reference, reference_width)),
        BOLD,
    ));
    spans.push(Span::raw(format!(
        "{} ",
        column(&plain(&task.item.title), title.max(4))
    )));
    if with_who {
        let who = task
            .counterpart
            .as_deref()
            .map(|name| format!("@{name}"))
            .unwrap_or_default();
        spans.push(Span::styled(format!("{} ", column(&who, 14)), DIM));
    }
    spans.push(Span::styled(format!("{extra:<3}"), DIM));
    if with_score {
        spans.push(Span::styled(
            format!("{:>4}", task.urgency.score()),
            Style::new().fg(match task.urgency.heat {
                Heat::Calm => Color::DarkGray,
                heat => heat_color(heat),
            }),
        ));
    }
    spans.push(Span::styled(format!("{:>4}", age(task.since, now)), DIM));
    ListItem::new(Line::from(spans))
}

fn heading(key: &GroupKey, count: usize, width: usize) -> ListItem<'static> {
    // A repository is known by its last segment; where it lives comes after.
    let full = wording::group(key);
    let (name, place) = match key {
        GroupKey::Repo(_) => match full.rsplit_once('/') {
            Some((parent, name)) => (name.to_string(), format!("{parent} ")),
            None => (full, String::new()),
        },
        _ => (full, String::new()),
    };
    let name = wording::fit(&name, width.saturating_sub(8));
    let tail = format!(" {count}");
    let room = width.saturating_sub(name.width() + tail.width() + 1);
    let place = if place.width() + 3 <= room {
        place
    } else {
        String::new()
    };
    let rule = "─".repeat(room - place.width());
    ListItem::new(Line::from(vec![
        Span::styled(format!("{name} "), BOLD),
        Span::styled(place, DIM),
        Span::styled(rule, DIM),
        Span::styled(tail, DIM),
    ]))
}

fn draw_pane(frame: &mut Frame, screen: &mut Screen, index: usize, area: Rect, now: Timestamp) {
    let space = screen.desk.space();
    let pane = &space.panes[index];
    let focused = index == space.focus;
    let groups = screen
        .board
        .as_ref()
        .map(|board| space.groups(pane, board))
        .unwrap_or_default();
    let total: usize = groups.iter().map(|group| group.tasks.len()).sum();

    let side = pane.filter.bucket().map(accent).unwrap_or(Color::Gray);
    let border = if focused { Style::new().fg(FOCUS) } else { DIM };
    let name = wording::fit(
        &wording::pane_title(pane),
        area.width.saturating_sub(18) as usize,
    );
    let mut block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title_top(Line::from(vec![
            Span::styled(format!(" {} ", index + 1), border),
            Span::styled(name, Style::new().fg(side).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {total} "), DIM),
        ]));
    let order = wording::sort(pane.sort, pane.reversed);
    let arrangement = match pane.grouping {
        Grouping::None => format!(" {order} "),
        grouping => format!(" by {} · {order} ", grouping.name()),
    };
    // A narrow pane keeps its room for its name.
    if area.width >= 56 {
        block = block.title_top(Line::from(Span::styled(arrangement, DIM)).right_aligned());
    }
    if screen.zoomed {
        block = block.title_bottom(Line::from(Span::styled(" zoomed · z ", DIM)).right_aligned());
    }
    let inner = block.inner(area);

    let agents = groups
        .iter()
        .flat_map(|group| &group.tasks)
        .any(|task| task.agent.is_some());

    // The list, and which task each of its lines is.
    let width = inner.width.saturating_sub(2) as usize;
    let mut items = Vec::new();
    let mut lines = Vec::new();
    let mut position = 0;
    for group in &groups {
        if group.key != GroupKey::All {
            items.push(heading(&group.key, group.tasks.len(), width));
            lines.push(None);
        }
        for task in &group.tasks {
            let columns = (agents, pane.sort == Sort::Urgency);
            items.push(row(task, pane.grouping, width, columns, now));
            lines.push(Some(position));
            position += 1;
        }
    }

    let cursor = screen.cursors.get(&pane.id).copied().unwrap_or(0);
    let mut state = ListState::default();
    state.select(lines.iter().position(|line| *line == Some(cursor)));

    if items.is_empty() {
        let message = match (&screen.board, pane.filter.bucket()) {
            (None, _) => "reading…",
            (_, Some(Bucket::Todo)) => "nothing waits on you here",
            (_, Some(Bucket::Waiting)) => "you wait on nothing here",
            (_, None) => "nothing here",
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(format!(" {message}"), DIM))).block(block),
            area,
        );
    } else {
        let highlight = if focused {
            Style::new()
                .bg(Color::Indexed(237))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        let list = List::new(items)
            .block(block)
            .highlight_symbol(if focused { "▸ " } else { "  " })
            .highlight_style(highlight)
            // Keeps the group heading above the first tasks in view.
            .scroll_padding(1);
        frame.render_stateful_widget(list, area, &mut state);
    }

    screen.hits.push(Hit::Pane {
        area,
        index,
        top: inner.y,
        offset: state.offset(),
        lines,
    });
}

fn detail(task: &Task, now: Timestamp) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled(task.item.short(), BOLD),
            Span::raw("  "),
            Span::raw(plain(&task.item.title)),
        ]),
        Line::from(Span::styled(
            format!(
                "{} · {} · you: {}{}",
                task.item.source,
                task.item.container,
                task.role.name(),
                task.counterpart
                    .as_deref()
                    .map(|name| format!(" · by @{name}"))
                    .unwrap_or_default(),
            ),
            DIM,
        )),
    ];
    for reason in &task.reasons {
        lines.push(Line::from(vec![
            Span::styled(
                format!("● {:<16} ", wording::label(reason.kind)),
                Style::new().fg(accent(reason.bucket)),
            ),
            Span::raw(wording::explain(reason)),
        ]));
    }
    if let Some(state) = task.agent {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", wording::agent(state)),
                Style::new()
                    .fg(agent_color(state))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{} · a shows it", wording::agent_state(state))),
        ]));
    }
    if let Some(until) = task.snoozed {
        lines.push(Line::from(Span::styled(
            format!("z {} · p brings it back", wording::set_aside(until, now)),
            DIM,
        )));
    }
    let mut score = wording::urgency(task);
    // A ticket that brings no points is still worth naming.
    if task.urgency.ticket == 0 && !task.tickets.is_empty() {
        score = format!("{score} · {}", wording::tickets(task));
    }
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", wording::heat(task.urgency.heat)),
            Style::new().fg(heat_color(task.urgency.heat)),
        ),
        Span::styled(format!("{score} · i explains"), DIM),
    ]));
    lines.push(Line::from(Span::styled(task.target_url().to_string(), DIM)));
    lines
}

const KEYS: &[(&str, &str)] = &[
    ("", "MOVE"),
    ("↑ ↓  PgUp PgDn  Home End", "the task, within the pane"),
    ("← →  Tab", "the pane (or click it)"),
    ("1-9  , .", "the space (or click it)"),
    (
        "Enter",
        "open in the browser, on the thread when there is one",
    ),
    ("a", "open your coding agent on it, briefed, in its clone"),
    (
        "p",
        "set it aside until a day or until it moves (filter is:snoozed shows them; p brings back)",
    ),
    (
        "i",
        "explain it: why it is here, how its urgency adds up, what every mark means",
    ),
    ("", "THE PANE"),
    (
        "/",
        "filter: is:todo|waiting|draft|snoozed source: repo: kind: top: role: by: heat: ticket: priority: agent: words",
    ),
    ("b", "side: to do → waiting → both"),
    ("g", "group: none → repo → kind → source → person → role"),
    (
        "s  v",
        "sort: urgency → rule → waiting → updated → created · reverse it",
    ),
    ("n  x", "new pane beside this one · close it"),
    ("Shift+← →", "move it among the others"),
    ("m 1-9", "send it to another space"),
    ("e", "rename it"),
    ("z  d", "zoom it · show or hide the detail"),
    ("", "THE SPACE"),
    (
        "I",
        "explain it: how it is made, its panes, the repositories it holds",
    ),
    ("t", "layout: columns → rows → master → grid"),
    ("N  R  X", "new space · rename · remove"),
    ("S", "scope: what belongs to this space at all"),
    ("<  >", "move it among the others"),
    ("", ""),
    ("r  ?  q", "refresh now · this help · quit"),
];

/// A framed window over the board, as large as its lines ask for and the
/// screen allows.
fn popup(frame: &mut Frame, title: &str, lines: Vec<Line<'static>>) {
    let area = frame.area();
    let widest = lines.iter().map(Line::width).max().unwrap_or(0) as u16;
    let width = (widest + 4).clamp(40, 112).min(area.width);
    let inner = width.saturating_sub(2).max(1) as usize;
    // Long lines wrap: count the rows they will take.
    let rows: usize = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(inner))
        .sum();
    let height = (rows as u16 + 2).min(area.height);
    let place = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, place);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::new()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(FOCUS))
                .title_top(format!(" {title} "))
                .title_bottom(Line::from(Span::styled(" any key closes ", DIM)).right_aligned()),
        ),
        place,
    );
}

fn section(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {title}"),
        Style::new().fg(FOCUS).add_modifier(Modifier::BOLD),
    ))
}

fn note(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(format!(" {}", text.into()), DIM))
}

fn draw_help(frame: &mut Frame) {
    let lines: Vec<Line> = KEYS
        .iter()
        .map(|(keys, what)| {
            if keys.is_empty() {
                section(what)
            } else {
                Line::from(vec![
                    Span::styled(format!(" {} ", column(keys, 25)), Style::new().fg(FOCUS)),
                    Span::raw(what.to_string()),
                ])
            }
        })
        .collect();
    popup(frame, "keys", lines);
}

/// Everything the board says about one task, in full sentences: why it is
/// listed, how its place is computed, and what each mark on its line means.
fn explanation(task: &Task, pane: &Pane, rules: &Rules, now: Timestamp) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!(" {}", task.item.short()), BOLD),
            Span::raw(format!("  {}", wording::fit(&plain(&task.item.title), 80))),
        ]),
        Line::default(),
        section("WHY IT IS HERE"),
    ];
    for reason in &task.reasons {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" ● {} ", column(wording::label(reason.kind), 16)),
                Style::new().fg(accent(reason.bucket)),
            ),
            Span::raw(wording::explain(reason)),
            Span::styled(
                format!(
                    " · {} · for {}",
                    match reason.bucket {
                        Bucket::Todo => "yours to do",
                        Bucket::Waiting => "waits on others",
                    },
                    age(reason.since, now)
                ),
                DIM,
            ),
        ]));
    }
    lines.push(note(match task.reasons.len() {
        1 => "This reason names the task and decides its side.",
        _ => "The first one is the most pressing: it names the task and decides its side.",
    }));
    if task.apart {
        lines.push(note(
            "It is a draft of yours, kept apart: it shows only in panes that ask is:draft ([drafts] mine).",
        ));
    }

    lines.push(Line::default());
    lines.push(section("HOW ITS URGENCY ADDS UP"));
    for part in wording::urgency_parts(task, rules, now) {
        let total = part.what == "urgency";
        let style = if total { BOLD } else { Style::new() };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", column(part.what, 9)), style),
            Span::styled(format!("{:>6}  ", part.points), style),
            Span::styled(part.how, DIM),
        ]));
    }
    lines.push(note(format!(
        "This pane is ordered by {}: {}.",
        wording::sort(pane.sort, pane.reversed),
        wording::sort_explained(pane.sort, pane.reversed)
    )));
    lines.push(note("s changes the order, v reverses it."));
    lines.push(note(
        "The weights are yours: [urgency] and [rules.*] in the configuration; `aki rules` prints them.",
    ));

    lines.push(Line::default());
    lines.push(section("READING A LINE"));
    for (mark, meaning) in [
        (
            "▲ ▴",
            "hot, warm: the urgency passed a threshold · z: set aside",
        ),
        (
            "◐ ? ✓ · ○",
            "your agent on it: working, waiting for you, done, idle, terminal open",
        ),
        (
            "Review…",
            "the most pressing reason · yellow: yours to do · cyan: waits on others",
        ),
        (
            "@name",
            "who is on the other side: the author when you review",
        ),
        ("+2", "two more reasons than the one shown"),
        ("185", "the urgency score, in panes ordered by urgency"),
        ("3d", "for how long the most pressing reason has been true"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", column(mark, 10)), Style::new().fg(FOCUS)),
            Span::raw(meaning),
        ]));
    }
    lines
}

/// How the shown space is made and what it holds.
fn about_space(screen: &Screen) -> Vec<Line<'static>> {
    let space = screen.desk.space();
    let or_all = |query: &str| match query {
        "" => "everything".to_string(),
        query => query.to_string(),
    };
    let field = |name: &str, value: String, key: &str| {
        Line::from(vec![
            Span::styled(format!(" {} ", column(name, 7)), BOLD),
            Span::raw(value),
            Span::styled(format!("  {key}"), DIM),
        ])
    };
    let mut lines = vec![
        section("HOW IT IS MADE"),
        field("name", space.name.clone(), "R renames"),
        field(
            "scope",
            or_all(space.scope.query()),
            "S changes what belongs here at all",
        ),
        field("layout", space.layout.name().to_string(), "t changes"),
        note(
            "Spaces start as \"All\" plus one per source. From then on they are what you make them:",
        ),
        note(format!(
            "every change is saved in {}, which can also be edited by hand.",
            if screen.kept_in.is_empty() {
                "spaces.toml"
            } else {
                &screen.kept_in
            }
        )),
        Line::default(),
        section("ITS PANES"),
    ];
    for (index, pane) in space.panes.iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", index + 1), Style::new().fg(FOCUS)),
            Span::styled(format!("{} ", column(&wording::pane_title(pane), 22)), BOLD),
            Span::raw(format!(
                "filter: {} · group: {} · sort: {}",
                or_all(pane.filter.query()),
                pane.grouping.name(),
                wording::sort(pane.sort, pane.reversed)
            )),
        ]));
    }
    lines.push(note(
        "/ filter · g group · s sort · n new pane · x close · e rename · m send to another space",
    ));

    lines.push(Line::default());
    lines.push(section("WHAT IT HOLDS RIGHT NOW"));
    let holdings = screen
        .board
        .as_ref()
        .map(|board| space.holdings(board))
        .unwrap_or_default();
    if holdings.is_empty() {
        lines.push(note("nothing"));
    }
    // A long list ends with how many are left out.
    let shown = holdings.len().min(18);
    for held in &holdings[..shown] {
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", column(&held.source, 8)), DIM),
            Span::raw(format!("{} ", column(&held.container, 44))),
            Span::styled(
                format!("{:>3} to do ", held.todo),
                Style::new().fg(accent(Bucket::Todo)),
            ),
            Span::styled(
                format!("{:>3} waiting", held.waiting),
                Style::new().fg(accent(Bucket::Waiting)),
            ),
        ]));
    }
    if holdings.len() > shown {
        lines.push(note(format!("and {} more", holdings.len() - shown)));
    }
    lines.push(note(
        "To keep a repository out: S, then -repo:name. For one of its own: N, then S repo:name.",
    ));
    lines
}

fn footer(screen: &Screen) -> Line<'static> {
    match &screen.mode {
        Mode::Prompt(prompt) => {
            let mut spans = vec![
                Span::styled(format!(" {}: ", prompt.ask.label()), BOLD),
                Span::raw(format!("{}▏", prompt.text)),
            ];
            match &prompt.error {
                Some(error) => spans.push(Span::styled(
                    format!("   {error}"),
                    Style::new().fg(Color::Red),
                )),
                None => spans.push(Span::styled("   Enter keep · Esc cancel", DIM)),
            }
            Line::from(spans)
        }
        Mode::Send => Line::from(Span::styled(
            " send this pane to space: 1-9 · any other key cancels",
            BOLD,
        )),
        Mode::Postpone => Line::from(Span::styled(
            " set aside until: t tomorrow · w a week · 1-9 days · m it moves · any other key cancels",
            BOLD,
        )),
        Mode::ConfirmRemoveSpace => Line::from(Span::styled(
            format!(
                " remove the space \"{}\" and its panes? y · any other key cancels",
                screen.desk.space().name
            ),
            BOLD,
        )),
        _ => match (&screen.notice, &screen.done) {
            (Some(notice), _) => Line::from(Span::styled(
                format!(" {notice}"),
                Style::new().fg(Color::Red),
            )),
            (None, Some(done)) => Line::from(Span::styled(
                format!(" {done}"),
                Style::new().fg(Color::Green),
            )),
            (None, None) => Line::from(Span::styled(
                " ←→ pane · ↑↓ task · 1-9 space · Enter open · a agent · i explain · / filter · g group · s sort · t layout · ? keys",
                DIM,
            )),
        },
    }
}

fn draw(frame: &mut Frame, screen: &mut Screen, now: Timestamp) {
    screen.hits.clear();
    let failures = screen
        .board
        .as_ref()
        .map(|board| board.failures.len())
        .unwrap_or(0) as u16;
    let detail_height = match screen.selected() {
        Some(task) if screen.detail => {
            let extra = u16::from(task.agent.is_some()) + u16::from(task.snoozed.is_some());
            (task.reasons.len() as u16 + 5 + extra).min(12)
        }
        _ => 0,
    };
    let [top, body, detail_area, failure_area, bottom]: [Rect; 5] = Split::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(detail_height),
        Constraint::Length(failures),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let line = dock(screen, top, now);
    frame.render_widget(Paragraph::new(line), top);

    let (count, focus, layout) = {
        let space = screen.desk.space();
        (space.panes.len(), space.focus, space.layout)
    };
    if screen.zoomed {
        draw_pane(frame, screen, focus, body, now);
    } else {
        for (index, area) in tile(layout, count, body).into_iter().enumerate() {
            draw_pane(frame, screen, index, area, now);
        }
    }

    if let Some(task) = screen.selected().filter(|_| detail_height > 0) {
        frame.render_widget(
            Paragraph::new(detail(task, now))
                .wrap(Wrap { trim: false })
                .block(Block::new().borders(Borders::TOP).border_style(DIM)),
            detail_area,
        );
    }

    if let Some(board) = &screen.board {
        let lines: Vec<Line> = board
            .failures
            .iter()
            .map(|failure| {
                Line::from(Span::styled(
                    format!(
                        " ✗ {}: {}{}",
                        failure.source,
                        failure.message,
                        match failure.showing_from {
                            Some(at) => format!(" · showing what was read {} ago", age(at, now)),
                            None => String::new(),
                        }
                    ),
                    Style::new().fg(Color::Red),
                ))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), failure_area);
    }

    frame.render_widget(Paragraph::new(footer(screen)), bottom);
    match screen.mode {
        Mode::Help => draw_help(frame),
        Mode::Explain => {
            if let Some(task) = screen.selected() {
                let lines = explanation(task, screen.desk.space().pane(), &screen.rules, now);
                popup(frame, "this task", lines);
            }
        }
        Mode::AboutSpace => {
            let title = format!(
                "space {} · {}",
                screen.desk.current() + 1,
                screen.desk.space().name
            );
            popup(frame, &title, about_space(screen));
        }
        _ => {}
    }
}

fn event_loop(terminal: &mut DefaultTerminal, desk: Desk, wiring: &Wiring) -> io::Result<()> {
    let mut screen = Screen::new(desk);
    screen.rules = wiring.rules.clone();
    screen.kept_in = wiring.kept_in.clone();
    loop {
        for update in wiring.updates.try_iter() {
            match update {
                Update::Refreshing => screen.refreshing = true,
                Update::Board(board) => {
                    screen.receive(board);
                    screen.mark_agents(wiring.agent_of);
                }
                Update::Agents(sessions) => {
                    screen.sessions = sessions;
                    screen.mark_agents(wiring.agent_of);
                }
            }
        }
        if std::mem::take(&mut screen.unsaved)
            && let Err(error) = (wiring.save)(&screen.desk)
        {
            screen.notice = Some(error);
        }
        let now = wiring.clock.now();
        terminal.draw(|frame| draw(frame, &mut screen, now))?;

        // Short enough that a fresh board shows up at once.
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let action = match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => screen.key(key),
            Event::Mouse(mouse) => {
                screen.mouse(mouse);
                Action::None
            }
            _ => Action::None,
        };
        match action {
            Action::None => {}
            Action::Quit => return Ok(()),
            Action::Refresh => {
                let _ = wiring.requests.send(Pace::Again);
            }
            Action::Open(url) => (wiring.open)(&url),
            Action::Postpone(item, choice) => {
                let snooze = screen.postpone(&item, choice, wiring.clock.now());
                if let Err(refused) = (wiring.snooze)(&item, snooze) {
                    screen.notice = Some(refused);
                }
            }
            Action::Agent(task) => match (wiring.agent)(&task) {
                Ok(done) => screen.done = Some(done),
                Err(refused) => screen.notice = Some(refused),
            },
        }
    }
}

/// Runs until the user quits, then tells the watch to stop.
pub fn run(desk: Desk, wiring: Wiring) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(io::stdout(), EnableMouseCapture);
    let outcome = event_loop(&mut terminal, desk, &wiring);
    let _ = crossterm::execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    let _ = wiring.requests.send(Pace::Stop);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::item::WorkItem;
    use crate::entities::rules::Rules;
    use crate::entities::triage::triage;
    use crate::use_cases::testing::{review_request, ticket};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn board(references: &[&str]) -> Board {
        let mut tasks: Vec<Task> = references
            .iter()
            .filter_map(|reference| {
                triage(
                    &review_request(reference),
                    &Rules::default(),
                    Timestamp(900),
                )
            })
            .collect();
        tasks.extend(triage(
            &ticket("X-1", "Blocked"),
            &Rules::default(),
            Timestamp(900),
        ));
        Board::new(
            tasks,
            vec!["gitlab".into(), "jira".into()],
            Vec::new(),
            Timestamp(900),
        )
    }

    /// All · Gitlab · Jira, each with "To do" beside "Waiting".
    fn screen(references: &[&str]) -> Screen {
        let mut screen = Screen::new(Desk::starting(&["gitlab".into(), "jira".into()]));
        screen.receive(board(references));
        screen
    }

    fn press(screen: &mut Screen, code: KeyCode) -> Action {
        screen.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_text(screen: &mut Screen, text: &str) {
        for character in text.chars() {
            press(screen, KeyCode::Char(character));
        }
    }

    fn render(screen: &mut Screen, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, screen, Timestamp(900 + 3 * 86_400)))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn the_cursor_follows_its_item_across_refreshes() {
        let mut screen = screen(&["!1", "!2", "!3"]);
        press(&mut screen, KeyCode::Down);
        let followed = screen.selected().unwrap().item.id();

        screen.receive(board(&["!0", "!1", "!2", "!3"]));
        assert_eq!(screen.selected().unwrap().item.id(), followed);

        // The item is gone: the cursor stays in range.
        screen.receive(board(&["!9"]));
        assert_eq!(screen.cursor(), 0);
        assert!(screen.selected().is_some());
    }

    #[test]
    fn keys_move_between_tasks_panes_and_spaces() {
        let mut screen = screen(&["!1", "!2"]);

        press(&mut screen, KeyCode::Up);
        assert_eq!(screen.cursor(), 0);
        press(&mut screen, KeyCode::End);
        assert_eq!(screen.cursor(), 1);
        assert_eq!(
            press(&mut screen, KeyCode::Enter),
            Action::Open(screen.selected().unwrap().item.url.clone())
        );

        // The other pane keeps its own cursor.
        press(&mut screen, KeyCode::Right);
        assert_eq!(screen.desk.space().focus, 1);
        assert_eq!(screen.tasks().len(), 1);
        assert_eq!(screen.cursor(), 0);
        press(&mut screen, KeyCode::Left);
        assert_eq!(screen.cursor(), 1);

        // Jira's space has nothing to do, one thing waiting.
        press(&mut screen, KeyCode::Char('3'));
        assert_eq!(screen.desk.space().name, "Jira");
        assert!(screen.tasks().is_empty());
        press(&mut screen, KeyCode::Char(','));
        assert_eq!(screen.desk.space().name, "Gitlab");
        press(&mut screen, KeyCode::Char('9'));
        assert!(screen.notice.is_some(), "no ninth space: say so");
        assert_eq!(screen.desk.space().name, "Gitlab");

        // The agent is handed the selected task itself.
        let selected = screen.selected().unwrap().clone();
        assert_eq!(
            press(&mut screen, KeyCode::Char('a')),
            Action::Agent(Box::new(selected))
        );
        assert_eq!(press(&mut screen, KeyCode::Char('r')), Action::Refresh);
        assert_eq!(press(&mut screen, KeyCode::Char('q')), Action::Quit);
    }

    #[test]
    fn a_filter_narrows_as_it_is_typed_and_escape_puts_it_back() {
        let mut screen = screen(&["!1", "!2"]);

        press(&mut screen, KeyCode::Char('/'));
        // While typing, letters are text, not commands.
        type_text(&mut screen, " !2 q");
        press(&mut screen, KeyCode::Backspace);
        assert_eq!(screen.tasks().len(), 1, "applied before Enter");
        press(&mut screen, KeyCode::Esc);
        assert_eq!(screen.tasks().len(), 2);
        assert!(!screen.unsaved);

        press(&mut screen, KeyCode::Char('/'));
        type_text(&mut screen, " !2");
        press(&mut screen, KeyCode::Enter);
        assert_eq!(screen.desk.space().pane().filter.query(), "is:todo !2");
        assert!(screen.unsaved);

        // A query that does not parse keeps the prompt open and says why.
        press(&mut screen, KeyCode::Char('/'));
        type_text(&mut screen, " repos:x");
        press(&mut screen, KeyCode::Enter);
        assert!(matches!(&screen.mode, Mode::Prompt(prompt) if prompt.error.is_some()));
        press(&mut screen, KeyCode::Esc);
        assert_eq!(screen.desk.space().pane().filter.query(), "is:todo !2");
    }

    #[test]
    fn panes_and_spaces_are_rearranged_from_the_keyboard() {
        let mut screen = screen(&["!1", "!2"]);

        // A new pane: the same view, asking at once what to narrow it to.
        press(&mut screen, KeyCode::Char('n'));
        assert!(matches!(screen.mode, Mode::Prompt(_)));
        press(&mut screen, KeyCode::Enter);
        assert_eq!(screen.desk.space().panes.len(), 3);
        assert_eq!(screen.desk.space().focus, 1);

        press(&mut screen, KeyCode::Char('b'));
        assert_eq!(screen.desk.space().pane().filter.query(), "is:waiting");
        press(&mut screen, KeyCode::Char('g'));
        assert_eq!(screen.desk.space().pane().grouping, Grouping::Repo);
        press(&mut screen, KeyCode::Char('t'));
        assert_eq!(screen.desk.space().layout, Layout::Rows);
        press(&mut screen, KeyCode::Char('L'));
        assert_eq!(screen.desk.space().focus, 2);

        // Send it to space 2 and follow it.
        press(&mut screen, KeyCode::Char('m'));
        press(&mut screen, KeyCode::Char('2'));
        assert_eq!(screen.desk.current(), 1);
        assert_eq!(screen.desk.space().panes.len(), 3);
        assert_eq!(screen.desk.spaces()[0].panes.len(), 2);

        press(&mut screen, KeyCode::Char('N'));
        type_text(&mut screen, "Perso");
        press(&mut screen, KeyCode::Enter);
        assert_eq!(screen.desk.space().name, "Perso");
        press(&mut screen, KeyCode::Char('S'));
        type_text(&mut screen, "source:github");
        press(&mut screen, KeyCode::Enter);
        assert_eq!(screen.desk.space().scope.query(), "source:github");

        // Removing asks first.
        press(&mut screen, KeyCode::Char('X'));
        press(&mut screen, KeyCode::Char('n'));
        assert_eq!(screen.desk.spaces().len(), 4);
        press(&mut screen, KeyCode::Char('X'));
        press(&mut screen, KeyCode::Char('y'));
        assert_eq!(screen.desk.spaces().len(), 3);

        press(&mut screen, KeyCode::Char('x'));
        press(&mut screen, KeyCode::Char('x'));
        press(&mut screen, KeyCode::Char('x'));
        assert_eq!(screen.desk.space().panes.len(), 1);
        assert!(screen.notice.is_some(), "the last pane stays, and says so");
    }

    #[test]
    fn a_pane_is_sorted_and_reversed_from_the_keyboard() {
        let mut screen = screen(&["!1", "!2"]);
        assert_eq!(screen.desk.space().pane().sort, Sort::Urgency);
        press(&mut screen, KeyCode::Down);

        press(&mut screen, KeyCode::Char('s'));
        assert_eq!(screen.desk.space().pane().sort, Sort::Rule);
        assert_eq!(screen.cursor(), 0, "the order changed: back to the top");
        assert!(screen.unsaved);

        let first = screen.selected().unwrap().item.id();
        press(&mut screen, KeyCode::Char('v'));
        assert!(screen.desk.space().pane().reversed);
        assert_ne!(screen.selected().unwrap().item.id(), first);

        // A new order starts in its natural direction.
        press(&mut screen, KeyCode::Char('s'));
        let pane = screen.desk.space().pane();
        assert_eq!((pane.sort, pane.reversed), (Sort::Waiting, false));
    }

    #[test]
    fn a_task_is_set_aside_and_brought_back_from_the_keyboard() {
        let mut screen = screen(&["!1", "!2"]);
        let now = Timestamp(1_000);
        let first = screen.selected().unwrap().item.id();

        // p, then for how long.
        press(&mut screen, KeyCode::Char('p'));
        assert!(matches!(screen.mode, Mode::Postpone));
        let action = press(&mut screen, KeyCode::Char('3'));
        assert_eq!(action, Action::Postpone(first.clone(), Postpone::Days(3)));
        let snooze = screen.postpone(&first, Postpone::Days(3), now).unwrap();
        assert_eq!(snooze.until, Until::At(Timestamp(1_000 + 3 * 86_400)));
        assert_eq!(snooze.item, first);

        // It left the pane, and the space's count.
        assert_eq!(screen.tasks().len(), 1);
        assert_ne!(screen.selected().unwrap().item.id(), first);
        let lines = render(&mut screen, 120, 16);
        assert!(lines[0].contains(" 1 All 1 "), "{}", lines[0]);
        assert!(lines[0].contains("1 set aside"));

        // Any other key cancels.
        press(&mut screen, KeyCode::Char('p'));
        assert_eq!(press(&mut screen, KeyCode::Esc), Action::None);
        assert_eq!(screen.tasks().len(), 1);

        // A pane that asks for them shows them; p there brings one back.
        press(&mut screen, KeyCode::Char('/'));
        type_text(&mut screen, " is:snoozed");
        press(&mut screen, KeyCode::Enter);
        assert_eq!(screen.selected().unwrap().item.id(), first);
        let lines = render(&mut screen, 120, 16);
        // The screen is drawn three days after the board: a moment is left.
        assert!(
            lines
                .iter()
                .any(|line| line.contains("z set aside for 1m more"))
        );
        assert_eq!(
            press(&mut screen, KeyCode::Char('p')),
            Action::Postpone(first.clone(), Postpone::BringBack)
        );
        assert_eq!(screen.postpone(&first, Postpone::BringBack, now), None);
        assert!(screen.tasks().is_empty(), "no longer set aside");

        assert_eq!(
            screen
                .postpone("gitlab:group/web!2", Postpone::UntilItMoves, now)
                .unwrap()
                .until,
            Until::Moves
        );
    }

    #[test]
    fn tasks_show_what_their_agents_are_doing() {
        let mut screen = screen(&["!1", "!2"]);
        screen.sessions = vec![OpenSession {
            workspace: "web".into(),
            tab: "!2".into(),
            state: AgentState::Blocked,
        }];
        // Stands for the launcher's naming: workspace = repository, tab = reference.
        let of = |task: &Task, open: &[OpenSession]| {
            open.iter()
                .find(|open| open.tab == task.item.reference)
                .map(|open| open.state)
        };
        screen.mark_agents(&of);

        let lines = render(&mut screen, 120, 16);
        let row = lines.iter().find(|line| line.contains("!2")).unwrap();
        assert!(row.contains("? "), "{row}");
        press(&mut screen, KeyCode::Down);
        let lines = render(&mut screen, 120, 16);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("agent waiting for you"))
        );

        // A fresh board keeps the marks; a closed terminal loses its own.
        screen.receive(board(&["!1", "!2"]));
        screen.mark_agents(&of);
        assert_eq!(screen.selected().unwrap().agent, Some(AgentState::Blocked));
        screen.sessions.clear();
        screen.mark_agents(&of);
        assert_eq!(screen.selected().unwrap().agent, None);
    }

    #[test]
    fn a_task_and_a_space_explain_themselves() {
        let mut screen = screen(&["!1", "!2"]);
        screen.kept_in = "~/.config/aki/spaces.toml".into();

        press(&mut screen, KeyCode::Char('i'));
        let lines = render(&mut screen, 130, 44);
        let shows = |lines: &[String], text: &str| lines.iter().any(|line| line.contains(text));
        assert!(shows(&lines, "WHY IT IS HERE"));
        assert!(shows(
            &lines,
            "your review is requested · yours to do · for 3d"
        ));
        assert!(shows(&lines, "HOW ITS URGENCY ADDS UP"));
        assert!(shows(
            &lines,
            "\"Review\" has priority 10 (0 first, 100 last): 100 − 10"
        ));
        // Drawn three days after the board was read.
        assert!(shows(&lines, "3 days × 3 a day"));
        assert!(shows(&lines, "the highest urgency score first"));
        assert!(shows(&lines, "READING A LINE"));

        // Any key closes it, and is not a command.
        assert_eq!(press(&mut screen, KeyCode::Char('q')), Action::None);
        assert!(matches!(screen.mode, Mode::Normal));

        press(&mut screen, KeyCode::Char('I'));
        let lines = render(&mut screen, 130, 44);
        assert!(shows(&lines, "space 1 · All"));
        assert!(shows(&lines, "saved in ~/.config/aki/spaces.toml"));
        assert!(shows(
            &lines,
            "filter: is:todo · group: none · sort: urgency ↓"
        ));
        assert!(shows(&lines, "WHAT IT HOLDS RIGHT NOW"));
        assert!(shows(&lines, "  2 to do   0 waiting"));
        press(&mut screen, KeyCode::Esc);

        // Nothing selected: nothing to explain, and it says so.
        press(&mut screen, KeyCode::Char('3'));
        press(&mut screen, KeyCode::Char('i'));
        assert!(matches!(screen.mode, Mode::Normal));
        assert!(screen.notice.is_some());
    }

    #[test]
    fn layouts_tile_the_whole_area() {
        let area = Rect::new(0, 1, 120, 30);
        for layout in Layout::ALL {
            for count in 1..=7 {
                let rects = tile(layout, count, area);
                assert_eq!(rects.len(), count, "{layout:?} × {count}");
                let covered: u32 = rects.iter().map(|rect| rect.area()).sum();
                assert_eq!(covered, area.area(), "{layout:?} × {count} leaves no hole");
            }
        }
        let master = tile(Layout::Master, 3, area);
        assert_eq!((master[0].width, master[0].height), (66, 30));
        assert_eq!((master[1].x, master[1].height), (66, 15));
        let grid = tile(Layout::Grid, 5, area);
        assert_eq!(grid[0].width, 40);
        assert_eq!(grid[3].width, 60, "the last row shares the width");
    }

    #[test]
    fn the_screen_shows_spaces_panes_and_groups() {
        let mut screen = screen(&["!1", "!2"]);
        press(&mut screen, KeyCode::Char('g'));
        let lines = render(&mut screen, 120, 16);

        assert!(lines[0].contains(" 1 All 2 "), "{}", lines[0]);
        assert!(lines[0].contains(" 2 Gitlab 2 "));
        assert!(lines[0].contains(" 3 Jira 0 "));
        assert!(lines[1].contains("╭ 1 To do 2 "), "{}", lines[1]);
        assert!(lines[1].contains(" by repo · urgency ↓ "), "{}", lines[1]);
        assert!(lines[1].contains(" 2 Waiting 1 "));
        assert!(lines[2].contains("web group ─"), "{}", lines[2]);
        assert!(lines[3].contains("▸   ● !1"), "{}", lines[3]);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("urgency 90 = rule 90"))
        );
        // The detail of the selected task, under the panes.
        assert!(
            lines
                .iter()
                .any(|line| line.contains("your review is requested"))
        );

        // Zoomed, the focused pane takes the body.
        press(&mut screen, KeyCode::Char('z'));
        let lines = render(&mut screen, 120, 16);
        assert!(!lines[1].contains("Waiting"));
        assert!(lines.iter().any(|line| line.contains("zoomed")));
    }

    #[test]
    fn rows_fit_the_pane_whatever_the_title() {
        let mut wide = review_request("!1");
        if let WorkItem::Change(change) = &mut wide {
            change.item.title = "🐛 ⚠️⚠️ fix(parser): ".repeat(8);
        }
        let task = triage(&wide, &Rules::default(), Timestamp(900)).unwrap();
        let mut screen = Screen::new(Desk::starting(&["gitlab".into()]));
        screen.receive(Board::new(
            vec![task],
            vec!["gitlab".into()],
            Vec::new(),
            Timestamp(900),
        ));
        press(&mut screen, KeyCode::Char('z'));

        let lines = render(&mut screen, 100, 12);
        let row = lines[2].trim_end();
        assert!(row.contains("▸   Review"), "{row:?}");
        // The age is the last thing before the border, not pushed off.
        assert!(row.ends_with("  3d│"), "{row:?}");
        // Ordered by urgency, the pane shows the score its order comes from.
        assert!(row.ends_with("  90  3d│"), "{row:?}");
    }

    #[test]
    fn clicks_pick_spaces_panes_and_tasks() {
        let mut screen = screen(&["!1", "!2"]);
        let _ = render(&mut screen, 120, 16);
        let click = |column, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };

        // Second task of the first pane, then the other pane.
        screen.mouse(click(10, 3));
        assert_eq!((screen.desk.space().focus, screen.cursor()), (0, 1));
        screen.mouse(click(90, 5));
        assert_eq!(screen.desk.space().focus, 1);

        // " aki " is 5 wide, " 1 All 2 " 9, a gap, then " 2 Gitlab 2 ".
        screen.mouse(click(20, 0));
        assert_eq!(screen.desk.space().name, "Gitlab");
    }
}
