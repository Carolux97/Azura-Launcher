use std::fs;
use std::path::Path;

/// Bundled resource with the license texts of the Rust crates, written by
/// scripts/generate-licenses.mjs before every `tauri build`.
const CRATE_LICENSES: &str = "licenses/rust-crates.txt";

fn main() {
  // tauri.conf.json bundles the crate licenses as a resource, and tauri_build fails when a resource
  // is missing. `tauri build` generates the real file first; a plain `cargo build` or `cargo test`
  // (tests, CI) does not, so it gets a placeholder that says how to generate the real one.
  if !Path::new(CRATE_LICENSES).exists() {
    fs::create_dir_all("licenses").expect("cannot create the licenses folder");
    fs::write(
      CRATE_LICENSES,
      "Crate licenses not generated: run `node scripts/generate-licenses.mjs` from the repository root.\n",
    )
    .expect("cannot write the crate licenses placeholder");
  }
  tauri_build::build()
}
