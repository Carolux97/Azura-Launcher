//! Azura launcher: the native side of the Tauri app.
//!
//! The interface lives in `src/` as plain HTML, CSS and JavaScript. This crate opens the
//! window that shows it and exposes the commands the interface calls through `invoke`.

mod achievements;
mod alboria;
mod builder;
mod diagnostics;
mod disc_image;
mod dlc;
mod game_folder;
mod gamepad;
mod gpus;
mod host_env;
mod install;
mod release_notes;
#[cfg(test)]
mod test_support;
mod runtime;
mod saves;
mod settings;
mod system;
mod toolchain;
mod updater;
mod versions;

use std::path::Path;

use achievements::AchievementsState;
use alboria::{Alboria, AlboriaError, CodeSent, Identity, Session};
use game_folder::FolderCheck;
use install::RuntimeStatus;
use runtime::{GameProcess, LaunchError, LaunchFolders};
use saves::{BackupInfo, SaveStatus, SavesError};
use settings::{Settings, SettingsPaths};
use system::SystemInfo;
use toolchain::CompilerStatus;
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use updater::UpdateInfo;
use versions::Registry;

/// Opens the native folder picker and checks the chosen folder.
///
/// Returns `None` when the user cancels. It is `async` so Tauri runs it off the main thread: the
/// blocking picker would otherwise freeze the window it belongs to.
#[tauri::command]
async fn pick_game_folder(app: tauri::AppHandle) -> Option<FolderCheck> {
    let picked = app.dialog().file().blocking_pick_folder()?;
    let path = picked.into_path().ok()?;
    Some(game_folder::check_folder(&path, &Registry::bundled()))
}

/// Opens the native file picker for a disc image (`.iso`) of the game and checks it. Returns
/// `None` when the user cancels. `async` for the same reason as `pick_game_folder`.
#[tauri::command]
async fn pick_game_image(app: tauri::AppHandle) -> Option<FolderCheck> {
    let picked = app.dialog().file().add_filter("Xbox 360 disc image", &["iso"]).blocking_pick_file()?;
    let path = picked.into_path().ok()?;
    Some(game_folder::check_folder(&path, &Registry::bundled()))
}

/// Checks a folder chosen earlier, so the launcher can confirm on start-up that it is still there
/// and still holds the game.
///
/// `async` for the same reason as `pick_game_folder`: hashing the executable reads ~20 MB, which
/// must not stall the window.
#[tauri::command]
async fn check_game_folder(path: String) -> FolderCheck {
    game_folder::check_folder(Path::new(&path), &Registry::bundled())
}

/// Paths of the config files under this app's data folder.
fn settings_paths(app: &tauri::AppHandle) -> Result<SettingsPaths, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(SettingsPaths::new(&dir))
}

/// Folder that holds the runtime executable and its `azura.toml` (see `runtime.rs`).
fn runtime_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("runtime"))
}

/// Folder for the runtime's shader cache, under the app data folder.
fn cache_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("cache"))
}

/// Folder that keeps the achievements read from each game build (see `achievements.rs`).
fn achievements_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("achievements"))
}

/// The player's achievements for the Achievements section, for the game in `game_folder`, with
/// their texts in `game_language` (an engine code such as "ESN"), else in the launcher's language
/// (`ui_locale`), else in English. The interface passes the folder and the language it shows,
/// because a change it has just made may not be saved yet. The first call for a game build has the
/// runtime read the achievements from the game (about a second); later calls read what it kept.
///
/// `async`: it hashes the game executable to know its build, and may run the runtime.
#[tauri::command]
async fn get_achievements(
    app: tauri::AppHandle,
    game_folder: Option<String>,
    game_language: Option<String>,
    ui_locale: String,
) -> Result<AchievementsState, String> {
    let Some(folder) = game_folder else {
        return Ok(AchievementsState::NoGame);
    };
    let FolderCheck::Ok { folder, version_id, .. } = game_folder::check_folder(Path::new(&folder), &Registry::bundled())
    else {
        return Ok(AchievementsState::NoGame);
    };
    let runtime = runtime_dir(&app)?;
    let export_dir = achievements_dir(&app)?.join(&version_id);
    let export = match achievements::read_export(&export_dir) {
        Some(export) => export,
        // No runtime yet, or one that needs an update: an older one may not know
        // --export_achievements and would start the game instead. The section asks for it to be
        // installed, as Play would do first.
        None if !runtime_current(&app) => return Ok(AchievementsState::NoRuntime),
        None => match achievements::export(&runtime, Path::new(&folder), &export_dir, None) {
            Ok(export) => export,
            Err(detail) => return Ok(AchievementsState::Unavailable { detail }),
        },
    };
    let saves = settings_paths(&app)?.saves_folder()?;
    // RAAM's Shadow's own achievements, when it is installed (its database also holds the game's
    // and other packs', which are left out). A failed export leaves them out too, and so does a
    // runtime that needs an update (it may not know --export_achievements_from).
    let dlc_dir = export_dir.join("raams-shadow");
    let game_ids: std::collections::HashSet<u32> = export.achievements.iter().map(|exported| exported.id).collect();
    let dlc_export = dlc::raams_shadow_achievements(&saves).and_then(|spa| {
        achievements::read_export(&dlc_dir)
            .or_else(|| {
                runtime_current(&app)
                    .then(|| achievements::export(&runtime, Path::new(&folder), &dlc_dir, Some(&spa)).ok())
                    .flatten()
            })
            .map(|mut dlc_export| {
                dlc_export.achievements.retain(|exported| {
                    dlc::RAAMS_SHADOW_ACHIEVEMENTS.contains(&exported.id) && !game_ids.contains(&exported.id)
                });
                dlc_export
            })
    });
    let unlocks = achievements::read_unlocks(&achievements::unlocks_path(&saves, &export.title_id));
    let languages: Vec<&str> = game_language
        .as_deref()
        .and_then(achievements::text_language)
        .into_iter()
        .chain(std::iter::once(ui_locale.as_str()))
        .collect();
    let mut sources = vec![(&export, export_dir.as_path())];
    if let Some(dlc_export) = &dlc_export {
        sources.push((dlc_export, dlc_dir.as_path()));
    }
    Ok(achievements::build_state(&sources, &unlocks, &languages))
}

/// Where the runtime comes from. A developer's explicit choice wins (`AZURA_RUNTIME_BUILD`, then
/// `AZURA_KIT`); then a project build, when the launcher runs from a checkout that has one; then
/// the player kit (bundled with the launcher, or the project's kit build), which is how players
/// get it.
enum RuntimeSource {
    Build(std::path::PathBuf),
    Kit(std::path::PathBuf),
    None,
}

