# aki

*À qui ?* — French for "whose turn?". One live list of what waits on **you**, and what waits on
**others**, across GitLab, GitHub and Jira.

```
 aki  To do 6   Waiting 7    gitlab ✓  github ✓  jira ✓  live
▸ Review           api!889      Typed errors for the key store               @ana       5d
  Re-review        api!741      Export an account                            @ben       2h
  Reply            web!970      The protocol in three crates                        +3  36m
  Resolve threads  api!862      Rewrite the signer                           @chloe     1d
  Fix checks       infra!80     Print node health when the probe fails                  9d
  Merge            infra!7      Trim the footprint to fit the cluster                   6w
```

## What it answers

**To do** — the next move is yours:

| | when |
|---|---|
| Review | your review is requested and you have not started |
| Re-review | you reviewed, and since then the author replied, pushed, asked again, or your approval was reset |
| Reply | someone waits for your answer in a thread, or mentioned you |
| Resolve threads | a thread you opened was answered: check it and close it |
| Fix checks / Fix conflicts | your change is red, or conflicts |
| Rebase | your change is behind the branch it targets |
| Merge | your change is approved, green and quiet |
| Ask a review | your change is ready and nobody was asked |
| Assigned, To do, In progress, Mentioned | changes and tickets that are yours |
| Finish draft | your change is still a draft |

**Waiting** — you cannot move it further:

| | when |
|---|---|
| Awaiting review | your change waits for reviewers (it names who has not looked) |
| Awaiting author | you reviewed; nothing new since |
| Awaiting reply | you answered; the other side has to close the thread |
| Checks running, Blocked, Draft | pipelines, blocked tickets, other people's drafts |

An item can be on the board for several reasons; it shows once, under its
most pressing one, with the others in the detail pane.

## Use

```sh
cargo install --git https://github.com/nathan-poncet/aki

aki                   # live board (the plain list when piped)
aki list              # print once; --todo, --waiting, --json, --count
                      #             --space Pro, --filter "repo:web -kind:draft"
                      #             --sort waiting, --reverse, --fresh
aki cache clear       # forget what was read (aki cache path says where)
aki agent web!916     # open your coding agent on a task (--dry-run to look first)
aki sounds            # the sounds a new task can ring with; `aki sounds Glass` plays one
aki spaces            # your spaces: how each is made, which repositories it holds
aki mcp               # serve the board to an assistant (`aki mcp setup` to plug it in)
aki doctor            # check every source
aki rules             # every rule, as configured
aki config init       # write ~/.config/aki/config.toml with the defaults
```

The board refreshes by itself every 30 seconds (`refresh_seconds`, down to
5) and sends a desktop notification when something new becomes yours. None
of the three services pushes events to a personal client, so "live" means
polling, and polling is kept cheap by a cache.

## What comes first

Every task has an **urgency** score, computed by aki and never left to
guess: the detail pane sums it up (`urgency 155 = rule 90, waiting +60,
PROJ-580 +5`), panes ordered by urgency show it on every line, and `i` on
a task lays it out with your own weights:

```
 WHY IT IS HERE
 ● Re-review        new commits, your approval was reset · yours to do · for 3w
 HOW ITS URGENCY ADDS UP
 rule          80  "Re-review" has priority 20 (0 first, 100 last): 100 − 20
 waiting      +60  21 days × 3 a day, at most 60
 ticket       +35  tied to PROJ-580, "high" (+30), and +5 for naming a ticket
 urgency    = 175  hot: warm ▴ from 100, hot ▲ from 130
 This pane is ordered by urgency ↓: the highest urgency score first; …
```

The same window says what every mark on a line means. The score is made of:

- **the rule** that lists it: a review to do weighs more than a draft to finish;
- **how long it has waited**, counted from when the ball arrived: since the
  review was requested, since the oldest unanswered reply, since your last
  word when you wait on someone;
- **the ticket behind it**: a change that names a ticket in its title or
  branch (`PROJ-12`) counts a little more, and takes the points of the
  ticket's priority when Jira can tell it, even for tickets that are not
  yours. A ticket weighs its own priority.

Tasks past a score are marked warm `▴`, then hot `▲`. Panes put the most
urgent first by default; `s` changes the order and `v` reverses it:

| order | first |
|---|---|
| `urgency` | the highest score |
| `rule` | the rules' own order, the freshest first among equals |
| `waiting` | what has waited longest |
| `updated` | what moved most recently |
| `created` | what was opened first |

The weights and thresholds are yours to set under `[urgency]` in the
configuration; `aki rules` prints them.

