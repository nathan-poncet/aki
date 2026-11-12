#!/usr/bin/env python3
"""Builds the site into ../docs, which GitHub Pages serves.

    python3 site/build.py

Pages are HTML fragments in site/pages, each starting with a comment that
gives its title and one-line description:

    <!-- title: Install | lead: One command, then sign in. -->

The landing page is site/landing.html. Everything else (the frame around a
page, the sidebar, "On this page", previous and next, the search index) is
made here. Standard library only.
"""

import html
import json
import re
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "docs"
REPO = "https://github.com/nathan-poncet/aki"
SITE = "https://nathan-poncet.github.io/aki"

# The documentation, in reading order.
NAV = [
    ("Start", ["install", "quick-start"]),
    ("The board", ["rules", "urgency", "spaces", "filters", "keys"]),
    ("Working from it", ["agents", "assistants", "set-aside"]),
    ("Reference", ["configuration", "sources", "cli", "faq"]),
]

EYES = '<span class="eyes"><i><b></b></i><i><b></b></i></span>'
MARK = f'<span class="badge">{EYES}</span>'
THEME = (
    '<button class="theme" type="button" aria-label="Switch between dark and light">'
    '<svg class="moon" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">'
    '<path d="M6.2 1.5a6.5 6.5 0 1 0 8.3 8.3A5.4 5.4 0 0 1 6.2 1.5z"/></svg>'
    '<svg class="sun" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" '
    'stroke-linecap="round" aria-hidden="true"><circle cx="8" cy="8" r="3"/>'
    '<path d="M8 1v1.6M8 13.4V15M1 8h1.6M13.4 8H15M3 3l1.2 1.2M11.8 11.8 13 13M3 13l1.2-1.2M11.8 4.2 13 3"/>'
    "</svg></button>"
)


def head(title, description, page):
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<meta name="description" content="{html.escape(description)}">
<meta property="og:title" content="{html.escape(title)}">
<meta property="og:description" content="{html.escape(description)}">
<meta property="og:type" content="website">
<meta property="og:url" content="{SITE}/{page}">
<meta property="og:image" content="{SITE}/media/share-card.png">
<meta name="twitter:card" content="summary_large_image">
<link rel="icon" href="media/favicon.svg" type="image/svg+xml">
<link rel="icon" href="media/favicon-32.png" sizes="32x32" type="image/png">
<link rel="apple-touch-icon" href="media/apple-touch-icon.png">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans:wght@400;500;600&display=swap" rel="stylesheet">
<link rel="stylesheet" href="styles.css">
<script>try{{document.documentElement.dataset.theme=localStorage.getItem("aki-theme")||(matchMedia("(prefers-color-scheme: light)").matches?"light":"dark")}}catch(e){{}}</script>
</head>
"""


def header(current, wide):
    def link(href, text, extra=""):
        here = ' aria-current="page"' if current == href else ""
        return f'<a href="{href}"{here}{extra}>{text}</a>'

    inner = (
        f'<a class="brand" href="index.html" aria-label="aki, home">{MARK}aki</a>'
        '<nav class="nav" aria-label="Site">'
        + link("install.html", "Docs")
        + link("quick-start.html", "Quick start", ' class="hide-s"')
        + link("faq.html", "FAQ", ' class="hide-s"')
        + f'<a class="gh" href="{REPO}">GitHub</a>'
        + THEME
        + ('<button class="menu" type="button" aria-label="Documentation menu">≡</button>' if wide else "")
        + "</nav>"
    )
    box = "bar" if wide else "wrap"
    return f'<header class="top"><div class="{box}">{inner}</div></header>\n'


FOOT = f"""<footer class="foot"><div class="wrap">
<span>aki — à qui ?</span>
<a href="install.html">Docs</a>
<a href="{REPO}">GitHub</a>
<a href="{REPO}/issues">Issues</a>
<span class="push">GPL-3.0-or-later</span>
</div></footer>
"""


def read(path):
    """A fragment's settings (from its first comment) and its body."""
    text = path.read_text()
    found = re.match(r"\s*<!--(.*?)-->", text, re.S)
    settings = {}
    if found:
        for pair in found.group(1).split("|"):
            key, _, value = pair.partition(":")
            settings[key.strip()] = value.strip()
        text = text[found.end():]
    return settings, text.strip()


def slugify(text):
    return re.sub(r"[^a-z0-9]+", "-", re.sub(r"<[^>]+>", "", text).lower()).strip("-")


def plain(fragment):
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", " ", fragment))).strip()


def anchor(body):
    """Gives every h2 and h3 an id; returns the body and the headings."""
    headings = []
    seen = set()

    def give(match):
        level, inner = match.group(1), match.group(2)
        slug = slugify(inner)
        while slug in seen:
            slug += "-"
        seen.add(slug)
        headings.append((int(level), slug, re.sub(r"<[^>]+>", "", inner)))
        return f'<h{level} id="{slug}">{inner}</h{level}>'

    return re.sub(r"<h([23])>(.*?)</h\1>", give, body, flags=re.S), headings


