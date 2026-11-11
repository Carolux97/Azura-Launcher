/*
 * Writes the license texts of every Rust crate compiled into the launcher, so they ship inside the
 * macOS .dmg and the Linux packages (their licenses require the notices to travel with binaries).
 *
 * Writes src-tauri/licenses/rust-crates.txt, which tauri.conf.json bundles as a resource. Tauri
 * runs this script before every `tauri build` (build.beforeBuildCommand), so the file always
 * matches Cargo.lock; it is not committed.
 *
 * The crates come from `cargo metadata` for every target the launcher is built for, following only
 * normal dependencies from the launcher itself (build scripts and tests are not shipped). Each
 * crate's license files (LICENSE*, LICENCE*, COPYING*, NOTICE*) are read from its source folder;
 * identical texts are written once, followed by the crates that use them.
 *
 * Usage (from the repository root): node scripts/generate-licenses.mjs
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const LAUNCHER = join(dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST = join(LAUNCHER, "src-tauri", "Cargo.toml");
const OUT_DIR = join(LAUNCHER, "src-tauri", "licenses");
const OUT_FILE = join(OUT_DIR, "rust-crates.txt");

// Platforms the launcher is packaged for: the macOS .dmg and the Linux packages.
const TARGETS = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];
const LICENSE_FILE = /^(licen[cs]e|copying|notice)/i;

/** Crates reachable from the launcher through normal dependencies on `target`, by package id. */
const shippedCrates = (target) => {
  const json = execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--locked", "--manifest-path", MANIFEST, "--filter-platform", target],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  const metadata = JSON.parse(json);
  const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const seen = new Set();
  const pending = [metadata.resolve.root];
  while (pending.length > 0) {
    const id = pending.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id)?.deps ?? []) {
      // dep_kinds lists how the dependency is used; kind null is a normal (shipped) dependency.
      if (dep.dep_kinds.some((kind) => kind.kind === null)) pending.push(dep.pkg);
    }
  }
  seen.delete(metadata.resolve.root);
  return [...seen].map((id) => packages.get(id)).filter((pkg) => pkg.source !== null);
};

/** The license texts found in a crate's source folder, one per file. */
const licenseTexts = (pkg) => {
  const folder = dirname(pkg.manifest_path);
  return readdirSync(folder)
    .filter((name) => LICENSE_FILE.test(name) && statSync(join(folder, name)).isFile())
    .sort()
    .map((name) => readFileSync(join(folder, name), "utf8").trim());
};

const crates = new Map();
for (const target of TARGETS) {
  for (const pkg of shippedCrates(target)) crates.set(`${pkg.name} ${pkg.version}`, pkg);
}

// Group crates by license text, so a text shared by many crates (a plain Apache 2.0, for example)
// is written once with the list of crates under it.
const groups = new Map();
const withoutText = [];
for (const [label, pkg] of [...crates].sort(([a], [b]) => a.localeCompare(b))) {
  const texts = licenseTexts(pkg);
  if (texts.length === 0) {
    withoutText.push(`${label} (${pkg.license ?? "license not declared"})${pkg.repository ? ` ${pkg.repository}` : ""}`);
    continue;
  }
  for (const text of texts) {
    const key = createHash("sha256").update(text).digest("hex");
    if (!groups.has(key)) groups.set(key, { text, crates: [] });
    groups.get(key).crates.push(label);
  }
}

const rule = "=".repeat(78);
const parts = [
  "Licenses of the Rust crates compiled into the Azura launcher",
  `Generated from Cargo.lock for ${TARGETS.join(", ")}: ${crates.size} crates.`,
  "",
];
for (const { text, crates: users } of groups.values()) {
  parts.push(rule, `Used by: ${users.join(", ")}`, rule, "", text, "");
}
if (withoutText.length > 0) {
  parts.push(rule, "Crates whose source has no license file (their declared license applies):", rule, "");
  parts.push(...withoutText, "");
}

mkdirSync(OUT_DIR, { recursive: true });
writeFileSync(OUT_FILE, parts.join("\n"));
console.log(`Wrote ${OUT_FILE}: ${crates.size} crates, ${groups.size} distinct license texts.`);