## Agents

`a` on a task, or `aki agent web!916`, hands it to your coding agent:
a terminal opens in the repository's clone, on the agent, already told
what there is to do.

```
This comes from my board of things to do.

Typed errors for the key store
https://gitlab.example/acme/api/-/merge_requests/889
api!889 on gitlab. I am a reviewer; the author is @ana.
Ticket: PROJ-440 (high).

What there is to do, most pressing first:
1. Re-review: the author replied, new commits.
   I already reviewed it. Look at what changed since: the commits pushed after my
   review and the answers to my comments. Tell me whether each of my points was
   addressed. If they all were and nothing new is wrong, say it is ready to approve.
   Start here: https://gitlab.example/acme/api/-/merge_requests/889#note_7
2. Resolve threads: 2 answered threads to check and resolve.
   …

Start by reading it where it lives, with the forge's or the tracker's own tools.
Then tell me what you found and what you propose. Do not post, approve, merge or
push anything before I say so.
```

The brief follows the task: a review to do asks for a review, answered
threads ask to be checked, a red pipeline asks to be fixed, and so on for
every rule. `instructions` adds your own closing words.

```toml
[agent]
command = "claude {prompt}"      # any agent: the brief replaces {prompt}
instructions = "Answer in French."
workspace = "{repo}"             # the names are aki's, from the task
tab = "{ticket} {ref}"
```

Where the terminal opens is a **launcher**:

- **Herdr**, when aki runs inside it: one workspace per repository, one
  tab per piece of work, named from the templates above and created when
  missing. A workspace or a tab is reused only if it bears exactly that
  name. A tab that is already open is never doubled: the agent running in
  it is told the new brief (you fixed the pipeline there yesterday, today
  it has to rebase); if the agent is gone, it is started again in that
  tab; and nothing is typed into a tab busy with something else.
