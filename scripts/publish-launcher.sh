#!/usr/bin/env bash
# Publishes a launcher release that scripts/release-launcher.sh built, so installed launchers
# update themselves:
#   1. merge the platforms' entries (latest-<platform>.json) into latest.json, the file the
#      launcher's updater reads (version, notes, date, and per platform the archive's URL and
#      signature);
#   2. upload the downloads and the update archives to launcher-v<version>/ on the download host
#      (release-common.sh), and the installers again under latest/ with fixed names;
#   3. upload latest.json to launcher-updates/, where the updater endpoint in tauri.conf.json
#      points, and to the GitHub release that launchers older than 0.6.9 still read.
#
# latest.json goes up last, so the updater never points at files not uploaded yet.
#
# A release may hold only the platforms that built. Publishing the same version again adds the
# platforms in <dist folder> (replacing their files) and merges their entries into the published
# latest.json, keeping the platforms already there.
#
# The notes shown with the update are the version's section of CHANGELOG.md.
#
# Credentials: AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY of an R2 API token that can only write
# to the bucket, and for the GitHub copy of latest.json, gh with write access to the legacy
# releases repository (a local `gh auth login`, or GH_TOKEN). The Release workflow passes both
# from its secrets.
#
# Usage:
#   scripts/publish-launcher.sh <dist folder>   publish the release built into that folder
#   scripts/publish-launcher.sh --status        print "released" or "new" for the current version
#                                               (failing for a new one without its notes)

set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT}/scripts/release-common.sh"

# Assigned before readonly so a failed check stops the script (set -e).
VERSION="$(launcher_version)"
readonly VERSION
readonly TAG="$(release_tag "${VERSION}")"

if [[ "${1:-}" == "--status" ]]; then
  if release_exists "${VERSION}"; then
    echo released
  else
    # A new version needs its notes before anything is built for it.
    release_notes "${VERSION}" >/dev/null
    echo new
  fi
  exit 0
fi

readonly DIST="${1:?Usage: scripts/publish-launcher.sh <dist folder> | --status}"
shopt -s nullglob
readonly ENTRIES=("${DIST}"/latest-*.json)
NOTES="$(release_notes "${VERSION}")"
readonly NOTES
if (( ${#ENTRIES[@]} == 0 )); then
  echo "No latest-<platform>.json in ${DIST}: build the release first (scripts/release-launcher.sh)." >&2
  exit 1
fi
RELEASED=false
release_exists "${VERSION}" && RELEASED=true
readonly RELEASED

# The published latest.json of this version, whose platforms are kept when adding others; an
# empty file for a new version (or a published one of an older version, which is replaced).
PUBLISHED="$(mktemp)"
readonly PUBLISHED
trap 'rm -f "${PUBLISHED}"' EXIT
if [[ "${RELEASED}" == true ]]; then
  curl -sSf -o "${PUBLISHED}" "${DOWNLOADS_URL}/${UPDATES_TAG}/latest.json"
fi

echo "==> Writing latest.json"
node -e '
  const fs = require("fs");
  const [file, published, version, notes, ...entries] = process.argv.slice(1);
  const text = fs.readFileSync(published, "utf8");
  const previous = text ? JSON.parse(text) : null;
  const kept = previous?.version === version ? previous.platforms : {};
  const platforms = Object.assign({}, kept, ...entries.map((entry) => JSON.parse(fs.readFileSync(entry, "utf8"))));
  const manifest = { version, notes, pub_date: previous?.version === version ? previous.pub_date : new Date().toISOString(), platforms };
  fs.writeFileSync(file, JSON.stringify(manifest, null, 2) + "\n");
' "${DIST}/latest.json" "${PUBLISHED}" "${VERSION}" "${NOTES}" "${ENTRIES[@]}"

# Every file of the release but the updater's own: the signatures are inside latest.json.
FILES=()
for file in "${DIST}"/*; do
  case "${file}" in
    *.json | *.sig) ;;
    *) FILES+=("${file}") ;;
  esac
done

# A version's files never change once published, so they may be cached for good; latest/ and
# latest.json change with every version, so they are never cached.
echo "==> Uploading ${TAG} to ${DOWNLOADS_URL}"
for file in "${FILES[@]}"; do
  r2 s3 cp "${file}" "s3://${R2_BUCKET}/${TAG}/$(basename "${file}")" --only-show-errors \
    --cache-control "public, max-age=31536000, immutable"
done

# The installers under fixed names, for the website's download links: the name without the version.
for file in "${FILES[@]}"; do
  name="$(basename "${file}")"
  case "${name}" in
    *.dmg | *.AppImage | *-setup.exe)
      r2 s3 cp "${file}" "s3://${R2_BUCKET}/latest/${name/_${VERSION}_/_}" --only-show-errors \
        --cache-control "no-cache" --content-disposition "attachment"
      ;;
  esac
done

echo "==> Publishing latest.json"
r2 s3 cp "${DIST}/latest.json" "s3://${R2_BUCKET}/${UPDATES_TAG}/latest.json" --only-show-errors \
  --cache-control "no-cache" --content-type "application/json"
gh release upload "${UPDATES_TAG}" --repo "${LEGACY_RELEASES_REPO}" --clobber "${DIST}/latest.json"

echo "==> Published ${TAG}"