fn runtime_source(app: &tauri::AppHandle) -> RuntimeSource {
    if std::env::var_os("AZURA_RUNTIME_BUILD").is_some() {
        if let Some(build) = install::find_build() {
            return RuntimeSource::Build(build);
        }
    }
    if std::env::var_os(builder::KIT_ENV).is_none() {
        if let Some(build) = install::find_build() {
            return RuntimeSource::Build(build);
        }
    }
    let resources = app.path().resource_dir().ok();
    builder::find_kit(resources.as_deref()).map(RuntimeSource::Kit).unwrap_or(RuntimeSource::None)
}

/// The game build in the saved game folder, for building the runtime from it: its folder and its
/// entry in the registry.
fn game_build(app: &tauri::AppHandle) -> Result<(std::path::PathBuf, versions::KnownVersion), builder::BuildError> {
    let settings = settings_paths(app).and_then(|paths| paths.load()).map_err(builder::BuildError::Io)?;
    let folder = settings.game_folder.ok_or(builder::BuildError::UnsupportedGame)?;
    let registry = Registry::bundled();
    let FolderCheck::Ok { folder, version_id, .. } = game_folder::check_folder(Path::new(&folder), &registry) else {
        return Err(builder::BuildError::UnsupportedGame);
    };
    let version = registry.versions.into_iter().find(|version| version.id == version_id).ok_or(builder::BuildError::UnsupportedGame)?;
    Ok((std::path::PathBuf::from(folder), version))
}

/// The work folder the runtime of a game build is built in.
fn work_dir(app: &tauri::AppHandle, version: &versions::KnownVersion) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("build").join(&version.id))
}

/// Reports whether the runtime is installed and whether it has to be installed again: from a
/// project build, when the build is newer; from the kit, when the installed runtime was not built
/// from this kit for the game in the saved folder.
#[tauri::command]
async fn get_runtime_status(app: tauri::AppHandle) -> Result<RuntimeStatus, String> {
    runtime_status(&app)
}

/// The runtime status get_runtime_status reports, for the commands that run the runtime too.
fn runtime_status(app: &tauri::AppHandle) -> Result<RuntimeStatus, String> {
    let dir = runtime_dir(app)?;
    match runtime_source(app) {
        RuntimeSource::Build(build) => Ok(install::status(&dir, Some(&build))),
        RuntimeSource::None => Ok(install::status(&dir, None)),
        RuntimeSource::Kit(kit) => {
            let mut status = install::status(&dir, None);
            status.source = "kit";
            status.build_found = true;
            let kind = match (builder::read_kit(&kit), game_build(app)) {
                (Ok(kit), Ok((_, version))) => {
                    let work = work_dir(app, &version)?;
                    let state = builder::read_state(&work);
                    // Installed means built here from the kit for this game: a runtime folder
                    // that holds an executable from elsewhere (a copied project build) is not.
                    status.installed &= state.as_ref().is_some_and(|state| state.xex_sha256 == version.xex_sha256);
                    let kept = state.as_ref().is_some_and(|state| builder::objects_kept(&work, state));
                    let compiler = toolchain::compiler_version().unwrap_or_default();
                    builder::update_kind(state.as_ref(), &kit.keys, &version.xex_sha256, &compiler, status.installed, kept)
                }
                _ => builder::UpdateKind::Build,
            };
            status.update_available = kind != builder::UpdateKind::None;
            status.update_kind = match kind {
                builder::UpdateKind::None => None,
                builder::UpdateKind::Link => Some("link"),
                builder::UpdateKind::Build => Some("build"),
            };
            Ok(status)
        }
    }
}

/// Only one runtime build runs at a time.
static BUILDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Installs (or updates) the runtime: copies a project build (see `install.rs`), or builds it here
/// from the player kit and the player's default.xex (see `builder.rs`), sending its progress as
/// `runtime-build-progress` events. `async`: a build takes minutes.
#[tauri::command]
async fn install_runtime(
    app: tauri::AppHandle,
    process: tauri::State<'_, GameProcess>,
) -> Result<RuntimeStatus, builder::BuildError> {
    if process.is_running() {
        return Err(builder::BuildError::AlreadyRunning);
    }
    let dir = runtime_dir(&app).map_err(builder::BuildError::Io)?;
    match runtime_source(&app) {
        RuntimeSource::Build(build) => {
            install::install(&build, &dir).map_err(builder::BuildError::Io)?;
        }
        RuntimeSource::Kit(kit) => {
            let (game, version) = game_build(&app)?;
            let work = work_dir(&app, &version).map_err(builder::BuildError::Io)?;
            let events = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let folders = builder::BuildFolders { kit: &kit, game: &game, work: &work, runtime: &dir };
                builder::build(&folders, &version.manifest, &version.xex_sha256, &events, &BUILDING)
            })
            .await
            .map_err(|e| builder::BuildError::Io(e.to_string()))??;
        }
        RuntimeSource::None => return Err(builder::BuildError::NoKit),
    }
    get_runtime_status(app).await.map_err(builder::BuildError::Io)
}

/// Shows a failed build's log in the system's file manager. Only a `build.log` inside the
/// launcher's build folder is shown: the path comes from the interface.
#[tauri::command]
async fn show_build_log(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let builds = app.path().app_data_dir().map_err(|e| e.to_string())?.join("build");
    let log = std::path::PathBuf::from(&path).canonicalize().map_err(|e| e.to_string())?;
    let builds = builds.canonicalize().map_err(|e| e.to_string())?;
    if !log.starts_with(&builds) || log.file_name().and_then(|name| name.to_str()) != Some("build.log") {
        return Err(format!("not a build log: {path}"));
    }
    reveal(&log)
}

/// Shows `path` in the system's file manager: selected on macOS and Windows, its folder on Linux.
fn reveal(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg("-R").arg(path).status();
    // Explorer opens the folder with the file selected; it exits with 1 even when it does.
    #[cfg(windows)]
    let status = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).status();
    #[cfg(not(any(target_os = "macos", windows)))]
    let status = host_env::use_host_env(&mut std::process::Command::new("xdg-open"))
        .arg(path.parent().unwrap_or(path))
        .status();
    status.map(|_| ()).map_err(|e| e.to_string())
}

