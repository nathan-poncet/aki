# myturn

Whose turn is it? One live list of what waits on **you**, and what waits on
**others**, across GitLab, GitHub and Jira.

```
 myturn  To do 6   Waiting 7    gitlab ✓  github ✓  jira ✓  live
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
cargo install --git https://github.com/nathan-poncet/myturn

myturn                # live board (the plain list when piped)
myturn list           # print once; --todo, --waiting, --json, --count
                      #             --space Pro, --filter "repo:web -kind:draft"
                      #             --sort waiting, --reverse, --fresh
myturn cache clear    # forget what was read (myturn cache path says where)
myturn spaces         # your spaces, and where they are kept
myturn doctor         # check every source
myturn rules          # every rule, as configured
myturn config init    # write ~/.config/myturn/config.toml with the defaults
```

The board refreshes by itself every 30 seconds (`refresh_seconds`, down to
5) and sends a desktop notification when something new becomes yours. None
of the three services pushes events to a personal client, so "live" means
polling, and polling is kept cheap by a cache.

## What comes first

Every task has an **urgency** score, computed by myturn and explained in
the detail pane (`urgency 155 = rule 90, waiting +60, PROJ-580 +5`):

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
configuration; `myturn rules` prints them.

## Cache

Everything read is kept in one SQLite file, `~/.cache/myturn/cache.db`:

- the board opens at once on what was known last time, then refreshes;
- `myturn list` reuses a reading younger than `cache_seconds` (60 by
  default) instead of asking again, so it can sit in a prompt or a status
  bar; `--fresh` asks anyway;
- a source that is down still shows what it said last, marked as such;
- a merge request's or pull request's discussions are read again only when
  it moved, and a ticket looked up for its priority is trusted for half an
  hour: a refresh is a handful of light requests.

It holds who spoke when and where, never what was said, and no token.
`myturn cache clear` empties it.

## Spaces, panes, layouts

The board is a desk of **spaces** (Pro, Perso, a project…), switched like
workspaces in a tiling window manager. A space has a **scope**, a filter
saying what belongs to it at all, and a set of **panes** tiled in a
**layout**. Each pane is one view of the space: a filter, and a way to
pack what it keeps.

```
 myturn  1 All 39   2 Pro 33   3 Perso 6   ◧ master · source:gitlab,jira   gitlab ✓ github ✓  live
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
| `kind:review` | tasks listed for a rule whose name contains it (`myturn rules` names them) |
| `top:draft` | the same, looking only at the most pressing reason |
| `role:author` | `author`, `reviewer`, `assignee`, `participant` |
| `by:alex` | the person on the other side |
| `heat:hot,warm` | tasks marked hot or warm (`calm` for the others) |
| `ticket:PROJ` | changes that name a ticket whose key contains it (`ticket:any`: any ticket) |
| `priority:high,highest` | tickets of that priority, and the changes that name one |
| `export` | a word in the reference, title, repository or person |
| `-term` | everything the term does not keep |

Every change is saved at once to `~/.config/myturn/spaces.toml`, which you
can also write by hand. On a first run you get "All", plus one space per
source when there are several.

## Sources

With no configuration, myturn uses gitlab.com if `glab` is installed and
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
token_env = "JIRA_API_TOKEN"   # or token_command = "security find-generic-password -s myturn-jira -w"
```

Declaring `[[sources]]` replaces the detection, so list every source you
want. Self-hosted instances take `host = "gitlab.acme.io"`. Tokens are read
from an environment variable or a command, never stored by myturn.

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
others = "wait"               # or "review", "ignore"

[nudge]
after_days = 7                # a waiting item idle that long becomes "Nudge" to do

[rules.draft_to_finish]       # any rule: enabled, bucket, priority
enabled = false

[rules.checks_running]
bucket = "todo"
```

Unknown keys and names are errors: a typo never silently disables a rule.

## Architecture

Clean architecture in one crate; dependencies point inward only.

```
src/entities    the vocabulary and the rules. Pure: no I/O, no clock, no serde.
                triage.rs decides whose turn it is and since when; urgency.rs
                scores it; rules.rs is what you can tune; filter.rs is the query
                language; desk.rs is spaces, panes, groups and orders.
src/use_cases   collect the board, watch it, open the desk, check the sources;
                ports.rs is what they need from outside (WorkSource, DeskStore, …).
src/adapters    gateways (GitLab, GitHub, Jira, spaces.toml, the SQLite cache,
                the machine),
                presenters (text, JSON, TUI, wording), the CLI parser, the config file.
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
- A change is tied to a ticket by a key in its title or branch name, not
  by the forge's or the tracker's own links. Without a tracker to confirm
  it, anything shaped like `PROJ-12` counts as one.

## License

[GPL-3.0-or-later](LICENSE): you can use, study, modify and share myturn,
commercially or not; if you distribute it, modified or not, you do so under
the same license, with the source. For a license under other terms, contact
the author.