def code(body):
    """Wraps every <pre> in the frame its copy button hangs on."""

    def wrap(match):
        caption = match.group(1)
        cap = f'<div class="cap">{caption}</div>' if caption else ""
        return f'<div class="code">{cap}<pre>{match.group(2)}</pre></div>'

    return re.sub(r'<pre(?: data-cap="([^"]*)")?>(.*?)</pre>', wrap, body, flags=re.S)


def sections(slug, title, lead, body, headings):
    """The page cut at its headings, for the search index."""
    entries = []
    pieces = re.split(r'<h[23] id="([^"]+)">.*?</h[23]>', body, flags=re.S)
    names = {slug_: name for _, slug_, name in headings}
    entries.append({"u": f"{slug}.html", "t": title, "h": "", "x": plain(lead + " " + pieces[0])})
    for index in range(1, len(pieces), 2):
        entries.append({
            "u": f"{slug}.html#{pieces[index]}",
            "t": title,
            "h": names[pieces[index]],
            "x": plain(pieces[index + 1]),
        })
    return entries


def build():
    if OUT.exists():
        shutil.rmtree(OUT)
    (OUT / "media").mkdir(parents=True)
    for asset in (HERE / "assets").iterdir():
        shutil.copy(asset, OUT / asset.name)
    for media in (HERE / "media").iterdir():
        shutil.copy(media, OUT / "media" / media.name)
    shutil.copy(HERE / "install.sh", OUT / "install.sh")
    # GitHub Pages must serve the files as they are.
    (OUT / ".nojekyll").write_text("")

    order = [slug for _, slugs in NAV for slug in slugs]
    pages = {slug: read(HERE / "pages" / f"{slug}.html") for slug in order}
    index = []

    for position, slug in enumerate(order):
        settings, body = pages[slug]
        title, lead = settings["title"], settings.get("lead", "")
        body, headings = anchor(code(body))
        index.extend(sections(slug, title, lead, body, headings))
        group = next(name for name, slugs in NAV if slug in slugs)

        side = ['<div class="search"><input type="search" placeholder="Search the docs" '
                'aria-label="Search the documentation" autocomplete="off"><kbd>/</kbd>'
                '<div class="results" role="listbox"></div></div>']
        for name, slugs in NAV:
            side.append(f"<h4>{name}</h4>")
            for other in slugs:
                here = ' aria-current="page"' if other == slug else ""
                side.append(f'<a href="{other}.html"{here}>{pages[other][0]["title"]}</a>')

        toc = "".join(
            f'<a href="#{anchor_}"{" class=\"sub\"" if level == 3 else ""}>{html.escape(name)}</a>'
            for level, anchor_, name in headings
        )
        toc = f'<aside class="toc" aria-label="On this page"><h4>On this page</h4>{toc}</aside>' if headings else "<aside class=\"toc\"></aside>"

        around = []
        if position:
            before = order[position - 1]
            around.append(f'<a href="{before}.html"><small>Previous</small>{pages[before][0]["title"]}</a>')
        if position + 1 < len(order):
            after = order[position + 1]
            around.append(f'<a class="next" href="{after}.html"><small>Next</small>{pages[after][0]["title"]}</a>')

        page = (
            head(f"{title} · aki", lead or f"{title}: aki documentation.", f"{slug}.html")
            + "<body>\n"
            + header("install.html", True)
            + '<div class="docs">\n<nav class="side" aria-label="Documentation">'
            + "".join(side)
            + '</nav>\n<main class="doc"><article>'
            + f'<div class="crumb">{group}</div><h1>{title}</h1>'
            + (f'<p class="lead">{lead}</p>' if lead else "")
            + body
            + f'<div class="pn">{"".join(around)}</div>'
            + "</article></main>\n"
            + toc
            + '</div>\n<script src="search-index.js"></script><script src="site.js"></script>\n</body></html>\n'
        )
        (OUT / f"{slug}.html").write_text(page)

    (OUT / "search-index.js").write_text("window.AKI_SEARCH=" + json.dumps(index, ensure_ascii=False) + ";\n")

    settings, body = read(HERE / "landing.html")
    (OUT / "index.html").write_text(
        head(settings["title"], settings["lead"], "").replace(
            '<link rel="stylesheet" href="styles.css">',
            '<link rel="stylesheet" href="styles.css">\n<link rel="stylesheet" href="landing.css">',
        )
        + '<body>\n'
        + header("index.html", False)
        + f'<main class="landing">{body}</main>\n'
        + FOOT
        + '<script src="site.js"></script><script src="demo.js"></script><script src="landing.js"></script>\n</body></html>\n'
    )

    (OUT / "404.html").write_text(
        head("Not found · aki", "This page does not exist.", "404.html").replace(
            'href="styles.css"', f'href="{SITE}/styles.css"'
        ).replace('href="media/', f'href="{SITE}/media/')
        + "<body>\n"
        + header("", False).replace('href="', f'href="{SITE}/').replace(f'href="{SITE}/{REPO}', f'href="{REPO}')
        + '<main class="lost wrap"><h1>404</h1><p>Nothing here.</p>'
        + f'<p><a href="{SITE}/">Home</a> · <a href="{SITE}/install.html">Documentation</a></p></main>\n'
        + f'<script src="{SITE}/site.js"></script>\n</body></html>\n'
    )
    print(f"built {len(order) + 2} pages into {OUT}")


if __name__ == "__main__":
    build()