/// Starts the game with the chosen saves folder. Its output and its end arrive as `game-output`
/// and `game-exit` events.
/// With `join_code` (a friend's room or an invitation's), the game joins that room once its main
/// menu is up (`alboria_join_code`, script_hooks.cpp).
#[tauri::command]
async fn launch_game(
    app: tauri::AppHandle,
    process: tauri::State<'_, GameProcess>,
    join_code: Option<String>,
) -> Result<(), LaunchError> {
    match join_code {
        Some(code) if !room_code_valid(&code) => Err(LaunchError::StartFailed(format!("not a room code: {code}"))),
        Some(code) => play(app, &process, &[&format!("--alboria_join_code={code}")]),
        None => play(app, &process, &[]),
    }
}

/// Starts the game with `extra` arguments, then minimizes the launcher when the player asked for
/// that: minimized, its window is not drawn while the game runs, and it comes back when the game
/// ends (GameEvents::exit in runtime.rs).
fn play(app: tauri::AppHandle, process: &GameProcess, extra: &[&str]) -> Result<(), LaunchError> {
    // The development launcher sends the game to the Alboria it chose (dev_server); the published
    // one leaves the game on production, its default.
    let api_url = format!("--alboria_api_url={}", app.state::<Alboria>().base());
    let mut args: Vec<&str> = extra.to_vec();
    if cfg!(feature = "dev") {
        args.push(&api_url);
    }
    start_runtime(app.clone(), process, &args)?;
    let minimize = settings_paths(&app).and_then(|paths| paths.load()).is_ok_and(|settings| settings.minimize_while_playing);
    if minimize {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.minimize();
        }
    }
    Ok(())
}

// --- The development launcher's Alboria server ------------------------------------------------

/// The Alboria server `npm run local` starts (alboria-server, scripts/local.ts).
const LOCAL_API_URL: &str = "http://127.0.0.1:8080";
/// Where the development launcher keeps its choice, in its app data folder: `local`, `production`
/// or an address. Its own file, apart from the settings the interface saves.
const DEV_SERVER_FILE: &str = "alboria-server.txt";

/// The Alboria server the development launcher talks to: its choice and the address it means.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DevServer {
    /// Whether this is the development launcher; the published one has no choice.
    dev: bool,
    /// `local`, `production` or `other`.
    choice: String,
    url: String,
}

/// The server `saved` (the file's contents, or nothing) chooses: Local by default.
fn dev_server_from(saved: Option<&str>) -> DevServer {
    let saved = saved.map(str::trim).unwrap_or_default();
    let (choice, url) = match saved {
        "production" => ("production", alboria::DEFAULT_API_URL.to_string()),
        url if server_url_valid(url) => ("other", url.trim_end_matches('/').to_string()),
        _ => ("local", LOCAL_API_URL.to_string()),
    };
    DevServer { dev: true, choice: choice.into(), url }
}

/// Whether `url` can be a server's address: http or https, a host, no spaces.
fn server_url_valid(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && url.len() <= 200
        && url.split("://").nth(1).is_some_and(|rest| !rest.is_empty())
        && !url.chars().any(char::is_whitespace)
}

fn dev_server_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join(DEV_SERVER_FILE))
}

/// The Alboria server this launcher talks to (Account › Development in the development launcher).
#[tauri::command]
fn alboria_server(app: tauri::AppHandle, alboria: tauri::State<'_, Alboria>) -> DevServer {
    if !cfg!(feature = "dev") {
        return DevServer { dev: false, choice: "production".into(), url: alboria.base() };
    }
    let saved = dev_server_file(&app).ok().and_then(|file| std::fs::read_to_string(file).ok());
    dev_server_from(saved.as_deref())
}

/// Development launcher only: talks to `choice` (`local`, `production` or an address) from now on,
/// signed out, and remembers it. The game gets the same server the next time it starts.
#[tauri::command]
fn alboria_server_set(app: tauri::AppHandle, alboria: tauri::State<'_, Alboria>, choice: String) -> Result<DevServer, String> {
    if !cfg!(feature = "dev") {
        return Err("only the development launcher chooses its Alboria server".into());
    }
    let choice = choice.trim();
    if !["local", "production"].contains(&choice) && !server_url_valid(choice) {
        return Err(format!("not a server: {choice}"));
    }
    let file = dev_server_file(&app)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, choice).map_err(|e| e.to_string())?;
    let server = dev_server_from(Some(choice));
    alboria.set_base(server.url.clone());
    Ok(server)
}

/// The file in the saves folder (the runtime's user data) where the launcher leaves a room code
/// for a running game to join; the runtime reads and deletes it (script_hooks.cpp).
const JOIN_REQUEST_FILE: &str = "alboria-join-request.txt";

