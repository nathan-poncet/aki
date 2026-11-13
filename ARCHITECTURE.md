# Architecture

Clean architecture in one crate. Dependencies point inward only: the rules
know nothing of GitLab, of the terminal, or of files.

```
src/entities    the vocabulary and the rules. Pure: no I/O, no clock, no serde.
                triage.rs decides whose turn it is and since when; urgency.rs
                scores it; rules.rs is what you can tune; filter.rs is the query
                language; desk.rs is spaces, panes, groups and orders; snooze.rs
                is what is set aside; handoff.rs is how agents are named.
src/use_cases   collect the board, watch it, open the desk, launch an agent,
                check the sources; ports.rs is what they need from outside
                (WorkSource, DeskStore, Launcher, Checkouts, …).
src/adapters    gateways (GitLab, GitHub, Jira, spaces.toml, the SQLite cache,
                Herdr, tmux, TUIOS, terminal and conversation launchers, local
                clones, the machine), presenters (text, JSON, TUI, wording),
                controllers (the CLI parser, the MCP server), the config file.
src/app.rs      the composition root: the only place that knows every ring.
```

## How a board is made

1. Every configured source is a `WorkSource`. It returns what it knows as
   `ChangeRequest`s and `Ticket`s: who wrote, who reviewed, who spoke last
   in each thread, what the checks say.
2. `triage.rs` applies the rules to each item and gives the reasons it is on
   the board, each with a side (to do or waiting) and the moment the ball
   arrived.
3. `urgency.rs` scores it: the rule, how long it has waited, the ticket
   behind it.
4. A presenter shows the result: the terminal interface, plain text, JSON,
   or the MCP server's answers.

## The cache

Everything read is kept in one SQLite file, `~/.cache/aki/cache.db`:

- the board opens at once on what was known last time, then refreshes;
- `aki list` reuses a reading younger than `cache_seconds` (60 by default)
  instead of asking again, so it can sit in a prompt or a status bar;
  `--fresh` asks anyway;
- a source that is down still shows what it said last, marked as such;
- a merge request's or pull request's discussions are read again only when
  it moved, and a ticket looked up for its priority is trusted for half an
  hour.

It holds who spoke when and where, never what was said, and no token.
`aki cache clear` empties it; `aki cache path` says where it is.

## Launchers

Where an agent opens is a `Launcher` (`src/use_cases/ports.rs`): something
that can open a named place on a command or a brief, and say which places
are open. Herdr, tmux and TUIOS each map a repository to a workspace or a
session, and a task to a tab or a window. They share the same rules for a
place that already exists: the agent running in it is told the new brief; if
the agent is gone, it is started again there; and nothing is typed into a
place busy with something else.

`launcher = "auto"` picks the multiplexer aki runs inside. TUIOS is looked
for before Herdr, because TUIOS panes also announce themselves as Herdr.
