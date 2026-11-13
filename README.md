# Azura Launcher

Desktop launcher for [Azura](https://alboria.org/en/azura/), the unofficial native port of
Gears of War 3 (Xbox 360) for macOS on Apple Silicon, Linux (x86-64) and Windows (x86-64), built with
[Tauri](https://tauri.app): the interface is web code (HTML, CSS and JavaScript) and a thin Rust
layer talks to the operating system.

**Its code is public so anyone can read what it does on their computer.** It is source-available
under the [PolyForm Strict License 1.0.0](LICENSE), not open source: you may read it and use it for
noncommercial purposes, but not change it or share it. The names Azura and Alboria and the artwork
are reserved ([TRADEMARKS.md](TRADEMARKS.md)).

**What is not here.** The game runtime (the recompiled game's support code, our Metal and Vulkan
backends, our changes to the ReXGlue SDK and Alboria's online client) is proprietary and lives in a
private repository. The launcher builds it on the player's machine from the *player kit* that every
published launcher bundles; this repository only pins which kit (`kit.toml`) and downloads it for a
release (`scripts/fetch-kit.sh`). There is no game code or data in either: the game is built from
each player's own copy.

What it does:

- choose the game files (a folder or a disc image) and check that they are a supported edition;
- build the runtime on the player's machine from their own `default.xex` with the player kit it
  carries, installing a C++ compiler when there is none (see Runtime and Compiler below);
- edit the runtime settings (game language, gamertag, graphics, display) that the runtime reads
  from `azura.toml`, manage the saves and show the achievements;
- sign the player in to Alboria, our online ecosystem, or let them play with a local account (see
  Account below);
- start the game and show its log;
- update itself, and with it the game, from the published releases (see Updates and releases);
- work with a mouse, a keyboard or a controller.

The interface is our own design with a theme inspired by the game. It uses only original artwork
and freely licensed fonts bundled with the app: no logos, fonts or images taken from the game or
from its publishers.

## Download

Azura and its launcher are downloaded from our website, which picks the right installer for your
system (macOS, Linux or Windows) and explains what you need before you start.

<a href="https://alboria.org/en/azura/"><img src=".github/download-button.svg" alt="Download" width="240" height="56"></a>

## Languages

The launcher follows the system language when it is one of the languages Gears of War 3 shipped
with (English, Czech, German, Spanish, French, Hungarian, Italian, Japanese, Korean, Polish,
Portuguese, Russian and Traditional Chinese) and falls back to English otherwise. The user can override it from the
language selector. Messages live in `src/locales/<code>.json`; `en.json` is the reference, and
every other file must have the same keys and placeholders:

```bash
npm test
```

## Settings split

Settings live in the app data folder (`~/Library/Application Support/com.gears3azura.launcher/`):
`runtime/azura.toml` holds what the runtime reads, under its own setting names (the game, a folder
or a disc image, is `game_data_root`), and `launcher.toml` what only the launcher uses. The launcher rewrites only its
own keys in `azura.toml`, keeping comments and anything the in-game menu wrote, and reloads the file
when its window regains focus.

The launcher only edits the settings the runtime reads at start-up: the display mode and the
render resolution. The render resolution (`render_resolution`: Automatic, 1280x720, 2560x1440 or
3840x2160) is the same setting as the Render resolution row of the game's Video menu; the runtime
turns it into the draw resolution scale and the texture memory when the game starts, and resolves
Automatic for the machine with the rule in `src/graphics.js` (2560x1440 on Pro, Max and Ultra chips
with at least 16 GB, else 1280x720). Before the first launch after an update, a scale written by an
older launcher (`draw_resolution_scale_x`, with `resolution_auto` in `launcher.toml`) becomes
`render_resolution`. Everything else (frame rate limit, anti-aliasing, motion blur, filtering, image
scaling, mouse sensitivity, bindings) is changed in the game's Options menu.
Both sides read and write the same `azura.toml`.

## Runtime

The runtime is the game recompiled for this computer. Players build it with the launcher from
their own `default.xex` (`src-tauri/src/builder.rs`): the player kit (built and published by the
private runtime repository, pinned in `kit.toml`) holds everything but the game, and `kit.json` the
commands to build it, which the launcher runs with the machine's compiler. Install in the
Installation view, or Play when nothing is installed, does it: codegen with the kit's code
generator in `build/<game build>/` under the app data folder, the generated files compiled one per
core, the link, and the new runtime folder swapped in by renames, keeping the player's
`azura.toml` and logs. Each object is reused while its key matches (its source, the command, the
compiler and the generated headers the compiler reported it used), so an interrupted build
resumes. Progress arrives as `runtime-build-progress` events.

Updates: `build/<game build>/state.json` records the kit's keys, the game executable's hash, the
compiler and the objects the installed runtime was linked from. When a newer kit only changes what
the link uses (our code, the SDK libraries, the staged files) and the objects are still there, the
runtime is linked again from them in under a second; when the generated code depends on what
changed (the code generator, the manifest, the headers, the commands), or the game or the compiler
changed, it is built again, compiling only the files whose key changed. The Installation view says
which an update takes, and Play installs it before starting the game.

Where the runtime comes from, in order: `AZURA_RUNTIME_BUILD` (a build folder, copied),
`AZURA_KIT` (a kit folder, built from), a runtime build of the Azura runtime project when the
launcher runs from a checkout of it (copied, as developers do), then the kit bundled with the
launcher. To try a kit locally, `scripts/fetch-kit.sh` downloads the pinned one into `kit/<platform>/`;
point `AZURA_KIT` at it.

`cargo test builds_a_runtime -- --ignored` builds a real runtime with the launcher's code, given
`AZURA_TEST_KIT` (a kit) and `AZURA_TEST_GAME` (a game folder or disc image).

The game is the folder of the player's dump or its disc image (`.iso`), picked with either button
of the Installation view or dropped on it. A disc image is read in place (`src-tauri/src/disc_image.rs`)
and the runtime mounts it itself; see `alboria-notes:azura/player-runtime-build.md`, section 5.

## Compiler

The runtime is built on the player's machine from their own `default.xex`
(`alboria-notes:azura/player-runtime-build.md`), so the launcher needs a C++ compiler there; the Installation
view shows its state and installs one when it is missing (`src-tauri/src/toolchain.rs`). A
compiler counts only when it compiles and links a small C++23 program. On macOS it is Apple's,
from the Command Line Tools or Xcode; `xcrun` is run only once `xcode-select -p` names a folder,
since on a Mac without them it would open Apple's install dialog by itself. Installing runs
`xcode-select --install`, which opens that dialog for the player, and the launcher checks again
every few seconds until the tools are there or the dialog is closed. On Linux it is the newest
`clang++` (clang 18 or newer) on the PATH; installing runs the distribution's package manager
(apt, dnf, pacman or zypper, found from `/etc/os-release`) through `pkexec`. Image-based systems
(ostree ones such as Bazzite, and SteamOS) get the manual instructions instead. The manual command
is always shown too.

## Achievements

The Achievements section lists the game's achievements with their artwork, the gamerscore earned and
the Embry Star (named after the game's own medal), earned by unlocking all of them. The player picks a
list, a grid of badges or sections by act (`achievementsView` in `launcher.toml`).

Nothing about the achievements ships with the launcher. Their names, descriptions in every language,
gamerscore, flags and artwork are in the XDBF resource of the player's `default.xex`, which is
encrypted, so the launcher asks the runtime: `azura --game_data_root=<game> --export_achievements=<folder>`
loads the executable without a window or a GPU, writes `achievements.json` and `icons/<imageId>.png`,
and exits (`runtime/src/achievement_export.h`). The launcher runs it once per game build and keeps the
result in `achievements/<build>/` under its app data folder. The unlocks and their dates come from the
file the runtime writes in the saves folder, `achievements/<title ID>.toml`. An achievement the
original game hid (its flags lack 0x8) is sent to the interface without its name, description,
gamerscore or artwork until it is unlocked. The section stays locked, with a button to the
Installation view, until there is a valid game folder and an installed runtime to read it.

## Controllers

The launcher can be driven with a controller. `src-tauri/src/gamepad.rs` reads the controllers
(the webview's own Gamepad API depends on how each platform's WebKit was built) and sends the
interface one `gamepad-input` event per action, only while the launcher window has the focus. On
macOS it uses Apple's GameController framework, the one that receives a DualSense's input over
Bluetooth (reading the HID device gets only its connection); on Linux, gilrs.

`src/gamepadNav.js` applies the actions: the D-pad and the left stick move the focus to the nearest
control in that direction (`src/gamepad.js`, within the rail or within the section), South (A,
Cross) presses, East (B, Circle) cancels a prompt or goes back to Play, the shoulder buttons switch
sections, Start starts the game; left and right change selects and sliders. South on a text field
(the gamertag, an email, a code) opens the on-screen keyboard (`src/virtualKeyboard.js`): West deletes, North types a
space, Start is Done, East cancels. While a controller is in use, a hint bar shows its buttons with
the glyphs of its family (Xbox and Steam Deck, PlayStation, Nintendo or generic), drawn in
`src/gamepadGlyphs.js`; the mouse or the keyboard hides it again. Nothing reacts while the game
runs. A cancel button that should answer East marks itself with `data-pad-back`. A dialog over the
launcher (the welcome screen, `data-modal`) keeps the focus among its own controls while it is
open.

## Account

`src/account.js` and `src-tauri/src/alboria.rs`. The player plays with an Alboria account (the
protocol is in `alboria-notes:azura/alboria-relay.md`, sections 3 and 4.6) or with a local account. Until they
choose, also after updating from a launcher without accounts, a welcome screen asks: create an
account, sign in to one, or play locally. The choice is `account` in `launcher.toml` and sets the
game's `network_mode` in `azura.toml`: `online` for an Alboria account, `system_link` for a local
one.

- An account has no password. A 6-digit code sent to its email creates it, links this installation
  to it, moves it to another email or deletes it.
- This installation is a device of the account, with the Ed25519 key in `identity.toml` in the
  saves folder, which the game also reads; the launcher makes the file, in the game's format, when
  the game has not run yet. The key never leaves the computer and is never logged.
- When it links the device, the launcher sends the hash of the computer's own ID (section 3.2), so
  that a banned player cannot come back with a new account from the same computer.
- The gamertag lives in the account. With an Alboria account, saving a new one signs in again so
  that Alboria gives the tag for the new name; with a local account it is saved as it is typed.
- Two buttons open `https://alboria.org` in the system browser: a small "with Alboria" pill under
  the brand (hidden in the narrow layout) and one in the Account section. They go through the
  `open_alboria_site` command (`tauri-plugin-opener`), whose address is fixed in Rust, so the
  interface cannot open any other.
- Requests go to `https://api.alboria.org`, or to `ALBORIA_API_URL` for a development server
  (`ALBORIA_MAILER=console` on that server prints the codes). In a browser, `src/platform.js`
  simulates the service: the code is always 123456, and `taken@example.com` already has an account.

On Linux, building needs libudev (`libudev-dev`), and `cargo test prints_controller_events --
--ignored --nocapture` prints what each controller sends.

## Layout

| Path | What it holds |
| --- | --- |
| `src/` | The interface: plain HTML, CSS and JavaScript, no build step. Tauri serves it as is. |
| `src/fonts/` | Bundled OFL fonts and their licenses, generated by `scripts/fetch-fonts.mjs`. |
| `src-tauri/` | The native side (Rust): window, security policy and the commands the interface calls. |
| `scripts/` | Build and release scripts (`build-launcher.sh`, `release-launcher.sh`, `publish-launcher.sh`, `fetch-kit.sh`), `fetch-fonts.mjs` (the bundled fonts) and `generate-licenses.mjs` (the Rust crates' licenses, run before each build). |
| `versions.toml` | The game builds the launcher supports, identified by the SHA-256 of their `default.xex`. The runtime repository keeps the same file for its own build; change both together. |
| `kit.toml` | The player kit each release bundles: its revision and per-platform SHA-256. |
| `test/` | Node tests for the locale, game languages, graphics, controller, build progress and achievements logic. |

## Requirements

- Node.js 20+ and npm.
- Rust (stable), for example `brew install rustup` and then `rustup default stable`. Homebrew does
  not put rustup on the PATH: add `/opt/homebrew/opt/rustup/bin` to it.
- On Linux, the libraries Tauri and the controller support build against: on Ubuntu,
  `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
  build-essential libudev-dev` (as `.github/workflows/launcher-tests.yml` installs them).
- For the Windows launcher, built on Linux: LLVM with `clang-cl`, `llvm-lib` and `llvm-rc` (the
  `clang` and `llvm` packages), and NSIS (`sudo apt install nsis`).

## Run and build

From the repository root:

```bash
npm install
npx tauri dev
```

`npx tauri build` produces `Azura Launcher.app` and a `.dmg` under `src-tauri/target/release/bundle/`
on macOS, and the packages of `tauri.conf.json` on Linux (releases use the AppImage, see Updates
and releases).

### The development launcher, Azura (DEV)

`scripts/build-launcher.sh --dev` builds `Azura (DEV).app`, a launcher that lives next to the
published one without touching it. `src-tauri/tauri.dev.conf.json`, laid over `tauri.conf.json`,
gives it its own name, bundle identifier (`com.gears3azura.launcher.dev`, so its settings, runtime,
achievements and backups live in their own app data folder), icon with a DEV band and window title.
The `dev` feature gives it its own default saves folder (`Games Saves/Gears3-Azura (DEV)`, and so
its own Alboria identity: link it to an account with a code like any new device) and turns its
updater off, so it never becomes the published launcher. It installs the runtime that
`scripts/build.sh` built in the Azura runtime project (`AZURA_PROJECT` when it is compiled, or a
checkout named `Azura` beside this repository),
also once the app is copied to /Applications.

It talks to a local Alboria server by default, so test accounts never reach the real database:
start one with `npm run local` in alboria-server (API on `http://127.0.0.1:8080`, relay on
127.0.0.1, codes in that terminal). Account › Development switches it to production or another
address; the choice is kept in `alboria-server.txt` in its app data folder, changing it signs in
again (each server has its own accounts), and the game it starts gets the same server
(`--alboria_api_url`). The published launcher always uses production and shows none of this.

#### Two players on one computer

Online features (matchmaking, invitations, joining a friend) can be tried on one computer with a
second game started by hand next to the one Azura (DEV) starts. The second one needs:

- **Its own saves folder**, outside `~/Documents` (a game started outside the launcher waits on the
  Documents access prompt): copy the DEV saves folder there, without its `identity.toml`, so it has
  the same profile and settings.
- **Its own Alboria account**: an `identity.toml` in that folder whose `key_seed` is the key of
  another account on the local server, such as a test account made with a script that creates it
  through `/v1/account/code` and `/v1/account/verify` (the code appears in the server's terminal).
  Its `xuid` (`E000` and 12 random hex digits) and `machine_id` (a locally administered MAC, 12 hex
  digits) must differ from the first game's, or the game takes both for one console.
- **The local server's address**: `--alboria_api_url=http://127.0.0.1:8080`. Without it the game
  goes to production, which does not know the test account and refuses the sign-in (`no_account`).
- **Its own gamertag**: `--user_name=<name>`. Without it the game takes the copied profile's name,
  and the test account is renamed after the first game's player.

Started from the DEV runtime folder (`com.gears3azura.launcher.dev/runtime` in the app data):

```bash
./azura "--game_data_root=<game folder>" "--user_data_root=<second saves>" \
  "--cache_root=<second cache>" "--alboria_api_url=http://127.0.0.1:8080" "--user_name=<name>"
```

Both games write their logs to the same `logs` folder, numbered in the order they started. Close
the second game when the test ends: only one game at a time otherwise.

### Windows

The launcher builds for Windows (x86-64) from Linux with `scripts/build-launcher-windows.sh`, into
`dist/windows/`, without a player kit: for a local test of the interface. Releases are built on
Windows itself by the Release workflow (`scripts/release-launcher.sh` in Git Bash), with the
`win-amd64` kit inside, as on the other platforms. On Windows the launcher names the PC it runs on
(processor, memory and graphics cards), builds the game with LLVM's clang and the Build Tools'
headers and libraries (`toolchain.rs` installs both with winget), and starts `azura.exe`. The script
cross-builds with [cargo-xwin](https://github.com/rust-cross/cargo-xwin), which it installs when
missing and which downloads Microsoft's CRT and Windows SDK once (about 1.1 GB, in
`~/.cache/cargo-xwin`), LLVM's `clang-cl`, `llvm-lib` and `llvm-rc`, and rustup's `rust-lld` as
the linker, and makes two files:

- `Azura Launcher_<version>_x64-setup.exe`, the installer (NSIS, set up in `bundle.windows.nsis` of
  `src-tauri/tauri.conf.json`). It installs for the current user, without administrator rights,
  with a Start menu entry, and registers the launcher in Windows' installed apps, from where its
  uninstaller removes it. The uninstaller offers to delete the launcher's data as well
  (`%APPDATA%\com.gears3azura.launcher`: settings, runtime, builds; and the interface's cache in
  `%LOCALAPPDATA%\com.gears3azura.launcher`); the saves, in
  `Documents\Games Saves\Gears3-Azura`, are never touched. It is offered in the interface's
  languages that Tauri's installer is translated to, and in English otherwise;
- `azura-launcher.exe`, the launcher alone, to run without installing.

Both need only what Windows 10 and 11 bring (WebView2 included; the installer gets it when
missing). They are not signed, so SmartScreen warns the first time. Released launchers update
themselves through the installer, which the updater runs.

The interface also opens in any browser, which is handy for quick design changes:

```bash
python3 -m http.server 5180 --directory src
```

## Updates and releases

Installed launchers update themselves with Tauri's updater. On start-up the launcher reads
`latest.json` from `https://updates.alboria.org/launcher-updates/`; when it lists a
newer version, the sidebar offers it, and installing downloads the archive, checks its signature
against `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` and restarts into it. Each launcher
carries the player kit of its platform, so a new launcher is also how fixes to the game reach
players: once it has updated, it rebuilds or relinks the runtime from the new kit.

Releases are published to Alboria's download host, `updates.alboria.org`: a Cloudflare R2 bucket,
`azura-updates`, public only through that domain (`scripts/release-common.sh`, and the updater
endpoint in `tauri.conf.json`). Each version's files go under `launcher-v<version>/`, the newest
installers again under `latest/` with fixed names (the website's download links), and `latest.json`
under `launcher-updates/`. Launchers older than 0.6.9 read `latest.json` from the release
`launcher-updates` of the GitHub repository `Carolux97/azura-releases`, which keeps only that file
(no downloads, it points at `updates.alboria.org`); the release workflow updates it too.

### How a release happens

A fix to the game (the runtime) reaches players in a new kit: publish it from the runtime
repository (its "Publish kit" workflow), paste the `kit.toml` lines it prints here, and release a
new launcher version with it.

Bump the version in `src-tauri/tauri.conf.json`, `package.json` and `src-tauri/Cargo.toml`, and
add its section to `CHANGELOG.md` (what the version brings, as players see it), in a pull request.
The section is the update's notes, and the launcher shows it once in Play after it is installed or
updated (`src-tauri/src/release_notes.rs`; the player's dismissal is `seen_release_notes` in
`launcher.toml`). A release without it fails, and so does the Rust test that reads the current
version's notes. When it is merged, the Release workflow (`.github/workflows/release.yml`) sees a
version that is not released yet and:

1. downloads the player kit `kit.toml` pins for each platform and checks its hash
   (`scripts/fetch-kit.sh`);
2. builds the launcher on macOS (the `.dmg`, and the `.app.tar.gz` the updater downloads), on
   Linux (the AppImage, which is both) and on Windows (the installer), each with its kit inside
   (`scripts/release-launcher.sh`);
3. uploads them to `launcher-v<version>/` and `latest/` on the download host, then the
   `latest.json` that lists every platform to `launcher-updates/` there and on the legacy GitHub
   release (`scripts/publish-launcher.sh`).

Other pushes to main do not start it. To check the whole pipeline without publishing, run
the workflow by hand (Actions, Release, Run workflow) with "Publish" off; with it on, it publishes a
version that is not released yet, which is also how to run a failed release again.

The workflow fails with a clear message when the three versions disagree, or when a version asked
to be published by hand is already released.

### Keys and secrets

Releases are signed with a key that never enters the repository. It was created once with
`npx tauri signer generate -w ~/.tauri/azura-launcher.key` (it asks for a password); its public half
is `plugins.updater.pubkey`. Keep both the file and the password in your password manager: losing
either means installed launchers can never be updated again.

The workflow needs five repository secrets:

- `TAURI_SIGNING_PRIVATE_KEY`: the contents of `~/.tauri/azura-launcher.key`;
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password;
- `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY`: an R2 API token (Cloudflare, R2, Manage API
  tokens) with Object Read & Write on the bucket `azura-updates` only;
- `RELEASES_TOKEN`: a fine-grained personal access token with access to `Carolux97/azura-releases`
  only and Contents set to read and write, for the legacy copy of `latest.json`.

### macOS without an Apple Developer ID

The app is signed ad hoc (`bundle.macOS.signingIdentity` is `-`), not with an Apple Developer ID,
and it is not notarized, so macOS blocks the first launch of a downloaded copy. The player opens it
once from System Settings, Privacy & Security, "Open Anyway" (Control-click, Open no longer works
since macOS 15). With a Developer ID, the same build signs and notarizes when Tauri finds Apple's
credentials in the environment (`APPLE_CERTIFICATE`, `APPLE_ID` and the rest), with no other change.

### Releasing by hand

The same scripts run on a Mac or a Linux machine with the platform's kit (`scripts/fetch-kit.sh`):

```bash
scripts/fetch-kit.sh
TAURI_SIGNING_PRIVATE_KEY=~/.tauri/azura-launcher.key TAURI_SIGNING_PRIVATE_KEY_PASSWORD=... scripts/release-launcher.sh --publish
```

Without `--publish` it only builds into `dist/<version>/`. A release by hand carries only the
platform it was built on, so Linux launchers are not offered it; the workflow is the normal way.

## App icon

The app icon is drawn in `src-tauri/icons/app-icon.svg`; regenerate every size with
`npx tauri icon src-tauri/icons/app-icon.svg -o src-tauri/icons` (then delete the `android` and
`ios` folders it also creates). The development launcher's icon is `app-icon-dev.svg`, the same
with a DEV band, generated the same way into `src-tauri/icons/dev`.

## License

Source-available under the [PolyForm Strict License 1.0.0](LICENSE). Copyright 2026 Carolina Molina
Elvira and Juan Llorca Martínez. The names and artwork are reserved ([TRADEMARKS.md](TRADEMARKS.md));
third-party code keeps its own licenses ([THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). Gears of
War is a trademark of Microsoft; this project is not affiliated with or endorsed by Microsoft, Epic
Games or The Coalition.
