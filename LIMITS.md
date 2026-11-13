# Known limits

## Sources

- **GitLab**: merge requests where you only commented, without being author,
  assignee or reviewer, are not listed.
- **GitLab** tells its history in English system notes ("added 2 commits",
  "requested review from"); pushes and re-requests are read from those.
- **Jira**: Cloud only (API token). "Flagged" is not read; use
  `tickets.waiting_statuses` or "is blocked by" links.
- A change's being behind its target is the forge's word: GitLab says it for
  every merge request, GitHub only where the branch must be up to date to
  merge.
- A change is tied to a ticket by a key in its title or branch name, not by
  the forge's or the tracker's own links. Without a tracker to confirm it,
  anything shaped like `PROJ-12` counts as one.

## Agents

- The Herdr, tmux and TUIOS launchers were run for real. The terminal ones
  are built from each terminal's documented command line, and the
  conversation one relies on the address prefilling a new chat.
- tmux only knows that an agent's window is open; Herdr and TUIOS also say
  what the agent is doing.
- In Herdr, a session restored after a restart may not bring an agent's tab
  back as it was.

## Assistants

- The MCP server speaks the protocol over standard input and output only.

## Live

- None of the three services pushes events to a personal client, so "live"
  means polling: every 30 seconds by default (`refresh_seconds`, down to 5).
