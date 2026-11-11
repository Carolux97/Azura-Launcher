#!/usr/bin/env bash
# Builds a signed release of the Azura launcher for the platform it runs on (macOS on Apple
# Silicon, Linux on x86-64, Windows on x86-64 from Git Bash), with the player kit inside, and, with
# --publish, publishes it (scripts/publish-launcher.sh) so installed launchers update themselves.
# The Release workflow runs it on every platform and publishes once with all of them; running it by
# hand releases one platform.
#
# Steps:
#   1. read the version from tauri.conf.json and check package.json and Cargo.toml agree; the
#      script never changes the version, bump it in those three files first;
#   2. check the player kit of the platform is there (scripts/fetch-kit.sh downloads the one
#      kit.toml pins): it goes into the bundle's resources as kit/, where the launcher's builder
#      looks for it;
#   3. build the bundle and its signed update archive: on macOS the .app (its update archive is
#      the .app.tar.gz) and the .dmg players download; on Linux the AppImage, which is both; on
#      Windows the NSIS installer, which is both too (the updater runs it);
#   4. copy them to dist/<version>/ with space-free names and write the platform's entry
#      of latest.json (latest-<platform>.json: signature and archive URL), which
#      publish-launcher.sh merges into the latest.json the launcher's updater reads.
#
# The update signing key never lives in the repository. Pass it through the environment:
#   TAURI_SIGNING_PRIVATE_KEY           path to the private key (or its contents)
#   TAURI_SIGNING_PRIVATE_KEY_PASSWORD  its password
# Optional:
#   AZURA_RELEASE_KIT       the kit folder to bundle (default: kit/<platform>, from scripts/fetch-kit.sh)
#   AZURA_UPDATE_BASE_URL   base URL of the archives in latest.json, for testing with a local
#                           server (default: the release's download URL, see release-common.sh)
#
# Usage: scripts/release-launcher.sh [--publish]

set -euo pipefail

# Ensure cargo and rustup toolchains are in PATH
if ! command -v cargo &>/dev/null; then
  if [[ -d "${HOME}/.rustup/toolchains/stable-aarch64-apple-darwin/bin" ]]; then
    export PATH="${HOME}/.rustup/toolchains/stable-aarch64-apple-darwin/bin:${PATH}"
  elif [[ -d "${HOME}/.cargo/bin" ]]; then
    export PATH="${HOME}/.cargo/bin:${PATH}"
  fi
fi

# On Windows (Git Bash) as a Windows path with forward slashes (pwd -W), which the native
# programs it is passed to (CMake, Python, Node) read; elsewhere pwd -W fails and pwd answers.
readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && { pwd -W 2>/dev/null || pwd; })"
readonly LAUNCHER="${ROOT}"
source "${ROOT}/scripts/release-common.sh"

PUBLISH=false
case "${1:-}" in
  "") ;;
  --publish) PUBLISH=true ;;
  *)
    echo "Unknown option '$1'. Usage: scripts/release-launcher.sh [--publish]" >&2
    exit 1
    ;;
esac

# The platform as the kit names it (scripts/sdk_prepare.sh) and as the updater names it in
# latest.json ({os}-{arch}), and the bundles to build.
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    readonly KIT_PLATFORM="mac-arm64" UPDATER_PLATFORM="darwin-aarch64" BUNDLES="app,dmg"
    ;;
  Linux-x86_64)
    readonly KIT_PLATFORM="linux-amd64" UPDATER_PLATFORM="linux-x86_64" BUNDLES="appimage"
    ;;
  MINGW64_NT*-x86_64 | MSYS_NT*-x86_64)
    readonly KIT_PLATFORM="win-amd64" UPDATER_PLATFORM="windows-x86_64" BUNDLES="nsis"
    ;;
  *)
    echo "Releases are built on macOS (Apple Silicon), Linux or Windows (x86-64), not $(uname -s) $(uname -m)." >&2
    exit 1
    ;;
esac

# Assigned before readonly so a failed check stops the script (set -e).
VERSION="$(launcher_version)"
readonly VERSION
readonly TAG="$(release_tag "${VERSION}")"
# The version's notes must exist before anything is built (CHANGELOG.md).
release_notes "${VERSION}" >/dev/null

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "TAURI_SIGNING_PRIVATE_KEY is not set. See the comment at the top of this script." >&2
  exit 1
fi

readonly KIT="${AZURA_RELEASE_KIT:-${ROOT}/kit/${KIT_PLATFORM}}"
if [[ ! -f "${KIT}/kit.json" ]]; then
  echo "No player kit in ${KIT}. Download it with scripts/fetch-kit.sh." >&2
  exit 1
fi
readonly KIT_FILE_PLATFORM="$(json_field "${KIT}/kit.json" platform)"
if [[ "${KIT_FILE_PLATFORM}" != "${KIT_PLATFORM}" ]]; then
  echo "The kit in ${KIT} is for ${KIT_FILE_PLATFORM}, not ${KIT_PLATFORM}." >&2
  exit 1
