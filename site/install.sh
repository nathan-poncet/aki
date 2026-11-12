#!/bin/sh
# Installs aki: builds it with Cargo from its repository, then says what to
# sign in to. Nothing is installed besides the `aki` program itself.
#
#   curl -fsSL https://nathan-poncet.github.io/aki/install.sh | sh
#
# AKI_REF=<tag or branch>   install that version instead of the latest
# AKI_RUSTUP=1              install the Rust toolchain first when it is missing

set -eu

REPO="https://github.com/nathan-poncet/aki"

say() { printf '%s\n' "$*"; }
fail() { printf 'aki: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin | Linux) ;;
  *) fail "this script supports macOS and Linux; on another system, build from source: $REPO" ;;
esac

if ! command -v cargo >/dev/null 2>&1; then
  if [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
  elif [ "${AKI_RUSTUP:-0}" = "1" ]; then
    say "Installing the Rust toolchain with rustup…"
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    PATH="$HOME/.cargo/bin:$PATH"
  else
    fail "Cargo is needed to build aki. Install Rust from https://rustup.rs, or run this again with AKI_RUSTUP=1 to have it installed for you."
  fi
fi
command -v git >/dev/null 2>&1 || fail "git is needed to fetch the sources"

say "Building aki (a minute or two the first time)…"
if [ -n "${AKI_REF:-}" ]; then
  cargo install --locked --git "$REPO" --rev "$AKI_REF" aki 2>/dev/null \
    || cargo install --locked --git "$REPO" --branch "$AKI_REF" aki
else
  cargo install --locked --git "$REPO" aki
fi

BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
say ""
say "aki is installed in $BIN."
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) say "Add it to your PATH:  export PATH=\"$BIN:\$PATH\"" ;;
esac

say ""
say "aki reads GitLab and GitHub with the logins of their own CLIs:"
if command -v glab >/dev/null 2>&1; then
  say "  glab  found      (sign in with: glab auth login)"
else
  say "  glab  not found  (https://gitlab.com/gitlab-org/cli)"
fi
if command -v gh >/dev/null 2>&1; then
  say "  gh    found      (sign in with: gh auth login)"
else
  say "  gh    not found  (https://cli.github.com)"
fi
say ""
say "Then:"
say "  aki doctor   checks every source"
say "  aki          opens the board"
say ""
say "Jira and everything else: https://nathan-poncet.github.io/aki/quick-start.html"
