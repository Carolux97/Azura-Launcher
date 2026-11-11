# Shared by release-launcher.sh and publish-launcher.sh (sourced, not run): where releases are
# published and the launcher's version.
#
# Releases are served from Alboria's own download host: the Cloudflare R2 bucket azura-updates,
# public only through the custom domain updates.alboria.org. Each version's files go under
# launcher-v<version>/, the newest installers also under latest/ with fixed names (the website's
# download links), and the updater's latest.json under launcher-updates/. The updater endpoint in
# src-tauri/tauri.conf.json points at that latest.json; change both together.
#
# Launchers older than 0.6.9 read latest.json from the GitHub repository Carolux97/azura-releases,
# which keeps only that file (no downloads), pointing at updates.alboria.org.

readonly DOWNLOADS_URL="https://updates.alboria.org"
readonly R2_BUCKET="azura-updates"
# The S3-compatible endpoint of the Cloudflare account that holds the bucket (the account ID is not
# a secret). Uploads need AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY: an R2 API token that can only
# write to this bucket.
readonly R2_ENDPOINT="https://6055806f7858e57c2d2e0cf2ebd30105.r2.cloudflarestorage.com"

# The GitHub repository whose release "launcher-updates" still holds latest.json for launchers
# older than 0.6.9, and the folder (on R2) and release (on GitHub) of the updater's latest.json.
readonly LEGACY_RELEASES_REPO="Carolux97/azura-releases"
readonly UPDATES_TAG="launcher-updates"

# Prints a JSON field of a file with Node, which the launcher already requires.
json_field() {
  node -e 'process.stdout.write(String(require(process.argv[1])[process.argv[2]]))' "$1" "$2"
}

# Prints the launcher's version after checking tauri.conf.json, package.json and Cargo.toml agree.
launcher_version() {
  local launcher="${ROOT}"
  local version package_version cargo_version
  version="$(json_field "${launcher}/src-tauri/tauri.conf.json" version)"
  package_version="$(json_field "${launcher}/package.json" version)"
  cargo_version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "${launcher}/src-tauri/Cargo.toml" | head -n 1)"
  if [[ "${version}" != "${package_version}" || "${version}" != "${cargo_version}" ]]; then
    echo "Versions differ: tauri.conf.json ${version}, package.json ${package_version}, Cargo.toml ${cargo_version}. Bump all three to the same version." >&2
    return 1
  fi
  printf '%s' "${version}"
}

# Prints the list items of a version's section of CHANGELOG.md, as the launcher shows them
# (src-tauri/src/release_notes.rs reads the same file); fails when the section is missing or empty.
release_notes() {
  local notes
  notes="$(awk -v heading="## $1" '
    $0 == heading { inside = 1; next }
    inside && /^## / { exit }
    inside && /^- / { print; next }
    inside && /^  [^ ]/ { sub(/^ +/, "  "); print }
  ' "${ROOT}/CHANGELOG.md")"
  if [[ -z "${notes}" ]]; then
    echo "CHANGELOG.md has no notes for version $1: add its section first." >&2
    return 1
  fi
  printf '%s\n' "${notes}"
}

# The release tag of a version.
release_tag() {
  printf 'launcher-v%s' "$1"
}

# Where a release's files are downloaded from.
release_download_url() {
  printf '%s/%s' "${DOWNLOADS_URL}" "$1"
}

# Prints the version of the published latest.json, or nothing when none is published yet; fails
# when the download host cannot be reached, so a network error never passes for a new version.
published_version() {
  local manifest http_status version
  manifest="$(mktemp)"
  http_status="$(curl -sS -o "${manifest}" -w '%{http_code}' "${DOWNLOADS_URL}/${UPDATES_TAG}/latest.json")" || {
    rm -f "${manifest}"
    return 1
  }
  case "${http_status}" in
    200)
      version="$(node -e 'process.stdout.write(String(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).version))' "${manifest}")" || {
        rm -f "${manifest}"
        return 1
      }
      ;;
    404) version="" ;;
    *)
      rm -f "${manifest}"
      echo "Cannot read ${DOWNLOADS_URL}/${UPDATES_TAG}/latest.json (HTTP ${http_status})." >&2
      return 1
      ;;
  esac
  rm -f "${manifest}"
  printf '%s' "${version}"
}

# Succeeds when a version is the one published (versions only go forward, so an older one is never
# published again).
release_exists() {
  local published
  published="$(published_version)" || exit 1
  [[ "${published}" == "$1" ]]
}

# Runs the AWS CLI against the R2 bucket's endpoint (R2 has no regions; "auto" stands for one).
r2() {
  AWS_DEFAULT_REGION=auto aws --endpoint-url "${R2_ENDPOINT}" "$@"
}
