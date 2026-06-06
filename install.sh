#!/usr/bin/env bash
# kyma installer — the context engine for coding agents.
# https://github.com/shakedaskayo/kyma
#
# Quick start (interactive wizard if run in a terminal):
#   curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash
#
# Non-interactive examples:
#   ... | bash -s -- --yes                      # install the binary only
#   ... | bash -s -- --yes --serve --plugin     # install + serve + wire Claude Code
#   ... | bash -s -- --from-source --yes        # build from a git clone
#
# Flags:
#   --version VERSION   Release tag to install (default: latest)
#   --dir DIR           Install dir for the binary. Default: /usr/local/bin if
#                       already writable, else ~/.local/bin — NO sudo needed;
#                       sudo is only used if you explicitly pick a root-owned dir.
#   --from-source       Build from a git clone instead of a prebuilt binary
#   --src-dir DIR       Where to clone for --source builds (default: ~/kyma)
#   --serve             Start `kyma serve` (local web UI + API) after install
#   --no-serve          Don't start the server (skip the prompt)
#   --plugin            Install the Claude Code memory plugin (implies --serve)
#   --no-plugin         Don't install the plugin (skip the prompt)
#   --port PORT         Port for `kyma serve` (default: 7777)
#   --token TOKEN       Static API token to use (default: generated)
#   --prod-deploy       After install, launch `kyma deploy init` (AWS+Supabase
#                       production wizard) instead of the local-dev flow
#   --yes, -y           Assume defaults; no prompts
#   --help, -h          Show this help
#
# Env: KYMA_INSTALL_DIR, KYMA_SRC_DIR, KYMA_PORT, GITHUB_TOKEN (private/rate-limit),
#      KYMA_NO_MODIFY_PATH=1 (don't touch shell rc files)
#
# Note: we deliberately do NOT use `set -e` — the interactive wizard relies on
# `[ … ] && …` tests whose "false" result is normal control flow. Must-succeed
# steps are guarded explicitly with `|| die`.
set -uo pipefail

REPO="shakedaskayo/kyma"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/main"
INSTALL_DIR="${KYMA_INSTALL_DIR:-}"   # empty → resolved after flags (no-sudo default)
SRC_DIR="${KYMA_SRC_DIR:-$HOME/kyma}"
PORT="${KYMA_PORT:-7777}"
VERSION=""
TOKEN=""
FROM_SOURCE=0
ASSUME_YES=0
PROD_DEPLOY=0
DO_SERVE=""     # "", 1, or 0
DO_PLUGIN=""    # "", 1, or 0

# ── pretty output ────────────────────────────────────────────────────────────
bold=$'\033[1m'; dim=$'\033[2m'; grn=$'\033[32m'; ylw=$'\033[33m'; red=$'\033[31m'; rst=$'\033[0m'
say()  { printf '%s\n' "$*"; }
info() { printf '%s▸%s %s\n' "$grn" "$rst" "$*"; }
warn() { printf '%s!%s %s\n' "$ylw" "$rst" "$*" >&2; }
err()  { printf '%s✗ %s%s\n' "$red" "$*" "$rst" >&2; }
die()  { err "$*"; exit 1; }

usage() { sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }

# ── parse args ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version|-v) VERSION="$2"; shift 2 ;;
    --dir|-d)     INSTALL_DIR="$2"; shift 2 ;;
    --src-dir)    SRC_DIR="$2"; shift 2 ;;
    --port)       PORT="$2"; shift 2 ;;
    --token)      TOKEN="$2"; shift 2 ;;
    --from-source) FROM_SOURCE=1; shift ;;
    --serve)      DO_SERVE=1; shift ;;
    --no-serve)   DO_SERVE=0; shift ;;
    --plugin)     DO_PLUGIN=1; shift ;;
    --no-plugin)  DO_PLUGIN=0; shift ;;
    --prod-deploy) PROD_DEPLOY=1; shift ;;
    --yes|-y)     ASSUME_YES=1; shift ;;
    --help|-h)    usage ;;
    *) die "Unknown option: $1 (try --help)" ;;
  esac
done

