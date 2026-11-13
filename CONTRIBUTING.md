# Contributing

Issues and pull requests are welcome. Read [ARCHITECTURE.md](ARCHITECTURE.md)
first: it says where each kind of change belongs.

## Build and test

```sh
git clone https://github.com/nathan-poncet/aki
cd aki
cargo build
cargo test
cargo fmt --check && cargo clippy --all-targets
```

`cargo install --path .` installs your build as `aki`.

Tests use invented data only: no real project, ticket or person's name.

## Adding a source

A source (Linear, Bitbucket, Gitea…) is one gateway in
`src/adapters/gateways` implementing `WorkSource`: map what the service says
to a `ChangeRequest` or a `Ticket`, and every rule applies to it unchanged.
Then give it a `kind` in `src/adapters/config.rs` and connect it in
`src/app.rs`.

## Adding a multiplexer or a terminal

A launcher is one adapter implementing `Launcher`. `multiplexers.rs` holds
tmux and TUIOS, `launch.rs` holds Herdr, the terminals and the Claude
conversation. Try a new one in a session of its own, never in the one you
work in: a launcher types into terminals.

## The site

The site (landing page and documentation) is served by GitHub Pages from
`docs/`, at https://nathan-poncet.github.io/aki/. It is generated from
`site/`: pages are HTML fragments in `site/pages`, and

```sh
python3 site/build.py
```

rebuilds `docs/` (standard library only). Edit `site/`, never `docs/`.