fi

echo "==> Cleaning old bundles in target"
rm -rf "${LAUNCHER}/src-tauri/target/debug/bundle"
rm -rf "${LAUNCHER}/src-tauri/target/release/bundle"

# The kit and the update archives only go into release builds: this configuration is merged into
# tauri.conf.json for this build, so development builds and tests need neither.
readonly RELEASE_CONFIG="$(node -e '
  const [kit] = process.argv.slice(1);
  process.stdout.write(JSON.stringify({
    bundle: { createUpdaterArtifacts: true, resources: { [kit + "/"]: "kit/" } },
  }));
' "${KIT}")"

echo "==> Building Azura Launcher ${VERSION} for ${KIT_PLATFORM} with kit $(json_field "${KIT}/kit.json" revision)"
(cd "${LAUNCHER}" && npx tauri build --bundles "${BUNDLES}" --config "${RELEASE_CONFIG}")

readonly BUNDLE="${LAUNCHER}/src-tauri/target/release/bundle"
readonly DIST="${LAUNCHER}/dist/${VERSION}"
mkdir -p "${DIST}"
rm -f "${DIST}/latest-${UPDATER_PLATFORM}.json"

if [[ "${KIT_PLATFORM}" == mac-* ]]; then
  readonly ARCHIVE="Azura-Launcher_${VERSION}_aarch64.app.tar.gz"
  readonly DOWNLOAD="Azura-Launcher_${VERSION}_aarch64.dmg"
  cp "${BUNDLE}/macos/Azura Launcher.app.tar.gz" "${DIST}/${ARCHIVE}"
  cp "${BUNDLE}/macos/Azura Launcher.app.tar.gz.sig" "${DIST}/${ARCHIVE}.sig"
  cp "$(find "${BUNDLE}/dmg" -name "*.dmg" -print -quit)" "${DIST}/${DOWNLOAD}"
  # The published copy lives in dist/; leaving the DMG in target/ would show a second
  # "Azura Launcher" when searching the Mac, so only the .app stays there.
  rm -rf "${BUNDLE}/dmg"

  # Clean ghost registrations in macOS LaunchServices caused by temporary DMG mount volumes
  readonly LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
  if [[ -x "${LSREGISTER}" ]]; then
    "${LSREGISTER}" -dump 2>/dev/null \
      | awk '/path:[[:space:]]*\/Volumes\/.*Azura Launcher\.app/ {sub(/^[[:space:]]*path:[[:space:]]*/, ""); sub(/[[:space:]]*\(0x[0-9a-f]+\)$/, ""); print}' \
      | while IFS= read -r p; do
          "${LSREGISTER}" -u "${p}" 2>/dev/null || true
        done
    "${LSREGISTER}" -gc 2>/dev/null || true
  fi
elif [[ "${KIT_PLATFORM}" == win-* ]]; then
  # The installer is what players download and what the updater runs to update.
  readonly ARCHIVE="Azura-Launcher_${VERSION}_x64-setup.exe"
  readonly GENERATED_INSTALLER="$(find "${BUNDLE}/nsis" -name "*-setup.exe" -print -quit)"
  if [[ -z "${GENERATED_INSTALLER}" ]]; then
    echo "No installer in ${BUNDLE}/nsis." >&2
    exit 1
  fi
  cp "${GENERATED_INSTALLER}" "${DIST}/${ARCHIVE}"
  cp "${GENERATED_INSTALLER}.sig" "${DIST}/${ARCHIVE}.sig"
else
  # The AppImage is what players download and what the updater replaces itself with.
  readonly ARCHIVE="Azura-Launcher_${VERSION}_amd64.AppImage"
  readonly GENERATED_APPIMAGE="$(find "${BUNDLE}/appimage" -name "*.AppImage" -print -quit)"
  cp "${GENERATED_APPIMAGE}" "${DIST}/${ARCHIVE}"
  cp "${GENERATED_APPIMAGE}.sig" "${DIST}/${ARCHIVE}.sig"
fi

readonly BASE_URL="${AZURA_UPDATE_BASE_URL:-$(release_download_url "${TAG}")}"
echo "==> Writing latest-${UPDATER_PLATFORM}.json"
node -e '
  const fs = require("fs");
  const [file, platform, url, sigFile] = process.argv.slice(1);
  const entry = { [platform]: { signature: fs.readFileSync(sigFile, "utf8").trim(), url } };
  fs.writeFileSync(file, JSON.stringify(entry, null, 2) + "\n");
' "${DIST}/latest-${UPDATER_PLATFORM}.json" "${UPDATER_PLATFORM}" "${BASE_URL}/${ARCHIVE}" "${DIST}/${ARCHIVE}.sig"

if ! ${PUBLISH}; then
  echo "==> Done (not published). Files in ${DIST}/"
  exit 0
fi

"${ROOT}/scripts/publish-launcher.sh" "${DIST}"