- **tmux** and **TUIOS**, the same way: a tmux session per repository and
  a window per piece of work; a TUIOS workspace (a free one, given the
  repository's name) and a named window. The same reuse rules apply. TUIOS
  reports what its agents are doing, like Herdr; tmux only knows that the
  window is open.
- **the terminal itself** otherwise: a new window or tab of Ghostty,
  WezTerm, kitty, iTerm or Terminal, whichever aki runs in, or your own
  command (`terminal = ["alacritty", "-e", "{command}"]`).
- **a Claude conversation**, for whoever does not live in a terminal
  (`launcher = "claude"`, and what `auto` falls back on when there is no
  terminal): a new conversation opens on the brief, at `chat_url`
  (`https://claude.ai/new?q={prompt}`, or `claude://claude.ai/new?q={prompt}`
  for the desktop app).

The clone is found by its git remote under the folders where you keep your
code (`roots`), or named in `[agent.repos]`. The agent is started by your
own login shell, so it finds the same `PATH` as at your prompt.

Once an agent is open on a task, the board shows what it is doing, next
to the task: `◐` working, `?` waiting for you, `✓` done and not looked at
yet, `·` idle, `○` its terminal is open. It is read from the launcher every
two seconds, by the same names: Herdr and TUIOS tell, tmux says only that
the window is open, a plain terminal cannot tell anything.
`agent:blocked` in a pane's filter gathers the agents that wait for you.

A launcher is one small adapter: something that can open a named place
on a command or a brief (`Launcher` in `src/use_cases/ports.rs`).

## Without a terminal: the board in an assistant

`aki mcp` serves the board over the Model Context Protocol, so that
Claude Desktop, Claude Code or any MCP client can be asked "what waits on
me?", "why is that one first?", "set it aside until Monday", "start on
the review of web!916". `aki mcp setup` prints what to paste where:

```json
{ "mcpServers": { "aki": { "command": "/path/to/aki", "args": ["mcp"] } } }
```

| tool | does |
|---|---|
| `list_tasks` | the board, most urgent first; `side`, `space`, `filter`, `sort`, `limit` |
| `get_task` | one task: its reasons, its urgency line by line, and the brief of what is expected |
| `list_spaces` | your spaces, their panes, the repositories they hold |
| `explain_rules` | every rule and how urgency is computed |
| `set_aside`, `bring_back` | hide a task for some days or until it moves; bring it back |
| `start_agent` | open your agent on a task, through the launcher you chose |

Reading is always allowed; the rest is yours to say:

```toml
[mcp]
enabled = true
set_aside = true
start_agent = true     # with launcher = "claude": a new conversation on the brief
```

## Setting aside, and being told

`p` sets the selected task aside: until tomorrow (`t`), for a week (`w`),
for one to nine days, or until anything happens on it (`m`). It leaves the
panes, the counts and the notifications, and comes back by itself. A pane
filtered on `is:snoozed` shows what is aside; `p` there brings a task back.

While the board is open, something new to do rings, once per refresh, with
the sound of the most pressing new task; a desktop notification says what.

```toml
[sounds]
new_task = "Glass"               # a system sound, or "~/sounds/ding.wav"

[sounds.rules]
review_requested = "Ping"        # a sound of its own for a rule
draft_to_finish = "none"         # or silence
```

## Cache

Everything read is kept in one SQLite file, `~/.cache/aki/cache.db`:

- the board opens at once on what was known last time, then refreshes;
- `aki list` reuses a reading younger than `cache_seconds` (60 by
  default) instead of asking again, so it can sit in a prompt or a status
  bar; `--fresh` asks anyway;
- a source that is down still shows what it said last, marked as such;
- a merge request's or pull request's discussions are read again only when
  it moved, and a ticket looked up for its priority is trusted for half an
  hour: a refresh is a handful of light requests.

It holds who spoke when and where, never what was said, and no token.
`aki cache clear` empties it.

## Spaces, panes, layouts

The board is a desk of **spaces** (Pro, Perso, a project…), switched like
workspaces in a tiling window manager. A space has a **scope**, a filter
saying what belongs to it at all, and a set of **panes** tiled in a
**layout**. Each pane is one view of the space: a filter, and a way to
pack what it keeps.

```
 aki  1 All 39   2 Pro 33   3 Perso 6   ◧ master · source:gitlab,jira   gitlab ✓ github ✓  live
╭ 1 To do 13 ────────────────────────────── by repo ╮╭ 2 Waiting 5 ───────────────────────╮
│  api acme/platform ────────────────────────────  7││  ● api!976    Error codes name…  57m│
│▸ Review       !889   Typed errors for the key…  5d││  ● web!929    Exchange builds …  22h│
│  Re-review    !893   Update images and charts  15h│╰────────────────────────────────────╯
│  Reply        !878   Refuse sessions beyond… +3 6d│╭ 3 My drafts 16 ─────────── by repo ╮
│  web acme/platform ────────────────────────────  2││  web acme/platform ────────────  14│
│  Review       !916   v1 api changes            13d││  ● !984   Each side reads and…   3h│
╰───────────────────────────────────────────────────╯╰────────────────────────────────────╯
```

| keys | |
|---|---|
| `1`-`9`, `,` `.` | show a space (or click it in the dock) |
| `←` `→`, `Tab` | focus a pane (or click it) |
| `↑` `↓`, `PgUp` `PgDn`, `Home` `End` | move among its tasks (or click, or scroll) |
| `Enter` | open in the browser, on the thread itself when there is one |
| `a` | open your coding agent on it, briefed, in its clone |
| `p` | set it aside: until tomorrow, for some days, or until it moves; again to bring it back |
| `/` | filter the pane, applied as you type |
| `b` | its side: to do → waiting → both |
| `g` | its grouping: none → repo → kind → source → person → role |
| `s` `v` | its order: urgency → rule → waiting → updated → created · reverse it |
| `n` `x` | new pane beside this one (same view, ready to narrow) · close it |
| `Shift+←` `Shift+→` | move the pane among the others |
| `m` then `1`-`9` | send the pane to another space |
| `e` | name the pane |
| `z` `d` | zoom the pane · show or hide the detail |
| `t` | the space's layout: columns → rows → master → grid |
| `N` `R` `X` | new space · rename · remove |
| `S` | the space's scope |
| `<` `>` | move the space among the others |
| `r` `?` `q` | refresh now · every key · quit |

Filters are short queries; terms must all hold:

```
is:todo source:gitlab repo:web,api -top:draft by:alex export
```

| term | keeps |
|---|---|
| `is:todo`, `is:waiting` | one side of the board |
| `source:gitlab,jira` | items from any of these sources |
| `repo:web` | repositories or projects whose path contains it |
| `kind:review` | tasks listed for a rule whose name contains it (`aki rules` names them) |
| `top:draft` | the same, looking only at the most pressing reason |
| `role:author` | `author`, `reviewer`, `assignee`, `participant` |
| `by:alex` | the person on the other side |
| `heat:hot,warm` | tasks marked hot or warm (`calm` for the others) |
| `ticket:PROJ` | changes that name a ticket whose key contains it (`ticket:any`: any ticket) |
| `priority:high,highest` | tickets of that priority, and the changes that name one |
| `agent:blocked` | tasks whose agent is `working`, `blocked`, `done`, `idle`, `open` (`agent:any`: has one) |
| `is:snoozed` | what you set aside, which nothing else shows |
| `is:draft` | draft changes (`-is:draft` leaves them out) |
| `export` | a word in the reference, title, repository or person |
| `-term` | everything the term does not keep |

Every change is saved at once to `~/.config/aki/spaces.toml`, which you
can also write by hand. On a first run you get "All", plus one space per
source when there are several; from then on spaces are what you make them.
`I` in the board, or `aki spaces`, says how the shown space is made
(scope, layout, panes) and which repositories it holds right now;
`aki spaces reset` starts over from your sources, keeping the old file
as `spaces.toml.bak`.

Drafts of yours can have a pane of their own. With `mine = "apart"` under
`[drafts]`, they leave "to do", the counts and the notifications, and show
only in panes that ask for them (`is:draft`); `"ignore"` takes them off the
board, `"todo"` (the default) leaves them with the rest.

## Sources

With no configuration, aki uses gitlab.com if `glab` is installed and
github.com if `gh` is, with their own logins. Jira needs three lines:

```toml
[[sources]]
kind = "gitlab"

[[sources]]
kind = "github"

[[sources]]
kind = "jira"
site = "https://your-company.atlassian.net"
email = "you@your-company.com"
token_env = "JIRA_API_TOKEN"   # or token_command = "security find-generic-password -s aki-jira -w"
```

Declaring `[[sources]]` replaces the detection, so list every source you
want. Self-hosted instances take `host = "gitlab.acme.io"`. Tokens are read
from an environment variable or a command, never stored by aki.

## Rules

Everything above is a rule you can change in the configuration file
(`config/example.toml` documents every key):

```toml
[threads]
who_resolves = "reviewer"     # or "author", "anyone"

[re_review]
triggers = ["author_replied", "new_commits", "re_requested", "approval_reset"]
after_approval = false

[drafts]
mine = "todo"                 # or "apart" (only in is:draft panes), "ignore"
others = "wait"               # or "review", "ignore"

[nudge]
after_days = 7                # a waiting item idle that long becomes "Nudge" to do

[rules.draft_to_finish]       # any rule: enabled, bucket, priority
enabled = false

[rules.checks_running]
bucket = "todo"
```

Unknown keys and names are errors: a typo never silently disables a rule.

## Site

The site (landing page and documentation) is in `docs/`, served by GitHub
Pages at https://nathan-poncet.github.io/aki/. It is generated from `site/`:
pages are HTML fragments in `site/pages`, and `python3 site/build.py`
rebuilds `docs/` (standard library only).

## Architecture

Clean architecture in one crate; dependencies point inward only.

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
                Herdr, tmux, TUIOS, terminal and conversation launchers, local clones, the
                machine), presenters (text, JSON, TUI, wording), controllers (the
                CLI parser, the MCP server), the config file.
