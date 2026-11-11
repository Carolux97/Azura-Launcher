#!/usr/bin/env bash
# Downloads the player kit that kit.toml pins for a platform from Alboria's download host, checks
# its SHA-256 and unpacks it into kit/<platform>/, where scripts/release-launcher.sh bundles it.
#
# The kit is built and published by the private Azura runtime repository; see kit.toml.
#
# Usage: scripts/fetch-kit.sh [mac-arm64|linux-amd64|win-amd64]   (default: this machine's)

set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly DOWNLOADS_URL="https://updates.alboria.org"

platform="${1:-}"
if [[ -z "${platform}" ]]; then
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) platform="mac-arm64" ;;
    Linux-x86_64) platform="linux-amd64" ;;
    MINGW64_NT*-x86_64 | MSYS_NT*-x86_64) platform="win-amd64" ;;
    *)
      echo "No kit for $(uname -s) $(uname -m)." >&2
      exit 1
      ;;
  esac
fi
readonly PLATFORM="${platform}"

# A value of kit.toml: the line `<key> = "<value>"` (the platforms' hashes are under [sha256]).
kit_value() {
  sed -n "s/^$1 = \"\\(.*\\)\"\$/\\1/p" "${ROOT}/kit.toml" | head -n 1
}

readonly REVISION="$(kit_value revision)"
readonly EXPECTED="$(kit_value "${PLATFORM}")"
if [[ -z "${REVISION}" || -z "${EXPECTED}" ]]; then
  echo "kit.toml pins no kit for ${PLATFORM}: publish one from the Azura runtime repository first." >&2
  exit 1
fi

readonly NAME="azura-kit-${PLATFORM}.tar.gz"
readonly ARCHIVE="${ROOT}/kit/${NAME}"
readonly DEST="${ROOT}/kit/${PLATFORM}"
mkdir -p "${ROOT}/kit"

echo "==> Downloading the ${PLATFORM} kit ${REVISION}"
curl -fsSL --retry 3 -o "${ARCHIVE}" "${DOWNLOADS_URL}/kits/${REVISION}/${NAME}"

if command -v sha256sum >/dev/null; then
  actual="$(sha256sum "${ARCHIVE}" | cut -d ' ' -f 1)"
else
  actual="$(shasum -a 256 "${ARCHIVE}" | cut -d ' ' -f 1)"
fi
if [[ "${actual}" != "${EXPECTED}" ]]; then
  echo "The kit's SHA-256 is ${actual}, not the ${EXPECTED} that kit.toml pins." >&2
  rm -f "${ARCHIVE}"
  exit 1
fi

# The archive holds a kit/ folder; it becomes kit/<platform>/.
rm -rf "${DEST}" "${ROOT}/kit/.unpack-${PLATFORM}"
mkdir -p "${ROOT}/kit/.unpack-${PLATFORM}"
tar -xzf "${ARCHIVE}" -C "${ROOT}/kit/.unpack-${PLATFORM}"
mv "${ROOT}/kit/.unpack-${PLATFORM}/kit" "${DEST}"
rm -rf "${ROOT}/kit/.unpack-${PLATFORM}" "${ARCHIVE}"
echo "==> Kit in ${DEST}"