/// Whether `code` looks like a room code: 8 to 16 letters, digits and dashes.
fn room_code_valid(code: &str) -> bool {
    (8..=16).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Asks the running game to join the room `code` (a friend's room, or an invitation's, from
/// Alboria): the code is left in the saves folder, and the game joins once it is at its main menu.
/// Joining is the game's own: the launcher takes no place in the room. With the game closed,
/// launch_game takes the code instead.
#[tauri::command]
async fn join_room(app: tauri::AppHandle, process: tauri::State<'_, GameProcess>, code: String) -> Result<(), LaunchError> {
    if !room_code_valid(&code) {
        return Err(LaunchError::StartFailed(format!("not a room code: {code}")));
    }
    if !process.is_running() {
        return Err(LaunchError::StartFailed("the game is not running".into()));
    }
    let saves = settings_paths(&app).and_then(|paths| paths.saves_folder()).map_err(LaunchError::SavesUnavailable)?;
    // Written whole and then renamed, so the game never reads half a code.
    let partial = saves.join(format!("{JOIN_REQUEST_FILE}.part"));
    std::fs::write(&partial, &code)
        .and_then(|()| std::fs::rename(&partial, saves.join(JOIN_REQUEST_FILE)))
        .map_err(|e| LaunchError::StartFailed(e.to_string()))
}

/// The friends' requests the interface may send (alboria-notes:azura/alboria-relay.md, section
/// 4.7), and the report one: any other path is refused, so the interface reaches nothing else of
/// the account.
const FRIENDS_PATHS: &[&str] = &[
    "/v1/friends",
    "/v1/friends/request",
    "/v1/friends/accept",
    "/v1/friends/availability",
    "/v1/friends/decline",
    "/v1/friends/cancel",
    "/v1/friends/remove",
    "/v1/friends/favourite",
    "/v1/friends/block",
    "/v1/friends/unblock",
    "/v1/friends/room",
    "/v1/invitations",
    "/v1/invitations/room",
    "/v1/invitations/decline",
    "/v1/reports",
];

/// A friends' request (`path` one of FRIENDS_PATHS) of the signed-in account; the service's answer.
#[tauri::command]
async fn alboria_friends(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    path: String,
    body: serde_json::Value,
) -> Result<serde_json::Value, AlboriaError> {
    if !FRIENDS_PATHS.contains(&path.as_str()) {
        return Err(AlboriaError::local_error("bad_request", format!("not a friends request: {path}")));
    }
    alboria_account_request(&app, &alboria, &path, body).await
}

/// The events channel's task: which run is current, so a stopped one ends and a new one starts clean.
#[derive(Default)]
struct AlboriaEvents {
    run: std::sync::atomic::AtomicU64,
}

/// The event the interface hears each service event on (`{ event, data }`), and whether the
/// channel is open (`{ connected }`).
const SERVICE_EVENT: &str = "alboria-event";
const SERVICE_EVENTS_STATE: &str = "alboria-events-state";

/// Opens Alboria's events channel and keeps it open (section 4.7): each event goes to the interface
/// as `alboria-event`. When it closes (the token ran out, the network dropped, the service
/// restarted) it opens again, waiting longer each time it fails, up to a minute. Starting again
/// replaces the running one; alboria_events_stop ends it.
#[tauri::command]
fn alboria_events_start(app: tauri::AppHandle, events: tauri::State<'_, AlboriaEvents>) {
    use std::sync::atomic::Ordering;
    let run = events.run.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        let current = || app.state::<AlboriaEvents>().run.load(Ordering::SeqCst) == run;
        let mut wait = std::time::Duration::from_secs(2);
        while current() {
            let opened = match (alboria_identity(&app), alboria_gamertag(&app)) {
                (Ok(identity), Ok(gamertag)) => app.state::<Alboria>().open_events(&identity, &gamertag).await,
                (Err(error), _) | (_, Err(error)) => Err(error),
            };
            match opened {
                Ok(mut response) => {
                    let _ = app.emit(SERVICE_EVENTS_STATE, serde_json::json!({ "connected": true }));
                    wait = std::time::Duration::from_secs(2);
                    let mut stream = alboria::EventStream::default();
                    while current() {
                        match response.chunk().await {
                            Ok(Some(chunk)) => {
                                for event in stream.push(&chunk) {
                                    let _ = app.emit(SERVICE_EVENT, event);
                                }
                            }
                            _ => break,
                        }
                    }
                    let _ = app.emit(SERVICE_EVENTS_STATE, serde_json::json!({ "connected": false }));
                }
                Err(error) => {
                    log::info!("Alboria's events channel did not open: {} {}", error.error, error.message);
                    wait = (wait * 2).min(std::time::Duration::from_secs(60));
                }
            }
            if current() {
                tokio::time::sleep(wait).await;
            }
        }
    });
}

/// Ends the events channel (the player signed out or chose a local account).
#[tauri::command]
fn alboria_events_stop(events: tauri::State<'_, AlboriaEvents>) {
    events.run.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Prepares the stored shaders before playing: starts the runtime in a window with
/// `shader_precompile_only`, which makes every shader and pipeline of earlier runs with the game
/// suspended and then closes, so the drivers keep them compiled. Its progress arrives as
/// `game-output` lines `AZURA_SHADERS <done> <total>` and `AZURA_SHADERS_DONE <total>`, and its
/// end as `game-exit`, as for the game.
#[tauri::command]
async fn prepare_shaders(
    app: tauri::AppHandle,
    process: tauri::State<'_, GameProcess>,
) -> Result<(), LaunchError> {
    start_runtime(app, &process, &["--shader_precompile_only=true", "--fullscreen=false"])
}

/// Bytes of the stored pipeline descriptions (`cache/shaders/shareable/*.xpso`): they grow as the
/// game meets new pipelines, so a size above the one of the last Prepare shaders means there are new
/// ones to prepare. 0 when there are none.
#[tauri::command]
async fn shader_storage_bytes(app: tauri::AppHandle) -> Result<u64, String> {
    let folder = cache_dir(&app)?.join("shaders").join("shareable");
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(0);
    };
    Ok(entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "xpso"))
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum())
}

/// Starts the runtime as the game, with `extra` arguments.
fn start_runtime(app: tauri::AppHandle, process: &GameProcess, extra: &[&str]) -> Result<(), LaunchError> {
    let runtime = runtime_dir(&app).map_err(LaunchError::StartFailed)?;
    let cache = cache_dir(&app).map_err(LaunchError::StartFailed)?;
    let paths = settings_paths(&app).map_err(LaunchError::StartFailed)?;
    // The runtime reads the render resolution from render_resolution only.
    paths.migrate_render_resolution().map_err(LaunchError::StartFailed)?;
    let game = paths.load().map_err(LaunchError::StartFailed)?.game_folder.map(std::path::PathBuf::from);
    let saves = paths.saves_folder().map_err(LaunchError::SavesUnavailable)?;
    saves::prepare(&saves, &cache).map_err(|e| LaunchError::SavesUnavailable(format!("{e:?}")))?;
    let folders = LaunchFolders { runtime: &runtime, game: game.as_deref(), saves: &saves, cache: &cache };
    // The chosen graphics card, checked against the cards present (on Linux; macOS has one).
    let chosen = paths.load().map_err(LaunchError::StartFailed)?.gpu;
    let devices = if chosen.is_some() && !cfg!(target_os = "macos") {
        gpus::list_from_runtime(&runtime).ok().map(|list| list.devices)
    } else {
        None
    };
    let gpu = gpus::launch_choice(chosen.as_deref(), devices.as_deref(), gpus::prime_offload_available());
    if gpu.chosen_missing {
        let _ = app.emit(GPU_MISSING_EVENT, ());
    }
    process.launch_with(app, &folders, &gpu, extra)
}

/// The downloadable content installed in the saves folder, and whether RAAM's Shadow can be
/// played. `async`: it hashes the game's script package, which takes a moment.
#[tauri::command]
async fn dlc_state(app: tauri::AppHandle) -> Result<dlc::DlcState, String> {
    let paths = settings_paths(&app)?;
    let game = paths.load()?.game_folder.map(std::path::PathBuf::from);
    let saves = paths.saves_folder()?;
    let runtime = runtime_dir(&app)?;
    Ok(dlc::state(&runtime, game.as_deref(), &saves, runtime_current(&app)))
}

/// Whether the runtime is installed and needs no update, by the same check as Play (a project build,
/// or the launcher's kit): an older runtime may not know the one-off tasks the launcher asks of it
/// (--install_content, --make_dlc_patch, --export_achievements_from) and would start the game
/// instead. Play installs the newer one first.
fn runtime_current(app: &tauri::AppHandle) -> bool {
    runtime_status(app).is_ok_and(|status| status.installed && !status.update_available)
}