# Interactive only when we have a real terminal and the user didn't pass --yes.
INTERACTIVE=0
[ "$ASSUME_YES" = "0" ] && [ -r /dev/tty ] && INTERACTIVE=1

ask() {  # ask "Question?" "Y|N"  -> exit 0 = yes
  local q="$1" def="${2:-Y}" ans
  if [ "$INTERACTIVE" = "0" ]; then [ "$def" = "Y" ]; return; fi
  if [ "$def" = "Y" ]; then printf '%s %s[Y/n]%s ' "$q" "$dim" "$rst" >/dev/tty
  else                       printf '%s %s[y/N]%s ' "$q" "$dim" "$rst" >/dev/tty; fi
  read -r ans </dev/tty || ans=""
  ans="${ans:-$def}"
  case "$ans" in [Yy]*) return 0 ;; *) return 1 ;; esac
}
prompt() {  # prompt "Question?" "default" -> echoes answer
  local q="$1" def="$2" ans
  if [ "$INTERACTIVE" = "0" ]; then printf '%s' "$def"; return; fi
  printf '%s %s[%s]%s ' "$q" "$dim" "$def" "$rst" >/dev/tty
  read -r ans </dev/tty || ans=""
  printf '%s' "${ans:-$def}"
}

have()     { command -v "$1" >/dev/null 2>&1; }
rand_hex() { openssl rand -hex 16 2>/dev/null || head -c16 /dev/urandom | od -An -tx1 | tr -d ' \n'; }

AUTH_HEADER=""
[ -n "${GITHUB_TOKEN:-}" ] && AUTH_HEADER="Authorization: token ${GITHUB_TOKEN}"
curl_gh() { if [ -n "$AUTH_HEADER" ]; then curl -H "$AUTH_HEADER" "$@"; else curl "$@"; fi; }

# ── install dir (no sudo by default) ────────────────────────────────────────
# kyma needs NO root: the binary runs as your user and all data lives in
# ~/.kyma. sudo only ever comes into play if you explicitly choose a
# root-owned dir (--dir /usr/local/bin or the interactive system-wide option).
resolve_install_dir() {
  [ -n "$INSTALL_DIR" ] && return   # --dir / KYMA_INSTALL_DIR: explicit choice
  # Already-writable /usr/local/bin (Intel-Mac Homebrew, root, custom): use it.
  if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
    INSTALL_DIR=/usr/local/bin
    return
  fi
  local user_dir="$HOME/.local/bin"
  if [ "$INTERACTIVE" = "1" ]; then
    if ask "Install to ${user_dir} (no sudo)? ${dim}('n' → /usr/local/bin via sudo)${rst}" "Y"; then
      INSTALL_DIR="$user_dir"
    else
      INSTALL_DIR=/usr/local/bin
    fi
  else
    INSTALL_DIR="$user_dir"
  fi
}

# Make INSTALL_DIR reachable in future shells (current shell is handled by
# ensure_on_path). Appends one guarded line to the shell rc; opt out with
# KYMA_NO_MODIFY_PATH=1.
persist_path() {
  case ":$PATH:" in *":$INSTALL_DIR:"*) return ;; esac
  local line rc
  line="export PATH=\"$INSTALL_DIR:\$PATH\""
  if [ -n "${KYMA_NO_MODIFY_PATH:-}" ]; then
    warn "$INSTALL_DIR is not on your PATH — add it yourself: $line"
    return
  fi
  case "${SHELL:-}" in
    */zsh)  rc="$HOME/.zshrc" ;;
    */bash) rc="$HOME/.bashrc" ;;
    */fish) warn "$INSTALL_DIR is not on your PATH — run: fish_add_path $INSTALL_DIR"; return ;;
    *)      rc="$HOME/.profile" ;;
  esac
  if [ "$INTERACTIVE" = "1" ]; then
    ask "Add ${INSTALL_DIR} to PATH in ${rc}?" "Y" || { say "  Add it yourself: $line"; return; }
  fi
  grep -qsF "$INSTALL_DIR" "$rc" 2>/dev/null || printf '\n# kyma\n%s\n' "$line" >> "$rc"
  info "Added $INSTALL_DIR to PATH in $rc — restart your shell (or: source $rc)"
}

