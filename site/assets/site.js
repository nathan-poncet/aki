// aki — what every page needs: theme, copy buttons, tabs, search, the
// table of contents following the scroll.

(function () {
  const root = document.documentElement;

  // Theme: the visitor's choice, else the system's.
  const stored = (() => {
    try { return localStorage.getItem("aki-theme"); } catch { return null; }
  })();
  const system = matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  root.dataset.theme = stored || system;
  document.addEventListener("click", (event) => {
    if (!event.target.closest(".theme")) return;
    root.dataset.theme = root.dataset.theme === "light" ? "dark" : "light";
    try { localStorage.setItem("aki-theme", root.dataset.theme); } catch {}
  });

  // The eyes look at the pointer, or at whatever a page tells them to
  // (window.akiLook = {x, y} in viewport coordinates).
  const look = (x, y) => {
    document.querySelectorAll(".eyes").forEach((eyes) => {
      const box = eyes.getBoundingClientRect();
      if (!box.width || eyes.dataset.held) return;
      const dx = x - (box.left + box.width / 2);
      const dy = y - (box.top + box.height / 2);
      const far = Math.hypot(dx, dy) || 1;
      const reach = parseFloat(getComputedStyle(eyes).fontSize) * 0.13 * Math.min(far / 60, 1);
      eyes.style.setProperty("--ex", `${(dx / far) * reach}px`);
      eyes.style.setProperty("--ey", `${(dy / far) * reach}px`);
    });
  };
  let moved = 0;
  addEventListener("pointermove", (event) => {
    moved = Date.now();
    look(event.clientX, event.clientY);
  }, { passive: true });
  setInterval(() => {
    // Left alone, they go back to what the page shows them.
    if (Date.now() - moved > 1800 && window.akiLook) look(window.akiLook.x, window.akiLook.y);
  }, 120);

  // A copy button on every block of code.
  const copyOf = (button, text) => {
    const done = () => {
      button.textContent = "copied";
      button.classList.add("done");
      setTimeout(() => {
        button.textContent = "copy";
        button.classList.remove("done");
      }, 1400);
    };
    (navigator.clipboard ? navigator.clipboard.writeText(text) : Promise.reject())
      .then(done)
      .catch(() => {
        const area = document.createElement("textarea");
        area.value = text;
        document.body.append(area);
        area.select();
        document.execCommand("copy");
        area.remove();
        done();
      });
  };
  document.querySelectorAll(".code").forEach((block) => {
    const button = document.createElement("button");
    button.className = "copy";
    button.type = "button";
    button.textContent = "copy";
    button.setAttribute("aria-label", "Copy this block");
    button.addEventListener("click", () => copyOf(button, block.querySelector("pre").innerText));
    block.append(button);
  });
  document.querySelectorAll(".cmd .copy").forEach((button) => {
    button.addEventListener("click", () => copyOf(button, button.dataset.copy));
  });

  // Tabs: several ways to do one thing, one shown at a time.
  document.querySelectorAll(".tabs").forEach((tabs) => {
    const sections = [...tabs.querySelectorAll(":scope > section")];
    const list = document.createElement("div");
    list.className = "tablist";
    list.setAttribute("role", "tablist");
    const show = (index) => {
      sections.forEach((section, at) => (section.hidden = at !== index));
      [...list.children].forEach((button, at) =>
        button.setAttribute("aria-selected", String(at === index)));
    };
    sections.forEach((section, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.setAttribute("role", "tab");
      button.textContent = section.dataset.tab;
      button.addEventListener("click", () => show(index));
      list.append(button);
    });
    tabs.prepend(list);
    show(0);
  });

  // The documentation's menu on small screens.
  const side = document.querySelector(".side");
  document.querySelector(".menu")?.addEventListener("click", () => side?.classList.toggle("open"));

  // Search over every section of every page.
  const input = document.querySelector(".search input");
  const results = document.querySelector(".results");
  if (input && results && window.AKI_SEARCH) {
    let on = -1;
    const words = (text) => text.toLowerCase().split(/\s+/).filter(Boolean);
    const render = () => {
      const asked = words(input.value);
      on = -1;
      if (!asked.length) return results.classList.remove("open");
      const found = window.AKI_SEARCH
        .map((entry) => {
          const title = (entry.t + " " + entry.h).toLowerCase();
          const body = entry.x.toLowerCase();
          let score = 0;
          for (const word of asked) {
            if (title.includes(word)) score += 5;
            else if (body.includes(word)) score += 1;
            else return null;
          }
          return { entry, score };
        })
        .filter(Boolean)
        .sort((a, b) => b.score - a.score)
        .slice(0, 8);
      results.innerHTML = found.length
        ? found.map(({ entry }) => {
            const at = entry.x.toLowerCase().indexOf(asked[0]);
            const excerpt = entry.x.slice(Math.max(0, at - 30), at + 90);
            return `<a href="${entry.u}"><b>${entry.t}${entry.h ? " › " + entry.h : ""}</b><span>${excerpt}</span></a>`;
          }).join("")
        : `<div class="none">Nothing for “${input.value.replace(/[<&]/g, "")}”.</div>`;
      results.classList.add("open");
    };
    input.addEventListener("input", render);
    input.addEventListener("focus", render);
    input.addEventListener("keydown", (event) => {
      const links = [...results.querySelectorAll("a")];
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        on = (on + (event.key === "ArrowDown" ? 1 : -1) + links.length) % links.length;
        links.forEach((link, at) => link.classList.toggle("on", at === on));
      } else if (event.key === "Enter" && links.length) {
        location.href = links[Math.max(on, 0)].href;
      } else if (event.key === "Escape") {
        input.blur();
        results.classList.remove("open");
      }
    });
    document.addEventListener("click", (event) => {
      if (!event.target.closest(".search")) results.classList.remove("open");
    });
    document.addEventListener("keydown", (event) => {
      const typing = /^(INPUT|TEXTAREA)$/.test(document.activeElement.tagName);
      if (event.key === "/" && !typing && !document.activeElement.closest(".term")) {
        event.preventDefault();
        input.focus();
      }
    });
  }

  // "On this page" follows the scroll.
  const marks = [...document.querySelectorAll(".toc a")];
  if (marks.length) {
    const heads = marks.map((mark) => document.getElementById(mark.hash.slice(1)));
    const follow = () => {
      let here = 0;
      heads.forEach((head, at) => {
        if (head && head.getBoundingClientRect().top < 120) here = at;
      });
      marks.forEach((mark, at) => mark.classList.toggle("here", at === here));
    };
    addEventListener("scroll", follow, { passive: true });
    follow();
  }
})();