src/app.rs      the composition root: the only place that knows every ring.
```

Adding a source (Linear, Bitbucket, Gitea…) is one gateway implementing
`WorkSource`: map what the service says to a `ChangeRequest` or a `Ticket`,
and every rule applies to it unchanged.

## Known limits

- GitLab: merge requests where you only commented, without being author,
  assignee or reviewer, are not listed.
- GitLab tells its history in English system notes ("added 2 commits",
  "requested review from"); pushes and re-requests are read from those.
- Jira: Cloud only (API token). "Flagged" is not read; use
  `tickets.waiting_statuses` or "is blocked by" links.
- A change's being behind its target is the forge's word: GitLab says it
  for every merge request, GitHub only where the branch must be up to date
  to merge.
- Agents: the Herdr, tmux and TUIOS launchers were run for real; the terminal ones are
  built from each terminal's documented command line, and the conversation
  one relies on the address prefilling a new chat. The MCP server speaks
  the protocol over standard input and output only. In Herdr, a session
  restored after a restart may not bring an agent's tab back as it was.
- A change is tied to a ticket by a key in its title or branch name, not
  by the forge's or the tracker's own links. Without a tracker to confirm
  it, anything shaped like `PROJ-12` counts as one.

## License

[GPL-3.0-or-later](LICENSE): you can use, study, modify and share aki,
commercially or not; if you distribute it, modified or not, you do so under
the same license, with the source. For a license under other terms, contact
the author.
