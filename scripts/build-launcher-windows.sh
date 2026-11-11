#!/usr/bin/env bash
# Builds the Azura Launcher for Windows (x86-64) from Linux, into dist/windows/:
#
#   - Azura Launcher_<version>_x64-setup.exe, the installer (NSIS): it installs the launcher for the
#     current user, without administrator rights, adds it to the Start menu and to Windows' installed
#     apps, from where its uninstaller removes it. The uninstaller offers to delete the launcher's
#     data too (%APPDATA%\com.gears3azura.launcher, and its interface cache in %LOCALAPPDATA%);
#     saves in Documents are never touched;
#   - azura-launcher.exe, the launcher alone, to run without installing.
#
# Only the launcher: the game has no Windows kit or runtime yet, so in Windows the launcher opens
# but cannot build or start the game. The .exe imports only system DLLs (the interface and the
# WebView2 loader are inside it); Windows 10 and 11 bring the WebView2 runtime it shows the
# interface with. It is not signed, so SmartScreen warns the first time it is opened.
#
# How it cross-builds (the only package from apt is nsis, for makensis):
#   - cargo-xwin (installed with cargo when missing) downloads Microsoft's CRT and Windows SDK to
#     its cache (about 1.1 GB, once; set XWIN_CACHE_DIR to put it elsewhere) and points the build
#     at them;
#   - clang-cl, llvm-lib and llvm-rc compile the C parts and the Windows resources: they come from
#     the newest /usr/lib/llvm-N with all three;
#   - the linker is rustup's rust-lld, which links as Microsoft's linker when called lld-link
#     (a link to it in the build folder), so lld does not need to be installed;
#   - `tauri build --runner cargo-xwin` builds through cargo-xwin and makes the installer with
#     makensis, with the settings in bundle.windows.nsis of tauri.conf.json.
#
# Usage: scripts/build-launcher-windows.sh

set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly LAUNCHER="${ROOT}"
readonly TARGET="x86_64-pc-windows-msvc"
readonly TARGET_DIR="${LAUNCHER}/src-tauri/target"
readonly OUTPUT_DIR="${LAUNCHER}/dist/windows"

# Where `cargo install` puts cargo-xwin, which tauri build runs by name.
if [[ -d "${HOME}/.cargo/bin" && ":${PATH}:" != *":${HOME}/.cargo/bin:"* ]]; then
  export PATH="${HOME}/.cargo/bin:${PATH}"
fi
for tool in cargo rustup; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "${tool} not found: install Rust with rustup first." >&2
    exit 1
  fi
done
for tool in node npm; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "${tool} not found: install Node.js and npm (sudo apt install npm)." >&2
    exit 1
  fi
done
if ! command -v makensis >/dev/null 2>&1; then
  echo "makensis not found: install NSIS (sudo apt install nsis)." >&2
  exit 1
fi

# The newest LLVM with clang-cl, llvm-lib and llvm-rc.
LLVM_BIN=""
for dir in $(ls -d /usr/lib/llvm-*/bin 2>/dev/null | sort -V -r); do
  if [[ -x "${dir}/clang-cl" && -x "${dir}/llvm-lib" && -x "${dir}/llvm-rc" ]]; then
    LLVM_BIN="${dir}"
    break
  fi
done
if [[ -z "${LLVM_BIN}" ]]; then
  echo "No LLVM with clang-cl, llvm-lib and llvm-rc in /usr/lib/llvm-*: install clang and llvm." >&2
  exit 1
fi
readonly LLVM_BIN

echo "==> Preparing the Windows target and cargo-xwin"
rustup target add "${TARGET}"
if ! command -v cargo-xwin >/dev/null 2>&1; then
  cargo install cargo-xwin --locked
fi

# rust-lld, called lld-link, links as Microsoft's linker.
readonly RUST_LLD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld"
if [[ ! -x "${RUST_LLD}" ]]; then
  echo "rust-lld not found at ${RUST_LLD}." >&2
  exit 1
fi
readonly LINKER_DIR="${TARGET_DIR}/xwin-linker"
mkdir -p "${LINKER_DIR}"
ln -sf "${RUST_LLD}" "${LINKER_DIR}/lld-link"

if [[ ! -d "${LAUNCHER}/node_modules" ]]; then
  echo "==> Installing the launcher's packages"
  (cd "${LAUNCHER}" && npm ci)
fi

readonly RELEASE="${TARGET_DIR}/${TARGET}/release"
rm -rf "${RELEASE}/bundle/nsis"

# tauri build writes the crate licenses first (beforeBuildCommand), then builds and packages.
echo "==> Building Azura Launcher and its installer for Windows with LLVM from ${LLVM_BIN}"
(cd "${LAUNCHER}" \
  && PATH="${LINKER_DIR}:${LLVM_BIN}:${PATH}" \
    npx tauri build --runner cargo-xwin --target "${TARGET}" --bundles nsis)

readonly INSTALLER="$(find "${RELEASE}/bundle/nsis" -name "*-setup.exe" -print -quit)"
if [[ -z "${INSTALLER}" ]]; then
  echo "No installer in ${RELEASE}/bundle/nsis." >&2
  exit 1
fi
rm -rf "${OUTPUT_DIR}"
mkdir -p "${OUTPUT_DIR}"
cp "${INSTALLER}" "${RELEASE}/azura-launcher.exe" "${OUTPUT_DIR}/"
echo "==> Done:"
echo "    ${OUTPUT_DIR}/$(basename "${INSTALLER}")"
echo "    ${OUTPUT_DIR}/azura-launcher.exe"
