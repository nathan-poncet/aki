// The board on the landing page: aki's interface, redrawn in the browser
// from sample data. Keys and clicks do what they do in the real one.

(function () {
  const term = document.querySelector(".term");
  if (!term) return;
  const screen = term.querySelector(".screen");

  // ── the sample board ─────────────────────────────────────────────
  const RULES = {
    "Review": 10, "Re-review": 20, "Reply": 30, "Resolve threads": 40, "Fix checks": 50,
    "Rebase": 57, "Merge": 60, "In progress": 75, "To do": 80, "Finish draft": 90,
    "Awaiting review": 10, "Awaiting author": 20, "Checks running": 40, "Blocked": 50,
  };
  const TASKS = [
    { id: 1, src: "gitlab", repo: "acme/api", ref: "!889", title: "Typed errors for the key store",
      side: "todo", role: "reviewer", who: "ana", days: 5, ticket: ["PROJ-440", "high", 30],
      why: [["Re-review", "the author replied, new commits"],
            ["Resolve threads", "2 answered threads to check and resolve"]] },
    { id: 2, src: "gitlab", repo: "acme/api", ref: "!916", title: "Version the public API",
      side: "todo", role: "reviewer", who: "ben", days: 21, ticket: ["PROJ-580", null, 0],
      why: [["Review", "your review is requested"]] },
    { id: 3, src: "gitlab", repo: "acme/web", ref: "!970", title: "The protocol in three crates",
      side: "todo", role: "author", days: 0.03,
      why: [["Reply", "2 threads wait for your answer"],
            ["Awaiting review", "1 approval missing · not reviewed yet: till"]] },
    { id: 4, src: "gitlab", repo: "acme/infra", ref: "!80", title: "Print node health when the probe fails",
      side: "todo", role: "author", days: 9,
      why: [["Fix checks", "the pipeline failed"]] },
    { id: 5, src: "github", repo: "nat/notes", ref: "#12", title: "Offline search index",
      side: "todo", role: "author", days: 42,
      why: [["Merge", "approved, green, no open thread"]] },
    { id: 6, src: "jira", repo: "PROJ", ref: "PROJ-612", title: "Rotate the signing keys",
      side: "todo", role: "assignee", days: 2, own: ["highest", 50],
      why: [["To do", "status: Selected for development"]] },
    { id: 7, src: "gitlab", repo: "acme/web", ref: "!953", title: "Draft: Retry policy for uploads",
      side: "todo", role: "author", days: 6,
      why: [["Rebase", "behind its target branch"], ["Finish draft", "still a draft"]] },
    { id: 13, src: "jira", repo: "PROJ", ref: "PROJ-601", title: "Retry policy for uploads",
      side: "todo", role: "assignee", days: 6, own: ["medium", 10],
      why: [["In progress", "status: In progress"]] },
    { id: 14, src: "jira", repo: "PROJ", ref: "PROJ-577", title: "Document the v1 API",
      side: "todo", role: "assignee", days: 12, own: ["low", 0],
      why: [["To do", "status: To do"]] },
    { id: 8, src: "gitlab", repo: "acme/api", ref: "!976", title: "Error codes name the failure",
      side: "waiting", role: "author", days: 0.12,
      why: [["Awaiting review", "1 approval missing · not reviewed yet: till"]] },
    { id: 9, src: "gitlab", repo: "acme/web", ref: "!914", title: "Reuse connections to the bridge",
      side: "waiting", role: "reviewer", who: "chloe", days: 1,
      why: [["Awaiting author", "2 threads of yours unanswered"]] },
    { id: 10, src: "github", repo: "nat/dotfiles", ref: "#40", title: "Add fish completions",
      side: "waiting", role: "author", days: 49,
      why: [["Awaiting review", "waiting for a maintainer"]] },
    { id: 11, src: "jira", repo: "PROJ", ref: "PROJ-498", title: "Audit log export",
      side: "waiting", role: "assignee", days: 4, own: ["medium", 10],
      why: [["Blocked", "blocked by PROJ-471"]] },
    { id: 12, src: "gitlab", repo: "acme/infra", ref: "!7", title: "Trim the footprint to fit the cluster",
      side: "waiting", role: "author", days: 0.008,
      why: [["Checks running", "the pipeline is running"]] },
  ];
  const SPACES = [
    { name: "All", scope: "everything", keep: () => true },
    { name: "Work", scope: "source:gitlab,jira", keep: (task) => task.src !== "github" },
    { name: "Personal", scope: "source:github", keep: (task) => task.src === "github" },
  ];
  const SIDES = ["todo", "waiting"];

  const parts = (task) => {
    const rule = 100 - RULES[task.why[0][0]];
    const whole = Math.floor(task.days);
    const wait = Math.min(whole * 3, 60);
    const ticket = (task.own ? task.own[1] : 0) + (task.ticket ? task.ticket[2] + 5 : 0);
    const score = rule + wait + ticket;
    return { rule, whole, wait, ticket, score, heat: score >= 130 ? "hot" : score >= 100 ? "warm" : "calm" };
  };
  const age = (days) => {
    const minutes = Math.round(days * 1440);
    if (minutes < 60) return `${Math.max(minutes, 1)}m`;
    if (minutes < 1440) return `${Math.floor(minutes / 60)}h`;
    return days < 14 ? `${Math.floor(days)}d` : `${Math.floor(days / 7)}w`;
  };
  const short = (task) => (task.src === "jira" ? task.ref : task.repo.split("/").pop() + task.ref);

  // ── state ────────────────────────────────────────────────────────
  const fresh = () => ({
    space: 0, pane: 0, cursor: [0, 0], group: ["none", "none"], sort: ["urgency", "urgency"],
    pop: null, asking: false, aside: new Set(), agents: new Map(), said: null, refused: null,
  });
  let state = fresh();
  let narrow = false;

  const listing = (pane) => {
    const tasks = TASKS.filter((task) =>
      task.side === SIDES[pane] && SPACES[state.space].keep(task) && !state.aside.has(task.id));
    const by = state.sort[pane] === "urgency"
      ? (a, b) => parts(b).score - parts(a).score || b.days - a.days
      : (a, b) => b.days - a.days;
    tasks.sort(by);
    if (state.group[pane] === "repo") {
      const order = [...new Set(tasks.map((task) => task.repo))];
      tasks.sort((a, b) => order.indexOf(a.repo) - order.indexOf(b.repo));
    }
    return tasks;
  };
  const selected = () => listing(state.pane)[state.cursor[state.pane]];
  const clamp = () => {
    [0, 1].forEach((pane) => {
      state.cursor[pane] = Math.max(0, Math.min(state.cursor[pane], listing(pane).length - 1));
    });
  };

  // ── drawing ──────────────────────────────────────────────────────
  const esc = (text) => [...text].map((char) => {
    if (char === "&") return "&amp;";
    if (char === "<") return "&lt;";
    return char.charCodeAt(0) > 126 ? `<i>${char}</i>` : char;
  }).join("");
  const seg = (text, cls) => (cls ? `<span class="${cls}">${esc(text)}</span>` : esc(text));
  const fit = (text, width) => (text.length <= width ? text.padEnd(width) : text.slice(0, Math.max(width - 1, 0)) + "…");
  const SIDE_CLASS = { todo: "t-you", waiting: "t-them" };

  function paneLines(pane, width, rows) {
    const inner = width - 2;
    const tasks = listing(pane);
    const focused = pane === state.pane;
    const edge = focused ? "t-focus" : "t-dim";
    const name = pane === 0 ? "To do" : "Waiting";
    const order = `${state.group[pane] === "repo" ? "by repo · " : ""}${state.sort[pane]} ↓`;
    const head = ` ${pane + 1} `;
    const count = ` ${tasks.length} `;
    const tail = ` ${order} `;
    const fill = Math.max(inner - head.length - name.length - count.length - tail.length, 1);
    const lines = [{
      html: seg("╭", edge) + seg(head, edge) + seg(name, `${SIDE_CLASS[SIDES[pane]]} t-b`) + seg(count, "t-dim")
        + seg("─".repeat(fill), edge) + seg(tail, "t-dim") + seg("╮", edge),
    }];

    const agents = tasks.some((task) => state.agents.has(task.id));
    const scored = state.sort[pane] === "urgency";
    const grouped = state.group[pane] === "repo";
    const refWidth = grouped ? 9 : 13;
    let body = [];
    let last = null;
    tasks.forEach((task, index) => {
      if (grouped && task.repo !== last) {
        last = task.repo;
        const repo = task.repo.split("/").pop();
        const owner = task.repo.includes("/") ? task.repo.split("/")[0] + " " : "";
        const many = ` ${tasks.filter((other) => other.repo === task.repo).length}`;
        const rule = Math.max(inner - 2 - repo.length - 1 - owner.length - many.length, 1);
        body.push({
          html: "  " + seg(repo + " ", "t-b") + seg(owner + "─".repeat(rule) + many, "t-dim"),
        });
      }
      const score = parts(task);
      const here = focused && index === state.cursor[pane];
      const mark = score.heat === "hot" ? "▲ " : score.heat === "warm" ? "▴ " : "  ";
      const more = task.why.length > 1 ? `+${task.why.length - 1}` : "";
      let room = inner - 2 - 2 - 16 - refWidth - 1 - 3 - 4 - (agents ? 2 : 0) - (scored ? 4 : 0);
      const agent = state.agents.get(task.id);
      let html = seg(here ? "▸ " : "  ", "t-focus")
        + seg(mark, score.heat === "hot" ? "t-hot" : "t-you")
        + (agents ? seg(agent === "working" ? "◐ " : "  ", "t-ok t-b") : "")
        + seg(fit(task.why[0][0], 16), SIDE_CLASS[task.side])
        + seg(fit(grouped ? task.ref : short(task), refWidth) + " ", "t-b")
        + seg(fit(task.title, Math.max(room, 4)))
        + seg(fit(more, 3), "t-dim")
        + (scored ? seg(String(score.score).padStart(4), score.heat === "hot" ? "t-hot" : score.heat === "warm" ? "t-you" : "t-dim") : "")
        + seg(age(task.days).padStart(4), "t-dim");
      body.push({ html, task: index, here });
    });
    if (!tasks.length) {
      body.push({ html: seg(fit(pane === 0 ? " nothing waits on you here" : " you wait on nothing here", inner), "t-dim") });
    }
    // Keep the selected task in view.
    const at = body.findIndex((line) => line.here);
    const from = Math.max(0, Math.min(at - rows + 2, body.length - rows));
    body = body.slice(at < rows - 1 ? 0 : from, (at < rows - 1 ? 0 : from) + rows);
    while (body.length < rows) body.push({ html: " ".repeat(inner) });
    body.forEach((line) => {
      lines.push({
        html: seg("│", edge) + (line.here ? `<span class="t-sel">${line.html}</span>` : line.html) + seg("│", edge),
        task: line.task, pane,
      });
    });
    lines.push({ html: seg("╰" + "─".repeat(inner) + "╯", edge) });
    return lines;
  }

  function dock(width) {
    let html = seg(" aki ", "t-b");
    SPACES.forEach((space, index) => {
      const todo = TASKS.filter((task) => task.side === "todo" && space.keep(task) && !state.aside.has(task.id)).length;
      const text = ` ${index + 1} ${space.name} ${todo} `;
      html += `<span data-space="${index}" class="${index === state.space ? "t-tab" : "t-dim"}">${esc(text)}</span> `;
    });
    let rest = "";
    if (SPACES[state.space].scope !== "everything") rest += ` · ${SPACES[state.space].scope}`;
    if (state.aside.size) rest += ` · ${state.aside.size} set aside`;
    html += seg(narrow ? rest : ` ▥ columns${rest}   `, "t-dim");
    if (!narrow) html += seg("gitlab ✓ github ✓ jira ✓ ", "t-ok") + seg(" live", "t-dim");
    return { html };
  }

  function detail(width) {
    const task = selected();
    const lines = [{ html: seg("─".repeat(width), "t-dim") }];
    if (task) {
      const score = parts(task);
      lines.push({ html: seg(short(task), "t-b") + "  " + seg(fit(task.title, width - short(task).length - 2)) });
      lines.push({
        html: seg(fit(`${task.src} · ${task.repo} · you: ${task.role}${task.who ? ` · by @${task.who}` : ""}`, width), "t-dim"),
      });
      task.why.slice(0, 2).forEach(([kind, why]) => {
        lines.push({ html: seg(`● ${kind.padEnd(16)} `, SIDE_CLASS[task.side]) + seg(fit(why, width - 19)) });
      });
      const sum = [`rule ${score.rule}`];
      if (score.wait) sum.push(`waiting +${score.wait}`);
      if (score.ticket) sum.push(`${task.ticket ? task.ticket[0] : "priority " + task.own[0]} +${score.ticket}`);
      lines.push({
        html: seg(score.heat === "hot" ? "▲ " : score.heat === "warm" ? "▴ " : "  ", score.heat === "hot" ? "t-hot" : "t-you")
          + seg(fit(`urgency ${score.score} = ${sum.join(", ")} · i explains`, width - 2), "t-dim"),
      });
    }
    while (lines.length < 6) lines.push({ html: " " });
    return lines;
  }

  function footer(width) {
    if (state.asking) {
      return { html: seg(fit(" set aside until: t tomorrow · w a week · 1-9 days · m it moves · any other key cancels", width), "t-b") };
    }
    if (state.refused) return { html: seg(fit(" " + state.refused, width), "t-hot") };
    if (state.said) return { html: seg(fit(" " + state.said, width), "t-ok") };
    const hint = narrow
      ? " ←→ pane · ↑↓ task · i explain · ? keys"
      : " ←→ pane · ↑↓ task · 1-3 space · Enter open · a agent · i explain · p set aside · g group · s sort · ? keys";
    return { html: seg(fit(hint, width), "t-dim") };
  }

  function popup() {
    const task = selected();
    const rows = [];
    const title = (text) => rows.push(seg(text, "t-focus t-b"));
    const note = (text) => rows.push(seg(text, "t-dim"));
    let name = "";
    if (state.pop === "explain" && task) {
      const score = parts(task);
      name = "this task";
      rows.push(seg(short(task), "t-b") + "  " + seg(task.title), "");
      title("WHY IT IS HERE");
      task.why.forEach(([kind, why], index) => {
        const side = RULES[kind] !== undefined && ["Awaiting review", "Awaiting author", "Checks running", "Blocked"].includes(kind)
          ? "waiting" : "todo";
        rows.push(seg(`● ${kind.padEnd(16)} `, SIDE_CLASS[side]) + seg(why)
          + seg(` · ${side === "todo" ? "yours to do" : "waits on others"}${index === 0 ? ` · for ${age(task.days)}` : ""}`, "t-dim"));
      });
      note(task.why.length > 1
        ? "The first one is the most pressing: it names the task and decides its side."
        : "This reason names the task and decides its side.");
      rows.push("");
      title("HOW ITS URGENCY ADDS UP");
      const kind = task.why[0][0];
      const row = (what, points, how, bold) =>
        rows.push(seg(`${what.padEnd(9)} ${points.padStart(6)}  `, bold ? "t-b" : "") + seg(how, "t-dim"));
      row("rule", String(score.rule), `"${kind}" has priority ${RULES[kind]} (0 first, 100 last): 100 − ${RULES[kind]}`);
      row("waiting", `+${score.wait}`, `${score.whole} day${score.whole === 1 ? "" : "s"} × 3 a day${score.whole * 3 > 60 ? ", at most 60" : ""}`);
      if (task.own) row("priority", `+${task.own[1]}`, `this ticket is "${task.own[0]}"`);
      if (task.ticket) {
        row("ticket", `+${task.ticket[2] + 5}`, task.ticket[1]
          ? `tied to ${task.ticket[0]}, "${task.ticket[1]}" (+${task.ticket[2]}), and +5 for naming a ticket`
          : `tied to ${task.ticket[0]} (priority unknown): +5 for naming a ticket`);
      }
      row("urgency", `= ${score.score}`, `${score.heat}: warm ▴ from 100, hot ▲ from 130`, true);
      note(`This pane is ordered by ${state.sort[state.pane]} ↓: ${state.sort[state.pane] === "urgency"
        ? "the highest urgency score first." : "what has waited longest first."}`);
      note("The weights are yours: [urgency] and [rules.*] in the configuration.");
    } else if (state.pop === "space") {
      const space = SPACES[state.space];
      name = `space ${state.space + 1} · ${space.name}`;
      title("HOW IT IS MADE");
      rows.push(seg("name    ", "t-b") + seg(space.name) + seg("  R renames", "t-dim"));
      rows.push(seg("scope   ", "t-b") + seg(space.scope) + seg("  S changes what belongs here at all", "t-dim"));
      rows.push(seg("layout  ", "t-b") + seg("columns") + seg("  t changes", "t-dim"));
      note("Every change is saved in ~/.config/aki/spaces.toml.");
      rows.push("");
      title("ITS PANES");
      [0, 1].forEach((pane) => rows.push(seg(`${pane + 1} `, "t-focus")
        + seg((pane ? "Waiting" : "To do").padEnd(9), "t-b")
        + seg(`filter: is:${SIDES[pane]} · group: ${state.group[pane]} · sort: ${state.sort[pane]} ↓`)));
      rows.push("");
      title("WHAT IT HOLDS RIGHT NOW");
      const held = new Map();
      TASKS.filter(space.keep).forEach((task) => {
        const key = `${task.src.padEnd(7)} ${task.repo}`;
        const entry = held.get(key) || [0, 0];
        entry[task.side === "todo" ? 0 : 1] += 1;
        held.set(key, entry);
      });
      [...held].sort((a, b) => b[1][0] + b[1][1] - a[1][0] - a[1][1]).forEach(([key, [todo, waiting]]) => {
        rows.push(seg(key.padEnd(24)) + seg(`${String(todo).padStart(2)} to do `, "t-you") + seg(`${String(waiting).padStart(2)} waiting`, "t-them"));
      });
    } else if (state.pop === "keys") {
      name = "keys";
      [["↑ ↓", "the task, within the pane"], ["← →  Tab", "the pane"], ["1-3", "the space"],
        ["Enter", "open in the browser"], ["a", "open your coding agent on it, briefed"],
        ["i  I", "explain the task · explain the space"], ["p", "set it aside"],
        ["g  s", "group by repository · change the order"], ["Esc", "close a window"],
      ].forEach(([keys, what]) => rows.push(seg(keys.padEnd(10), "t-focus") + seg(what)));
      note("The real board has more: filters, new panes, layouts, new spaces.");
    } else {
      return "";
    }
    return `<pre class="pop" style="border:1px solid var(--t-focus);border-radius:8px;padding:10px 14px;white-space:pre-wrap">`
      + seg(name, "t-focus") + seg("   any key closes", "t-dim") + "\n\n" + rows.join("\n") + "</pre>";
  }

  function draw() {
    const box = screen.clientWidth - 24;
    narrow = box < 620;
    // As many columns as the width gives at a readable size, an even count.
    const cols = narrow ? 58 : Math.max(108, Math.min(150, Math.floor(box / 7.5 / 2) * 2));
    screen.style.setProperty("--fs", Math.min(13.5, box / (cols * 0.602)).toFixed(2) + "px");
    clamp();

    const lines = [dock(cols)];
    const rows = narrow ? 9 : 14;
    if (narrow) {
      lines.push(...paneLines(state.pane, cols, rows));
    } else {
      const left = paneLines(0, cols / 2, rows);
      const right = paneLines(1, cols / 2, rows);
      left.forEach((line, index) => {
        const other = right[index];
        lines.push({ html: `<span data-pane="0"${line.task !== undefined ? ` data-task="${line.task}"` : ""}>${line.html}</span>`
          + `<span data-pane="1"${other.task !== undefined ? ` data-task="${other.task}"` : ""}>${other.html}</span>` });
      });
    }
    lines.push(...detail(cols), footer(cols));
    screen.innerHTML = "<pre>" + lines.map((line) => {
      const attrs = narrow && line.task !== undefined ? ` data-pane="${line.pane}" data-task="${line.task}"` : "";
      return `<span class="ln"${attrs}>${line.html}</span>`;
    }).join("") + "</pre>" + popup();
  }

  // ── keys ─────────────────────────────────────────────────────────
  function press(key) {
    state.said = null;
    state.refused = null;
    if (state.pop) {
      state.pop = null;
      return draw();
    }
    const task = selected();
    if (state.asking) {
      state.asking = false;
      const until = key === "t" ? "set aside for 1d more" : key === "w" ? "set aside for 7d more"
        : key === "m" ? "set aside until it moves" : /^[1-9]$/.test(key) ? `set aside for ${key}d more` : null;
      if (until && task) {
        state.aside.add(task.id);
        state.said = until;
      }
      return draw();
    }
    const move = (by) => { state.cursor[state.pane] += by; };
    switch (key) {
      case "ArrowDown": case "j": move(1); break;
      case "ArrowUp": case "k": move(-1); break;
      case "ArrowLeft": case "ArrowRight": case "Tab": case "h": case "l": state.pane = 1 - state.pane; break;
      case "1": case "2": case "3": state.space = Number(key) - 1; break;
      case "i":
        if (task) state.pop = "explain";
        else state.refused = "no task selected to explain (I explains the space)";
        break;
      case "I": state.pop = "space"; break;
      case "?": state.pop = "keys"; break;
      case "p": if (task) state.asking = true; break;
      case "g": state.group[state.pane] = state.group[state.pane] === "none" ? "repo" : "none"; state.cursor[state.pane] = 0; break;
      case "s": state.sort[state.pane] = state.sort[state.pane] === "urgency" ? "waiting" : "urgency"; state.cursor[state.pane] = 0; break;
      case "a":
        if (task) {
          const tab = `${task.ticket ? task.ticket[0] + " " : ""}${task.ref}`;
          const place = `${task.repo.split("/").pop()} › ${tab} in ~/code/${task.repo.split("/").pop()}`;
          state.said = state.agents.has(task.id) ? `agent already there, told what to do: ${place}` : `agent opened: ${place}`;
          state.agents.set(task.id, "working");
        }
        break;
      case "Enter": if (task) state.said = `opened in your browser: ${short(task)}`; break;
      case "reset": state = fresh(); break;
      default: return;
    }
    draw();
  }

  term.addEventListener("keydown", (event) => {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const known = ["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight", "Enter", "Escape"].includes(event.key)
      || event.key.length === 1 || (event.key === "Tab" && !event.shiftKey && state.pane === 0);
    if (!known) return;
    event.preventDefault();
    press(event.key);
  });
  screen.addEventListener("click", (event) => {
    term.focus({ preventScroll: true });
    if (state.pop) return press("Escape");
    const space = event.target.closest("[data-space]");
    if (space) return press(String(Number(space.dataset.space) + 1));
    const row = event.target.closest("[data-pane]");
    if (!row) return;
    state.pane = Number(row.dataset.pane);
    if (row.dataset.task !== undefined) state.cursor[state.pane] = Number(row.dataset.task);
    state.said = state.refused = null;
    draw();
  });
  document.querySelectorAll(".keys button").forEach((button) => {
    button.addEventListener("click", () => {
      press(button.dataset.key);
      term.focus({ preventScroll: true });
    });
  });

  // The page around the board asks for a redraw when it shows it.
  window.aki = {
    draw,
    reset() {
      state = fresh();
      draw();
    },
  };

  addEventListener("resize", draw);
  (document.fonts ? document.fonts.ready : Promise.resolve()).then(draw);
  draw();
})();
