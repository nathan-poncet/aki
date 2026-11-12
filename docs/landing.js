// The landing page: the pile is pulled into the button and aki appears;
// its three forms zoom in and out; a merge request shows how a verdict is
// reached; wires tie the integrations to the core.

(function () {
  const stage = document.querySelector(".stage");
  if (!stage) return;
  const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ── the pile goes in, aki comes out ──────────────────────────────
  const orbit = stage.querySelector(".orbit");
  const tryButton = stage.querySelector(".try");
  const reveal = stage.querySelector(".reveal");
  const forms = reveal.querySelector(".forms");
  const panels = [...forms.querySelectorAll(".form")];
  let form = "board";

  function layout() {
    const width = forms.clientWidth;
    const height = forms.clientHeight;
    const all = form === "all";
    // The overview spreads over the notes' room too: they are hidden then.
    const split = forms.parentElement.getBoundingClientRect();
    const own = forms.getBoundingClientRect();
    const start = split.left - own.left;
    const gap = 28;
    const scale = Math.min(0.6, (split.width - 2 * gap) / (3 * width));
    const captions = forms.querySelector(".captions");
    captions.style.left = `${start}px`;
    captions.style.right = `${own.right - split.right}px`;
    reveal.classList.toggle("overview", all);
    panels.forEach((panel, index) => {
      const mine = panel.dataset.form === form;
      panel.classList.toggle("mini", all);
      panel.classList.toggle("off", !all && !mine);
      panel.style.transform = all || !mine
        ? `translate(${start + index * (width * scale + gap)}px, ${(height - 470 * scale) / 2 - 44}px) scale(${scale})`
        : "none";
    });
    reveal.querySelectorAll(".forms-switch button").forEach((button) =>
      button.setAttribute("aria-pressed", String(button.dataset.form === form)));
    if (form === "board" && window.aki) window.aki.draw();
  }
  function choose(name) {
    form = name;
    layout();
  }
  reveal.querySelectorAll(".forms-switch button").forEach((button) =>
    button.addEventListener("click", () => choose(button.dataset.form)));
  panels.forEach((panel) => panel.addEventListener("click", () => {
    if (form === "all") choose(panel.dataset.form);
  }));
  addEventListener("resize", () => { if (!reveal.hidden) layout(); });

  function open() {
    stage.dataset.state = "aki";
    reveal.hidden = false;
    form = "board";
    if (window.aki) window.aki.reset();
    layout();
    reveal.querySelector(".term").focus({ preventScroll: true });
  }
  // The pile turns around the button, all the time: windows on the outer
  // ring, teammates on the inner one, the other way round.
  const sats = [...orbit.querySelectorAll(".sat")].map((element) => ({
    element,
    inner: element.dataset.ring === "in",
    angle: (Number(element.dataset.a) * Math.PI) / 180,
  }));
  let turning = true;
  let watched = 0;
  const place = (time) => {
    const wide = innerWidth;
    const small = wide < 900;
    const rx = small ? wide * (wide < 560 ? 0.3 : 0.34) : Math.min(wide * 0.4, 520);
    const ry = small ? 250 : 240;
    const size = wide < 560 ? 0.5 : small ? 0.62 : 1;
    const outer = (time / 80000) * Math.PI * 2;
    const inner = -(time / 52000) * Math.PI * 2;
    sats.forEach((sat) => {
      const angle = sat.angle + (sat.inner ? inner : outer);
      const x = Math.cos(angle) * (sat.inner ? Math.min(rx * 0.5, 250) : rx);
      const y = Math.sin(angle) * (sat.inner ? (small ? 150 : 138) : ry);
      const scale = sat.inner ? (small ? 0.8 : 1) : size * (0.88 + 0.12 * Math.sin(angle));
      sat.element.style.transform = `translate(${x}px, ${y}px) scale(${scale})`;
    });
    // The eyes follow one window, then the next.
    const target = sats[watched % sats.length].element.getBoundingClientRect();
    window.akiLook = { x: target.left + target.width / 2, y: target.top + target.height / 2 };
  };
  const tick = (time) => {
    if (turning) place(time);
    requestAnimationFrame(tick);
  };
  setInterval(() => { watched += 1; }, 2600);
  if (calm) place(0);
  else requestAnimationFrame(tick);

  tryButton.addEventListener("click", () => {
    if (calm || !orbit.animate) return open();
    tryButton.disabled = true;
    turning = false;
    window.akiLook = null;
    // Each window leaves from where it is, toward the button.
    const gone = [];
    sats.forEach((sat, index) => {
      gone.push(sat.element.animate(
        [{ transform: sat.element.style.transform, opacity: 1 }, { transform: "translate(0px, 0px) scale(0.04)", opacity: 0 }],
        { duration: 600, delay: index * 55, easing: "cubic-bezier(.6, 0, .9, .4)", fill: "forwards" },
      ));
    });
    const total = 600 + sats.length * 55;
    gone.push(tryButton.animate(
      [{ transform: "scale(1)" }, { transform: "scale(1.16)", offset: 0.75 }, { transform: "scale(0.2)", opacity: 0 }],
      { duration: total + 180, easing: "ease-in", fill: "forwards" },
    ));
    setTimeout(() => {
      open();
      gone.forEach((animation) => animation.cancel());
    }, total + 160);
  });
  reveal.querySelector(".again").addEventListener("click", () => {
    stage.dataset.state = "mess";
    reveal.hidden = true;
    tryButton.disabled = false;
    turning = true;
    if (calm) place(0);
    tryButton.focus({ preventScroll: true });
  });

  // ── what the team did, and what it changes ───────────────────────
  const feed = [...document.querySelectorAll(".feed li")];
  const outcomes = [...document.querySelectorAll(".outcome li")];
  if (feed.length) {
    let at = 0;
    let held = false;
    const light = (index) => {
      at = index;
      feed.forEach((row, position) => row.classList.toggle("on", position === index));
      const out = feed[index].dataset.out;
      outcomes.forEach((row) => row.classList.toggle("on", row.dataset.i === out));
      // The eyes of this section read the event at hand.
      const seer = document.querySelector(".seer .eyes");
      const row = feed[index].getBoundingClientRect();
      const box = seer.getBoundingClientRect();
      const dx = row.right - (box.left + box.width / 2);
      const dy = row.top + row.height / 2 - (box.top + box.height / 2);
      const far = Math.hypot(dx, dy) || 1;
      seer.dataset.held = "1";
      seer.style.setProperty("--ex", `${(dx / far) * 8}px`);
      seer.style.setProperty("--ey", `${(dy / far) * 8}px`);
    };
    feed.forEach((row, index) => {
      row.addEventListener("pointerenter", () => { held = true; light(index); });
      row.addEventListener("pointerleave", () => { held = false; });
    });
    light(0);
    if (!calm) setInterval(() => { if (!held) light((at + 1) % feed.length); }, 2200);
  }

  // ── how it decides ───────────────────────────────────────────────
  const note = (who, tone, name, when, text) =>
    `<div class="c"><span class="av ${tone}">${who}</span><div><b>${name}</b><small>${when}</small><p>${text}</p></div></div>`;
  const thread = (file, notes, resolve) =>
    `<div class="f">${file}</div>${notes.join("")}<div class="r"><span>Reply…</span>${resolve ? "<span>Resolve thread</span>" : ""}</div>`;
  const CASES = [
    {
      name: "A review is asked of you",
      ref: "!916", title: "Version the public API", by: "ben", tone: "a3",
      meta: "<b>ben</b> requested a review from <b>you</b> 3 weeks ago",
      pipe: ["ok", "Pipeline passed"], appr: [false, "Requires 1 approval from eligible users"],
      thread: "", push: "", lit: ["approval"],
      side: "todo", line: "▲ Review        api!916   Version the public API   @ben   155  3w",
      why: "Your review is requested and you have not started.",
      since: "The request, 3 weeks ago. Every day adds to its urgency.",
      act: "Review it.",
    },
    {
      name: "The author answered you",
      ref: "!889", title: "Typed errors for the key store", by: "ana", tone: "a2",
      meta: "<b>ana</b> requested a review from <b>you</b> · you reviewed 6 days ago",
      pipe: ["ok", "Pipeline passed"], appr: [false, "Requires 1 approval from eligible users"],
      thread: thread("src/store/error.rs", [
        note("N", "a1", "you", "6 days ago", "This swallows the cause. Can the store's error carry it?"),
        note("A", "a2", "ana", "5 days ago", "Done in 4c1f9e: the cause is kept and shown."),
      ], true),
      push: "ana added 2 commits 5 days ago", lit: ["thread", "push"],
      side: "todo", line: "▲ Re-review     api!889   Typed errors for the key store   @ana  +1  130  5d",
      why: "You reviewed, and since then the author replied and pushed. A thread you opened was answered.",
      since: "The first thing that moved after your review, 5 days ago.",
      act: "Look at what changed, then resolve your thread or follow up.",
    },
    {
      name: "You answered, they have not",
      ref: "!914", title: "Reuse connections to the bridge", by: "chloe", tone: "a4",
      meta: "<b>chloe</b> requested a review from <b>you</b> · you reviewed yesterday",
      pipe: ["ok", "Pipeline passed"], appr: [false, "Requires 1 approval from eligible users"],
      thread: thread("bridge/pool.rs", [
        note("N", "a1", "you", "1 day ago", "What bounds the pool when the bridge is slow?"),
      ], true),
      push: "", lit: ["thread"],
      side: "waiting", line: "  Awaiting author  web!914   Reuse connections to the bridge   @chloe   83  1d",
      why: "You reviewed and spoke last. Nothing new since.",
      since: "Your last word, 1 day ago.",
      act: "Nothing. It comes back to your side when chloe answers or pushes.",
    },
    {
      name: "Your pipeline failed",
      ref: "!80", title: "Print node health when the probe fails", by: "you", tone: "a1",
      meta: "<b>you</b> created this 9 days ago",
      pipe: ["ko", "Pipeline failed: 1 job failed (integration)"], appr: [false, "Requires 1 approval from eligible users"],
      thread: "", push: "you added 1 commit 9 days ago", lit: ["pipeline"],
      side: "todo", line: "  Fix checks    infra!80   Print node health when the probe fails   77  9d",
      why: "The pipeline of your change failed. Reviewers will not look until it is green.",
      since: "Your last push, 9 days ago.",
      act: "Fix the failing job.",
    },
    {
      name: "Approved and green",
      ref: "#12", title: "Offline search index", by: "you", tone: "a1",
      meta: "<b>you</b> created this 6 weeks ago",
      pipe: ["ok", "All checks have passed"], appr: [true, "Approved by sam"],
      thread: "", push: "", lit: ["pipeline", "approval"],
      side: "todo", line: "▴ Merge         notes#12   Offline search index   100  6w",
      why: "Your change is approved, green, with no open thread, and you can merge it.",
      since: "The approval, 6 weeks ago.",
      act: "Merge it.",
    },
    {
      name: "Waiting for reviewers",
      ref: "!976", title: "Error codes name the failure", by: "you", tone: "a1",
      meta: "<b>you</b> requested a review from <b>till</b> 3 hours ago",
      pipe: ["ok", "Pipeline passed"], appr: [false, "Requires 1 approval · till has not reviewed yet"],
      thread: "", push: "", lit: ["approval"],
      side: "waiting", line: "  Awaiting review  api!976   Error codes name the failure   90  3h",
      why: "One approval is missing, and till has not looked yet.",
      since: "Your last push, 3 hours ago.",
      act: "Nothing. If you set a nudge delay, it comes back as “Nudge” after that many idle days.",
    },
  ];
  const bench = document.querySelector(".bench");
  const cases = document.querySelector(".cases");
  if (bench && cases) {
    const $ = (selector) => bench.querySelector(selector);
    const show = (index) => {
      const item = CASES[index];
      $(".mr-ref").textContent = item.ref;
      $(".mr-title h3").textContent = item.title;
      $(".mr-av").className = `av ${item.tone} mr-av`;
      $(".mr-av").textContent = item.by[0].toUpperCase();
      $(".mr-by").innerHTML = item.meta;
      $(".mr-widget .gl-ci").className = `gl-ci ${item.pipe[0]}`;
      $(".mr-pipe").textContent = item.pipe[1];
      $(".mr-check").className = `mr-check${item.appr[0] ? " ok" : ""}`;
      $(".mr-appr").textContent = item.appr[1];
      $(".mr-thread").innerHTML = item.thread;
      $(".mr-push").textContent = item.push;
      bench.querySelectorAll("[data-fact]").forEach((fact) =>
        fact.classList.toggle("lit", item.lit.includes(fact.dataset.fact)));
      $(".v-side").className = `v-side ${item.side}`;
      $(".v-side").textContent = item.side === "todo" ? "TO DO · WAITING ON YOU" : "WAITING · ON OTHERS";
      $(".v-line").textContent = item.line;
      $(".v-why").textContent = item.why;
      $(".v-since").textContent = item.since;
      $(".v-do").textContent = item.act;
      [...cases.children].forEach((button, at) => button.setAttribute("aria-pressed", String(at === index)));
    };
    CASES.forEach((item, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = item.name;
      button.addEventListener("click", () => show(index));
      cases.append(button);
    });
    show(1);
  }

  // ── wires between the integrations and the core ──────────────────
  const flow = document.querySelector(".flow");
  if (flow) {
    const svg = flow.querySelector(".wires");
    const wire = () => {
      const box = flow.getBoundingClientRect();
      const core = flow.querySelector(".core").getBoundingClientRect();
      const paths = [];
      flow.querySelectorAll(".node").forEach((node) => {
        const rect = node.getBoundingClientRect();
        const incoming = node.dataset.wire === "in";
        const x1 = (incoming ? rect.right : rect.left) - box.left;
        const y1 = rect.top + rect.height / 2 - box.top;
        const x2 = (incoming ? core.left : core.right) - box.left;
        const y2 = core.top + core.height / 2 - box.top + (y1 - (core.top + core.height / 2 - box.top)) * 0.16;
        const bend = (x2 - x1) / 2;
        // Drawn in the direction the information goes.
        const d = incoming
          ? `M${x1},${y1} C${x1 + bend},${y1} ${x2 - bend},${y2} ${x2},${y2}`
          : `M${x2},${y2} C${x2 - bend},${y2} ${x1 + bend},${y1} ${x1},${y1}`;
        paths.push(`<path d="${d}"/><path class="live" d="${d}"/>`);
      });
      svg.innerHTML = paths.join("");
    };
    addEventListener("resize", wire);
    (document.fonts ? document.fonts.ready : Promise.resolve()).then(wire);
    wire();
  }
})();