# ── platform + version ──────────────────────────────────────────────────────
detect_platform() {
  local os arch
  case "$(uname -s)" in
    Darwin) os="darwin" ;;
    Linux)  os="linux" ;;
    *) die "Unsupported OS: $(uname -s) — use --from-source" ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64)  arch="x64" ;;
    aarch64|arm64) arch="arm64" ;;
    *) die "Unsupported arch: $(uname -m) — use --from-source" ;;
  esac
  printf 'kyma-%s-%s' "$os" "$arch"
}

resolve_version() {
  [ -n "$VERSION" ] && { printf '%s' "$VERSION"; return; }
  local v
  v=$(curl_gh -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
      | grep '"tag_name"' | head -1 | cut -d'"' -f4 || true)
  [ -z "$v" ] && v=$(curl_gh -fsSL "https://api.github.com/repos/${REPO}/releases" 2>/dev/null \
      | grep '"tag_name"' | head -1 | cut -d'"' -f4 || true)
  printf '%s' "$v"
}

# ── install: prebuilt binary ────────────────────────────────────────────────
install_binary() {
  local platform version artifact url tmp
  platform=$(detect_platform)
  version=$(resolve_version)
  [ -z "$version" ] && return 1   # no release yet → caller falls back to source
  artifact="${platform}.tar.gz"
  url="https://github.com/${REPO}/releases/download/${version}/${artifact}"

  info "Installing kyma ${bold}${version}${rst} (${platform}) → ${INSTALL_DIR}/kyma"
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' RETURN
  if ! curl_gh -fSL --progress-bar "$url" -o "$tmp/$artifact"; then
    warn "No prebuilt binary at $url"
    return 1
  fi
  if curl_gh -fsSL "${url}.sha256" -o "$tmp/sum" 2>/dev/null; then
    local want got
    want=$(awk '{print $1}' "$tmp/sum")
    if have sha256sum; then got=$(sha256sum "$tmp/$artifact" | awk '{print $1}')
    else got=$(shasum -a 256 "$tmp/$artifact" | awk '{print $1}'); fi
    [ "$want" = "$got" ] || die "Checksum mismatch (expected $want, got $got)"
    info "Checksum OK"
  fi
  tar xzf "$tmp/$artifact" -C "$tmp"
  "$tmp/kyma" --help >/dev/null 2>&1 || die "Downloaded binary won't run — wrong platform?"
  mkdir -p "$INSTALL_DIR" 2>/dev/null || true
  if [ -w "$INSTALL_DIR" ]; then mv "$tmp/kyma" "$INSTALL_DIR/kyma"
  else
    warn "Need sudo to write ${INSTALL_DIR} (a system dir — use --dir ~/.local/bin for a sudo-free install)"
    sudo mv "$tmp/kyma" "$INSTALL_DIR/kyma"
  fi
  chmod +x "$INSTALL_DIR/kyma"
  return 0
}

# ── install: build from source (clone → web build → cargo install) ──────────
install_source() {
  have cargo || die "Rust toolchain not found. Install from https://rustup.rs then re-run."
  have git   || die "git not found."
  have pnpm  || die "pnpm not found (needed to build the web UI). Install Node + pnpm, or use a prebuilt binary."

  local repo_root
  if [ -f "crates/kyma-cli/Cargo.toml" ]; then
    repo_root="$PWD"; info "Building from the current checkout: $repo_root"
  else
    if [ -d "$SRC_DIR/.git" ]; then
      info "Updating existing clone at $SRC_DIR"; git -C "$SRC_DIR" pull --ff-only || warn "git pull failed; building current state"
    else
      info "Cloning ${REPO} → $SRC_DIR"; git clone --depth 1 "https://github.com/${REPO}.git" "$SRC_DIR"
    fi
    repo_root="$SRC_DIR"
  fi

  info "Building the web UI (pnpm -C web build)…"
  ( cd "$repo_root" && pnpm install --frozen-lockfile && pnpm -C web build ) \
    || die "web UI build failed"
  info "Building + installing the CLI (cargo install) — this can take several minutes…"
  ( cd "$repo_root" && cargo install --path crates/kyma-cli --locked --force ) \
    || die "cargo install failed"
  # cargo installs to ~/.cargo/bin
  return 0
}

