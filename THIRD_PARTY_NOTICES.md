# Third-party notices

The Azura Launcher's own code is source-available under the PolyForm Strict License 1.0.0 (see
[LICENSE](LICENSE)); its names and artwork are reserved (see [TRADEMARKS.md](TRADEMARKS.md)). It
builds on work by others, listed here with their licenses, which keep applying to that work.

## The launcher

The launcher is built with [Tauri](https://tauri.app) and depends on Rust crates and npm packages
listed in `src-tauri/Cargo.lock` and `package-lock.json`. The npm packages are build tools only;
the Rust crates compiled into the launcher are almost all under MIT or Apache License 2.0, plus a
few under BSD, Zlib, ISC, Unicode, Unlicense and MPL 2.0. The MPL 2.0 crates (`cssparser`,
`cssparser-macros`, `dtoa-short`, `option-ext` and `selectors`) are used unmodified; their source
is available on crates.io.

The launcher's fonts ship with their SIL Open Font License texts in `src/fonts/licenses/`.

## The player kit

Every published launcher bundles the player kit of its platform (`kit/` in its resources): what the
launcher needs to build the game on the player's machine. The kit is not part of this repository.
It is proprietary software of Carolina Molina Elvira and Juan Llorca Martínez, built from the
private Azura runtime, and carries its own `LICENSE` and `THIRD_PARTY_NOTICES.md` with the licenses
of the third-party code in it (the ReXGlue SDK and the libraries it bundles).

## What every package carries

Every package of the launcher carries these notices in its `licenses` resource folder (on macOS,
`Azura Launcher.app/Contents/Resources/licenses/`):

- `LICENSE` and `THIRD_PARTY_NOTICES.md`, this repository's license and this file;
- `rust-crates.txt`, the license texts of every Rust crate compiled into it, generated from
  `Cargo.lock` by `scripts/generate-licenses.mjs` before each `tauri build`.

## Game

Gears of War, its code and its data belong to their owners. They are not part of the launcher or
the kit: the game is built on each player's machine from their own copy.
