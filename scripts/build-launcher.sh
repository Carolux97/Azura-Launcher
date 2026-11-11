#!/usr/bin/env bash
# Builds the Azura Launcher app for local use: target/release/bundle/macos/Azura Launcher.app.
#
# Only the .app is built, so the Mac knows a single "Azura Launcher". The .dmg players download is
# made by scripts/release-launcher.sh into dist/. Leftovers that would show up as a second
# launcher (an old .dmg in target/, LaunchServices entries for disk images Tauri mounted while
# building one) are removed.
#
# With --dev it builds the development launcher instead, "Azura (DEV).app": tauri.dev.conf.json
# gives it its own name, bundle identifier (so its data lives apart), icon with a DEV band and
# window title, and the `dev` feature gives it its own default saves folder and turns its updater
# off. It installs next to the published launcher without touching it.
#
# Usage: scripts/build-launcher.sh [--dev]

set -euo pipefail

# Ensure cargo and rustup toolchains are in PATH
if ! command -v cargo &>/dev/null; then
  if [[ -d "${HOME}/.rustup/toolchains/stable-aarch64-apple-darwin/bin" ]]; then
    export PATH="${HOME}/.rustup/toolchains/stable-aarch64-apple-darwin/bin:${PATH}"
  elif [[ -d "${HOME}/.cargo/bin" ]]; then
    export PATH="${HOME}/.cargo/bin:${PATH}"
  fi
fi

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly LAUNCHER="${ROOT}"
readonly BUNDLE="${LAUNCHER}/src-tauri/target/release/bundle"
readonly DMG_DIR="${BUNDLE}/dmg"

APP_NAME="Azura Launcher"
TAURI_ARGS=(--bundles app)
if [[ "${1:-}" == "--dev" ]]; then
  APP_NAME="Azura (DEV)"
  TAURI_ARGS+=(--config src-tauri/tauri.dev.conf.json --features dev)
elif [[ $# -gt 0 ]]; then
  echo "Usage: scripts/build-launcher.sh [--dev]" >&2
  exit 2
fi
readonly APP_NAME TAURI_ARGS

echo "==> Cleaning old launcher bundles in target"
rm -rf "${LAUNCHER}/src-tauri/target/debug/bundle"
rm -rf "${DMG_DIR}"
rm -f "${BUNDLE}/macos/"*.tar.gz* "${BUNDLE}/macos/"rw.*.dmg 2>/dev/null || true

echo "==> Building ${APP_NAME}"
(cd "${LAUNCHER}" && npx tauri build "${TAURI_ARGS[@]}")

# Forget LaunchServices entries for apps inside disk images that are no longer mounted (Tauri
# mounts one while building a .dmg; the entry outlives the mount).
if [[ -x "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister" ]]; then
  readonly LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
  "${LSREGISTER}" -dump 2>/dev/null \
    | awk -v app="${APP_NAME}.app" 'index($0, "/Volumes/") && index($0, app) && /path:/ {sub(/^[[:space:]]*path:[[:space:]]*/, ""); sub(/[[:space:]]*\(0x[0-9a-f]+\)$/, ""); print}' \
    | while IFS= read -r p; do
        "${LSREGISTER}" -u "${p}" 2>/dev/null || true
      done
  "${LSREGISTER}" -gc 2>/dev/null || true
fi

echo "==> Done: ${BUNDLE}/macos/${APP_NAME}.app"