ensure_on_path() {
  have kyma && return 0
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) export PATH="$INSTALL_DIR:$PATH" ;;
  esac
  [ -d "$HOME/.cargo/bin" ] && export PATH="$HOME/.cargo/bin:$PATH"
  have kyma || die "kyma not found on PATH after install."
}

# ── end-to-end wiring ────────────────────────────────────────────────────────
server_version() {  # /health version of whatever is listening on :PORT
  curl -fsS "http://127.0.0.1:${PORT}/health" 2>/dev/null \
    | sed -n 's/.*"version":"\([^"]*\)".*/\1/p'
}

stop_stale_server() {  # the web UI is embedded in the binary → old process = old UI
  local pid="" i
  [ -f "$HOME/.kyma/serve.pid" ] && pid=$(cat "$HOME/.kyma/serve.pid" 2>/dev/null)
  if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null; then
    pid=""
    if have lsof; then
      pid=$(lsof -ti "tcp:${PORT}" 2>/dev/null | head -1)
      # Only kill it if it really is a kyma process.
      [ -n "$pid" ] && { ps -p "$pid" -o comm= 2>/dev/null | grep -q kyma || pid=""; }
    fi
  fi
  if [ -z "$pid" ]; then
    warn "Couldn't find the old server's pid — restart it yourself to load the new web UI."
    return 1
  fi
  kill "$pid" 2>/dev/null
  for i in $(seq 1 20); do
    kill -0 "$pid" 2>/dev/null || { rm -f "$HOME/.kyma/serve.pid"; return 0; }
    sleep 0.5
  done
  warn "Old server (pid $pid) didn't exit; restart it yourself to load the new web UI."
  return 1
}

start_serve() {
  [ -z "$TOKEN" ] && TOKEN="kyma-local-$(rand_hex)"
  mkdir -p "$HOME/.kyma"
  local log="$HOME/.kyma/serve.log"
  local need_start=1
  if curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
    # A server is up — but if it predates the binary we just installed, it is
    # still serving the OLD embedded web UI. Restart it on a version mismatch.
    local new_ver running_ver
    new_ver=$(kyma version 2>/dev/null | awk '{print $2}')
    running_ver=$(server_version)
    if [ -n "$new_ver" ] && [ "$running_ver" = "$new_ver" ]; then
      info "A server is already listening on :${PORT} (v${running_ver})"
      need_start=0
    else
      warn "Server on :${PORT} is running v${running_ver:-unknown}; you just installed v${new_ver} — restarting it."
      stop_stale_server || need_start=0   # couldn't stop it → don't double-bind the port
    fi
  fi
  if [ "$need_start" = "1" ]; then
    info "Starting kyma serve on http://127.0.0.1:${PORT} …"
    KYMA_AUTH_TOKENS="${TOKEN}:admin" nohup "$(command -v kyma)" serve \
      --addr "127.0.0.1:${PORT}" >"$log" 2>&1 &
    echo $! >"$HOME/.kyma/serve.pid"
    local ok=0 i
    for i in $(seq 1 60); do
      curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1 && { ok=1; break; }
      sleep 1
    done
    [ "$ok" = "1" ] || { warn "Server didn't become healthy; see $log"; return 1; }
    info "Server healthy (logs: $log)"
  fi
  info "Connecting the CLI (kyma connect)…"
  kyma connect "http://127.0.0.1:${PORT}" --token "$TOKEN" >/dev/null
}

run_smoke_test() {
  info "Smoke test: save + recall a memory (first run may download the embedding model)…"
  kyma remember "kyma install smoke-test — wired up via install.sh" \
    --topic-key install/smoke >/dev/null 2>&1 || { warn "remember failed (skipping)"; return 0; }
  if kyma recall "install smoke test" --limit 1 2>/dev/null | grep -qi "smoke-test"; then
    info "${grn}Memory round-trip OK${rst} — save → recall works end to end."
  else
    warn "recall didn't return the test memory yet (the model may still be downloading)."
  fi
}