/// Installs a downloadable content package the player chose into the saves folder. Returns its
/// name.
#[tauri::command]
async fn install_dlc(app: tauri::AppHandle, package: String) -> Result<String, String> {
    let paths = settings_paths(&app)?;
    let game = paths.load()?.game_folder.map(std::path::PathBuf::from).ok_or("no game folder")?;
    let saves = paths.saves_folder()?;
    let runtime = runtime_dir(&app)?;
    if !runtime_current(&app) {
        return Err("the runtime needs its update first (Play installs it)".into());
    }
    dlc::install(&runtime, &game, &saves, std::path::Path::new(&package))
}

/// Removes an installed downloadable content of the catalogue (only the folder its package made).
#[tauri::command]
async fn remove_dlc(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let saves = settings_paths(&app)?.saves_folder()?;
    dlc::remove(&saves, &id)
}

/// Event sent when the chosen graphics card is not in the computer any more, before the game starts
/// with the automatic choice.
const GPU_MISSING_EVENT: &str = "gpu-missing";

/// Lists the graphics cards the game can run on, for the choice in the Graphics options (empty on
/// macOS). `async`: it runs the runtime, which takes a moment.
#[tauri::command]
async fn list_gpus(app: tauri::AppHandle) -> Result<gpus::GpuList, String> {
    gpus::list(&runtime_dir(&app)?)
}

/// Returns the current state of saved games.
#[tauri::command]
async fn get_save_status(app: tauri::AppHandle) -> Result<SaveStatus, String> {
    let paths = settings_paths(&app)?;
    let saves = paths.saves_folder()?;
    Ok(saves::inspect_saves(&app, &saves))
}

/// The saves folder, with a failure to find it as a saves error for the interface.
fn saves_folder(app: &tauri::AppHandle) -> Result<std::path::PathBuf, SavesError> {
    settings_paths(app).and_then(|paths| paths.saves_folder()).map_err(SavesError::Io)
}

/// Removes orphaned checkpoint headers, after backing up the saves; returns how many it removed.
/// Errors are `{"kind": "...", "detail": "..."}` (`SavesError`), worded by the interface.
#[tauri::command]
async fn repair_saves(app: tauri::AppHandle) -> Result<usize, SavesError> {
    saves::repair_saves_internal(&app, &saves_folder(&app)?)
}

/// Resets saved games so the player can start a clean campaign, backing up first.
#[tauri::command]
async fn reset_saves(app: tauri::AppHandle) -> Result<(), SavesError> {
    saves::reset_saves_internal(&app, &saves_folder(&app)?)
}

/// Creates a backup of the current saves (keeping the newest `saves::MAX_BACKUPS`).
#[tauri::command]
async fn create_save_backup(app: tauri::AppHandle) -> Result<BackupInfo, SavesError> {
    saves::create_backup_internal(&app, &saves_folder(&app)?)
}

/// Restores a chosen backup.
#[tauri::command]
async fn restore_save_backup(app: tauri::AppHandle, backup_id: String) -> Result<(), SavesError> {
    saves::restore_backup_internal(&app, &saves_folder(&app)?, &backup_id)
}

/// Opens the save games folder in the system file manager.
#[tauri::command]
async fn open_saves_folder(app: tauri::AppHandle) -> Result<(), String> {
    let paths = settings_paths(&app)?;
    let saves = paths.saves_folder()?;
    saves::open_saves_folder_internal(&saves)
}

/// Opens the file picker for a downloadable content package. Returns `None` when the user cancels.
#[tauri::command]
async fn pick_dlc_package(app: tauri::AppHandle) -> Option<String> {
    let picked = app.dialog().file().blocking_pick_file()?;
    Some(picked.into_path().ok()?.to_string_lossy().into_owned())
}

/// Opens the folder picker for a new saves folder. Returns `None` when the user cancels; nothing
/// moves until `change_saves_folder`.
#[tauri::command]
async fn pick_saves_folder(app: tauri::AppHandle) -> Option<String> {
    let picked = app.dialog().file().blocking_pick_folder()?;
    Some(picked.into_path().ok()?.to_string_lossy().into_owned())
}

/// Moves the saves to `folder` and makes it the saves folder. With saves already in `folder` it
/// fails with `destinationHasSaves` unless `replace` is set (the interface asks first).
#[tauri::command]
async fn change_saves_folder(
    app: tauri::AppHandle,
    process: tauri::State<'_, GameProcess>,
    folder: String,
    replace: bool,
) -> Result<String, SavesError> {
    if process.is_running() {
        return Err(SavesError::Io("the game is running".into()));
    }
    let paths = settings_paths(&app).map_err(SavesError::Io)?;
    let current = paths.saves_folder().map_err(SavesError::Io)?;
    let target = std::path::PathBuf::from(&folder);
    // Saves still in the runtime's old folder count as the current ones.
    let source = if saves::has_saves(&current) { current } else { saves::legacy_folder() };
    // Backups first (the saves that leave, and those a replace overwrites): no backup, no move.
    saves::backup_before_move(&app, &source, &target, replace)?;
    saves::move_saves(&source, &target, replace)?;
    std::fs::create_dir_all(&target).map_err(|e| SavesError::Io(e.to_string()))?;
    paths.set_saves_folder(&target).map_err(SavesError::Io)?;
    Ok(folder)
}

/// Loads the launcher and runtime settings (see `settings.rs`).
#[tauri::command]
async fn load_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    settings_paths(&app)?.load()
}

/// Saves the launcher and runtime settings, keeping whatever else `azura.toml` holds.
#[tauri::command]
async fn save_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    settings_paths(&app)?.save(&settings)
}

/// The installation's identity in the saves folder (see `alboria.rs`), made when the game has not
/// made it yet.
fn alboria_identity(app: &tauri::AppHandle) -> Result<Identity, AlboriaError> {
    let folder = settings_paths(app)
        .and_then(|paths| paths.saves_folder())
        .map_err(|e| AlboriaError::local_error("identity", e))?;
    alboria::load_or_create_identity(&folder).map_err(|e| AlboriaError::local_error("identity", e))
}

/// The gamertag the launcher keeps (`user_name`), which the account follows when it signs in.
fn alboria_gamertag(app: &tauri::AppHandle) -> Result<String, AlboriaError> {
    let settings = settings_paths(app)
        .and_then(|paths| paths.load())
        .map_err(|e| AlboriaError::local_error("settings", e))?;
    Ok(settings.user_name.map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).unwrap_or_else(|| "User".into()))
}