# ── run ──────────────────────────────────────────────────────────────────────
say ""
say "${bold}kyma${rst} — the context engine for coding agents"
say "${dim}https://github.com/${REPO}${rst}"
say ""

resolve_install_dir
if [ "$FROM_SOURCE" = "1" ]; then
  install_source
else
  install_binary || { warn "Falling back to a source build."; install_source; }
fi
# persist first: ensure_on_path mutates this process's PATH, which would make
# the "already on PATH" check a false positive for future shells.
persist_path
ensure_on_path
# A leftover copy elsewhere (e.g. an old sudo install in /usr/local/bin) can
# shadow the fresh one depending on PATH order — surface it.
resolved="$(command -v kyma 2>/dev/null || true)"
if [ "$FROM_SOURCE" = "0" ] && [ -n "$resolved" ] && [ "$resolved" != "$INSTALL_DIR/kyma" ]; then
  warn "Another kyma is on your PATH at ${resolved} — the new install is ${INSTALL_DIR}/kyma."
fi
info "Installed: $(command -v kyma)  ($(kyma version 2>/dev/null || echo kyma))"
say ""

# ── production deployment hand-off ────────────────────────────────────────
# --prod-deploy: skip the local-dev flow entirely; the deploy wizard owns
# all prompts from here (AWS Fargate + S3 + Supabase, Terraform/Pulumi).
if [ "$PROD_DEPLOY" = "1" ]; then
  info "Launching the production deployment wizard…"
  exec kyma deploy init
fi

# Decide serve / plugin (flags > prompt > non-interactive default of NO).
if [ -z "$DO_PLUGIN" ]; then
  if ask "Install the Claude Code memory plugin (hooks + MCP + slash commands)?" "Y"; then DO_PLUGIN=1; else DO_PLUGIN=0; fi
fi
if [ -z "$DO_SERVE" ]; then
  if [ "$DO_PLUGIN" = "1" ]; then DO_SERVE=1   # plugin needs a server to talk to
  elif ask "Start the local server (web UI + API) on :${PORT} now?" "Y"; then DO_SERVE=1; else DO_SERVE=0; fi
fi
[ "$DO_PLUGIN" = "1" ] && [ "$DO_SERVE" = "0" ] && { warn "Plugin needs a server; enabling --serve."; DO_SERVE=1; }

if [ "$INTERACTIVE" = "1" ] && [ "$DO_SERVE" = "1" ]; then
  PORT="$(prompt "Port for the server?" "$PORT")"
fi

SERVED=0
if [ "$DO_SERVE" = "1" ]; then
  if start_serve; then SERVED=1; fi
fi
if [ "$DO_PLUGIN" = "1" ] && [ "$SERVED" = "1" ]; then
  info "Installing the kyma-memory plugin…"
  kyma install-plugin >/dev/null && info "Plugin installed → ~/.claude/skills/kyma-memory"
fi
[ "$SERVED" = "1" ] && run_smoke_test

# ── summary ───────────────────────────────────────────────────────────────
say ""
say "${grn}${bold}kyma is installed.${rst}"
if [ "$SERVED" = "1" ]; then
  say "  Web UI:   ${bold}http://127.0.0.1:${PORT}/${rst}   (sign in: admin / admin)"
  say "  Stop it:  kill \$(cat ~/.kyma/serve.pid)"
  say "  Restart:  KYMA_AUTH_TOKENS=\"${TOKEN}:admin\" kyma serve --addr 127.0.0.1:${PORT}"
  [ "$DO_PLUGIN" = "1" ] && say "  Plugin:   restart Claude Code, then run ${bold}/kyma-status${rst}"
else
  say "  Start it: ${bold}kyma serve${rst}   →  http://127.0.0.1:7777/  (admin / admin)"
  say "  Wire an agent: ${bold}kyma setup claude-code${rst}   (stdio MCP, zero infra)"
  say "  Or the plugin: ${bold}kyma connect <url> && kyma install-plugin${rst}"
fi
say "  Update:   ${bold}kyma update${rst}   (new binary + web UI, restarts the local server)"
say "  Docs:     https://www.getkyma.dev"
say ""