/// Signs this installation in to its Alboria account with `gamertag`, which the account takes on.
/// `no_account` (403) while no account has linked it.
#[tauri::command]
async fn alboria_sign_in(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    gamertag: String,
) -> Result<Session, AlboriaError> {
    let identity = alboria_identity(&app)?;
    alboria.sign_in(&identity, gamertag.trim()).await
}

/// Asks for a code sent to `email`: to create an account with `gamertag` ("create") or to link this
/// installation to the account of the email ("signin"). `language` is the interface's.
#[tauri::command]
async fn alboria_request_code(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    purpose: String,
    email: String,
    gamertag: Option<String>,
    language: String,
) -> Result<CodeSent, AlboriaError> {
    let identity = alboria_identity(&app)?;
    alboria.request_code(&identity, &purpose, &email, gamertag.as_deref().map(str::trim), &language).await
}

/// Uses the code: creates the account with `gamertag` or links this installation to it, and signs
/// in. No code (`None`) while the service sends none: the account is created unverified.
#[tauri::command]
async fn alboria_verify(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    purpose: String,
    email: String,
    code: Option<String>,
    gamertag: Option<String>,
    language: Option<String>,
) -> Result<Session, AlboriaError> {
    let identity = alboria_identity(&app)?;
    let code = code.as_deref().map(str::trim);
    let gamertag = gamertag.as_deref().map(str::trim);
    alboria.verify(&identity, &purpose, &email, code, gamertag, language.as_deref().unwrap_or("en")).await
}

/// How the service is (alboria.rs, Status): whether it sends codes now, maintenance, notices and
/// warnings, with the game `title_id`'s own state when given.
#[tauri::command]
async fn alboria_status(alboria: tauri::State<'_, Alboria>, title_id: Option<String>) -> Result<alboria::Status, AlboriaError> {
    alboria.status(title_id.as_deref()).await
}

/// Asks for a code to verify the email of an account made while the service sent no codes.
#[tauri::command]
async fn alboria_verify_email_code(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    language: String,
) -> Result<CodeSent, AlboriaError> {
    let body = serde_json::json!({ "language": language });
    let answer = alboria_account_request(&app, &alboria, "/v1/account/verify-email/code", body).await?;
    Ok(alboria::code_sent_from(&answer))
}

/// Verifies the account's email with `code`.
#[tauri::command]
async fn alboria_verify_email(app: tauri::AppHandle, alboria: tauri::State<'_, Alboria>, code: String) -> Result<bool, AlboriaError> {
    alboria_account_request(&app, &alboria, "/v1/account/verify-email", serde_json::json!({ "code": code.trim() })).await?;
    Ok(true)
}

/// A request of the signed-in account, signing in first when needed.
async fn alboria_account_request(
    app: &tauri::AppHandle,
    alboria: &Alboria,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, AlboriaError> {
    let identity = alboria_identity(app)?;
    let gamertag = alboria_gamertag(app)?;
    alboria.account_request(&identity, &gamertag, path, body).await
}

/// The account as the Account screen shows it: email, gamertag, tag and devices (each with
/// `current` for this one).
#[tauri::command]
async fn alboria_account(app: tauri::AppHandle, alboria: tauri::State<'_, Alboria>) -> Result<serde_json::Value, AlboriaError> {
    alboria_account_request(&app, &alboria, "/v1/account", serde_json::json!({})).await
}

/// Removes a device from the account; this one too, which signs this installation out of it.
#[tauri::command]
async fn alboria_remove_device(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    public_key: String,
) -> Result<(), AlboriaError> {
    let this_one = alboria_identity(&app)?.public_key().map_err(|e| AlboriaError::local_error("identity", e))? == public_key;
    alboria_account_request(&app, &alboria, "/v1/account/devices/remove", serde_json::json!({ "public_key": public_key })).await?;
    if this_one {
        alboria.forget_token();
    }
    Ok(())
}

/// Asks for a code sent to `email`, to move the account there.
#[tauri::command]
async fn alboria_email_code(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    email: String,
    language: String,
) -> Result<CodeSent, AlboriaError> {
    let body = serde_json::json!({ "email": email, "language": language });
    let answer = alboria_account_request(&app, &alboria, "/v1/account/email/code", body).await?;
    Ok(alboria::code_sent_from(&answer))
}

/// Moves the account to `email` with the code sent there.
#[tauri::command]
async fn alboria_change_email(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    email: String,
    code: String,
) -> Result<(), AlboriaError> {
    let body = serde_json::json!({ "email": email, "code": code.trim() });
    alboria_account_request(&app, &alboria, "/v1/account/email/verify", body).await.map(|_| ())
}

/// Saves everything Alboria keeps about the account in a JSON file the player picks; `false` when
/// they cancel. `async` so the picker does not freeze the window.
#[tauri::command]
async fn alboria_export(app: tauri::AppHandle, alboria: tauri::State<'_, Alboria>) -> Result<bool, AlboriaError> {
    let data = alboria_account_request(&app, &alboria, "/v1/account/export", serde_json::json!({})).await?;
    let Some(picked) = app
        .dialog()
        .file()
        .set_file_name("alboria-account.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = picked.into_path().map_err(|e| AlboriaError::local_error("export", e.to_string()))?;
    let text = serde_json::to_string_pretty(&data).map_err(|e| AlboriaError::local_error("export", e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| AlboriaError::local_error("export", e.to_string()))?;
    Ok(true)
}

/// Which browser waits on Alboria's website with `code` (the website shows it when the player signs
/// in there "with a launcher"), and roughly from where, before the player allows it.
#[tauri::command]
async fn alboria_web_link(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    code: String,
    language: String,
) -> Result<serde_json::Value, AlboriaError> {
    alboria_account_request(&app, &alboria, "/v1/account/web-link", serde_json::json!({ "code": code, "language": language })).await
}

/// The player's answer for the browser waiting with `code`: allowed, it is signed in to this
/// account on the website; refused, the website is told so.
#[tauri::command]
async fn alboria_web_link_answer(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    code: String,
    allow: bool,
) -> Result<(), AlboriaError> {
    alboria_account_request(&app, &alboria, "/v1/account/web-link/answer", serde_json::json!({ "code": code, "allow": allow })).await?;
    Ok(())
}

/// Asks for the code that confirms deleting the account, sent to its email.
#[tauri::command]
async fn alboria_delete_code(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    language: String,
) -> Result<CodeSent, AlboriaError> {
    let answer = alboria_account_request(&app, &alboria, "/v1/account/delete/code", serde_json::json!({ "language": language })).await?;
    Ok(alboria::code_sent_from(&answer))
}

/// Deletes the account with the code (none while the service sends no codes: this device's key is
/// the proof); every device, this one too, is signed out.
#[tauri::command]
async fn alboria_delete(
    app: tauri::AppHandle,
    alboria: tauri::State<'_, Alboria>,
    code: Option<String>,
) -> Result<(), AlboriaError> {
    let body = match code {
        Some(code) => serde_json::json!({ "code": code.trim() }),
        None => serde_json::json!({}),
    };
    alboria_account_request(&app, &alboria, "/v1/account/delete", body).await?;
    alboria.forget_token();
    Ok(())
}

/// Alboria's website, which the launcher's Alboria buttons open.
const ALBORIA_SITE: &str = "https://alboria.org";

/// Opens Alboria's website in the system browser. The address is fixed here: the interface can open
/// nothing else.
#[tauri::command]
fn open_alboria_site(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(ALBORIA_SITE, None::<&str>).map_err(|e| e.to_string())
}

/// The languages of Alboria's website, each with its own blog at alboria.org/<language>/blog/.
const SITE_LANGUAGES: [&str; 13] = ["en", "es", "fr", "de", "it", "pt", "pl", "cs", "hu", "ru", "ja", "ko", "zh-Hant"];

/// The address of Alboria's blog in `language`, or in English when the website has no such language.
fn alboria_blog_url(language: &str) -> String {
    let language = SITE_LANGUAGES.iter().find(|code| **code == language).copied().unwrap_or("en");
    format!("{ALBORIA_SITE}/{language}/blog/")
}

/// Opens Alboria's blog in the system browser, in the launcher's language. Like the site, the
/// address is built here: the interface only says the language.
#[tauri::command]
fn open_alboria_blog(app: tauri::AppHandle, language: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(alboria_blog_url(&language), None::<&str>).map_err(|e| e.to_string())
}

/// Alboria's official Discord server. The website redirects this address to the server's current
/// invite, so a new invite needs no launcher release.
const ALBORIA_DISCORD: &str = "https://alboria.org/discord";

/// Opens Alboria's official Discord server in the system browser. The address is fixed here: the
/// interface can open nothing else.
#[tauri::command]
fn open_alboria_discord(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(ALBORIA_DISCORD, None::<&str>).map_err(|e| e.to_string())
}

/// Opens a link of an Alboria notice in the system's browser (or mail app). Only `https:` and
/// `mailto:` links open: a notice is written by people, and nothing else may leave the launcher.
#[tauri::command]
fn open_notice_link(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if !notice_link_allowed(&url) {
        return Err(format!("not an https or mailto link: {url}"));
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Whether a notice's link may open: `https:` or `mailto:`, in any case, with something after it.
fn notice_link_allowed(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["https://", "mailto:"].iter().any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
}

/// Whether there is a working C++ compiler to build the runtime with, and how to install one
/// otherwise (see `toolchain.rs`). `async`: it compiles a small test program.
#[tauri::command]
async fn get_compiler_status() -> CompilerStatus {
    toolchain::status()
}

/// Installs the compiler: opens Apple's Command Line Tools dialog on macOS (the interface then
/// checks the status until they are there), or runs the package manager on Linux and waits.
#[tauri::command]
async fn install_compiler() -> Result<(), String> {
    toolchain::install()
}

/// Describes this Mac (chip, memory), for the status lines and the "Auto" preset.
#[tauri::command]
async fn get_system_info() -> SystemInfo {
    system::system_info()
}

/// The last diagnostic file written, the only one `show_diagnostic` shows.
#[derive(Default)]
struct LastDiagnostic(std::sync::Mutex<Option<std::path::PathBuf>>);

/// What the diagnostic file is made from (see `diagnostics.rs`): the launcher's folders, the system
/// and the private values to clean out of it.
async fn diagnostic_sources(app: &tauri::AppHandle) -> Result<diagnostics::Sources, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let paths = settings_paths(app)?;
    let settings = paths.load().ok();
    let (home, users) = diagnostics::home_and_users();
    let crash_report_dirs = if cfg!(target_os = "macos") {
        let mut dirs = vec![std::path::PathBuf::from("/Library/Logs/DiagnosticReports")];
        if let Some(home) = &home {
            dirs.insert(0, home.join("Library").join("Logs").join("DiagnosticReports"));
        }
        dirs
    } else {
        Vec::new()
    };
    let runtime = runtime_dir(app)?;
    let gpus = tauri::async_runtime::spawn_blocking(move || gpus::list(&runtime))
        .await
        .map_err(|e| e.to_string())?
        .map(|list| list.devices.into_iter().map(|gpu| (gpu.name, gpu.kind)).collect())
        .unwrap_or_default();
    let private = diagnostics::Private {
        home,
        users,
        gamertags: settings.as_ref().and_then(|settings| settings.user_name.clone()).into_iter().collect(),
        game_folder: settings.as_ref().and_then(|settings| settings.game_folder.clone()).map(std::path::PathBuf::from),
        saves_folder: paths.saves_folder().ok(),
        hosts: diagnostics::host_names(),
    };
    Ok(diagnostics::Sources {
        data_dir,
        kit_dir: builder::find_kit(app.path().resource_dir().ok().as_deref()),
        launcher_version: app.package_info().version.to_string(),
        system: system::system_info(),
        gpus,
        compiler: toolchain::compiler().map(|compiler| compiler.version),
        crash_report_dirs,
        private,
    })
}

/// The files a diagnostic file would hold, for the interface to list before it is created.
#[tauri::command]
async fn diagnostic_plan(app: tauri::AppHandle) -> Result<Vec<diagnostics::PlanEntry>, String> {
    let sources = diagnostic_sources(&app).await?;
    tauri::async_runtime::spawn_blocking(move || diagnostics::plan(&sources)).await.map_err(|e| e.to_string())
}

/// Asks where to save the diagnostic file and writes it there, without the entries in `left_out`;
/// `game_output` is the game output the launcher showed. `None` when the player cancels.
#[tauri::command]
async fn create_diagnostic(
    app: tauri::AppHandle,
    left_out: Vec<String>,
    game_output: Vec<String>,
) -> Result<Option<diagnostics::Created>, diagnostics::DiagnosticError> {
    let sources = diagnostic_sources(&app).await.map_err(diagnostics::DiagnosticError::Io)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    let Some(picked) = app
        .dialog()
        .file()
        .set_file_name(format!("azura-diagnostic-{now}.zip"))
        .add_filter("Zip", &["zip"])
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let mut destination = picked.into_path().map_err(|e| diagnostics::DiagnosticError::Unwritable(e.to_string()))?;
    if destination.extension().is_none_or(|extension| !extension.eq_ignore_ascii_case("zip")) {
        destination.set_extension("zip");
    }
    let target = destination.clone();
    let created = tauri::async_runtime::spawn_blocking(move || diagnostics::create(&sources, &target, &left_out, &game_output))
        .await
        .map_err(|e| diagnostics::DiagnosticError::Io(e.to_string()))??;
    *app.state::<LastDiagnostic>().0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(destination);
    Ok(Some(created))
}

/// Shows the last diagnostic file written in the system's file manager.
#[tauri::command]
async fn show_diagnostic(app: tauri::AppHandle) -> Result<(), String> {
    let path = app.state::<LastDiagnostic>().0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    let path = path.filter(|path| path.is_file()).ok_or("no diagnostic file written")?;
    reveal(&path)
}

/// What this launcher version brings, for the "What's new" panel (see `release_notes.rs`).
/// `None` when the changelog has no section for it.
#[tauri::command]
async fn get_release_notes(app: tauri::AppHandle) -> Option<release_notes::ReleaseNotes> {
    release_notes::current(&app.package_info().version.to_string())
}

/// Checks whether a newer launcher version is published (see `updater.rs`).
#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    updater::check(&app).await
}

/// Installs the newer launcher version and restarts into it. Restarting would leave the game
/// without its launcher, so it waits until the game is closed.
#[tauri::command]
async fn install_update(app: tauri::AppHandle, process: tauri::State<'_, GameProcess>) -> Result<(), String> {
    if process.is_running() {
        return Err("the game is running: close it to update the launcher".into());
    }
    updater::install(app).await
}

/// Builds and runs the Tauri application. Blocks until the last window closes.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .manage(GameProcess::default())
        .manage(Alboria::default())
        .manage(AlboriaEvents::default())
        .manage(LastDiagnostic::default())
        .invoke_handler(tauri::generate_handler![
            pick_game_folder,
            pick_game_image,
            check_game_folder,
            load_settings,
            save_settings,
            get_system_info,
            get_release_notes,
            list_gpus,
            dlc_state,
            install_dlc,
            remove_dlc,
            pick_dlc_package,
            get_runtime_status,
            install_runtime,
            launch_game,
            prepare_shaders,
            shader_storage_bytes,
            pick_saves_folder,
            change_saves_folder,
            check_for_update,
            install_update,
            get_save_status,
            repair_saves,
            reset_saves,
            create_save_backup,
            restore_save_backup,
            open_saves_folder,
            get_achievements,
            get_compiler_status,
            install_compiler,
            show_build_log,
            diagnostic_plan,
            create_diagnostic,
            show_diagnostic,
            alboria_sign_in,
            alboria_request_code,
            alboria_verify,
            alboria_status,
            alboria_verify_email_code,
            alboria_verify_email,
            alboria_account,
            alboria_remove_device,
            alboria_email_code,
            alboria_change_email,
            alboria_export,
            alboria_delete_code,
            alboria_web_link,
            alboria_web_link_answer,
            alboria_delete,
            open_alboria_site,
            open_alboria_blog,
            open_alboria_discord,
            open_notice_link,
            join_room,
            alboria_friends,
            alboria_events_start,
            alboria_events_stop,
            alboria_server,
            alboria_server_set
        ])
        .setup(|app| {
            // The development launcher talks to the Alboria server it chose (Local by default).
            if cfg!(feature = "dev") {
                let saved = dev_server_file(app.handle()).ok().and_then(|file| std::fs::read_to_string(file).ok());
                app.state::<Alboria>().set_base(dev_server_from(saved.as_deref()).url);
            }
            // Controllers navigate the interface (gamepad.rs).
            gamepad::spawn(app.handle().clone());
            // A runtime install the launcher was closed in the middle of is finished first.
            if let Ok(dir) = runtime_dir(app.handle()) {
                builder::recover(&dir);
            }
            // Debug builds log to the terminal that started them; release builds stay quiet.
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Azura launcher");
}

#[cfg(test)]
mod notice_link_tests {
    use super::{alboria_blog_url, notice_link_allowed, ALBORIA_DISCORD, room_code_valid, FRIENDS_PATHS};

    #[test]
    fn the_development_launcher_chooses_its_server() {
        use super::{dev_server_from, server_url_valid, LOCAL_API_URL};
        assert_eq!(dev_server_from(None).url, LOCAL_API_URL);
        assert_eq!(dev_server_from(Some("local\n")).choice, "local");
        assert_eq!(dev_server_from(Some("production")).url, crate::alboria::DEFAULT_API_URL);
        let other = dev_server_from(Some("http://192.168.1.20:8080/"));
        assert_eq!((other.choice.as_str(), other.url.as_str()), ("other", "http://192.168.1.20:8080"));
        // Anything else falls back to Local.
        assert_eq!(dev_server_from(Some("ftp://x")).choice, "local");
        assert!(!server_url_valid("http://"));
        assert!(!server_url_valid("https://a b"));
    }

    #[test]
    fn only_room_codes_and_friends_paths_pass() {
        assert!(room_code_valid("K7QM-2XPA"));
        assert!(room_code_valid("K7QM2XPA"));
        assert!(!room_code_valid("K7Q"));
        assert!(!room_code_valid("K7QM 2XPA; rm"));
        assert!(!room_code_valid("--alboria_api_url=x"));
        assert!(FRIENDS_PATHS.contains(&"/v1/friends/room"));
        assert!(!FRIENDS_PATHS.contains(&"/v1/account/delete"));
    }

    #[test]
    fn only_https_and_mailto_links_open() {
        assert_eq!(alboria_blog_url("es"), "https://alboria.org/es/blog/");
        assert_eq!(alboria_blog_url("zh-Hant"), "https://alboria.org/zh-Hant/blog/");
        assert_eq!(alboria_blog_url("../evil"), "https://alboria.org/en/blog/");
        assert_eq!(ALBORIA_DISCORD, "https://alboria.org/discord");
        assert!(notice_link_allowed("https://alboria.org/estado"));
        assert!(notice_link_allowed("HTTPS://alboria.org"));
        assert!(notice_link_allowed("mailto:hola@alboria.org"));
        assert!(!notice_link_allowed("http://alboria.org"));
        assert!(!notice_link_allowed("javascript:alert(1)"));
        assert!(!notice_link_allowed("file:///etc/passwd"));
        assert!(!notice_link_allowed("https://"));
    }
}
