//! Starts the game runtime and relays its output to the interface.
//!
//! The runtime is the recompiled game executable, `azura`. It lives in the `runtime` folder of the
//! app data folder, next to the `azura.toml` the launcher writes, because the runtime reads its
//! settings from beside its own executable (`install.rs` puts it there).
//!
//! The folders for saves and for the shader cache are passed on the command line
//! (`--user_data_root`, `--cache_root`): the runtime picks them before it reads `azura.toml`, so
//! the config file cannot set them. The game folder goes on the command line too
//! (`--game_data_root`), so starting does not depend on `azura.toml` having been written.
//!
//! While the game runs, every line it prints is sent to the interface as a `game-output` event,
//! and its end as a `game-exit` event, so the interface can show a live log and explain a failed
//! start. The end is only reported once the game's last lines have been sent, so the log never
//! shows output after the game has ended.
//!
//! Some settings only apply when the game starts, and the game offers to restart when they change.
//! The runtime is started with `AZURA_LAUNCHER=1`, which tells it to leave the restart to the
//! launcher: it exits with `RESTART_EXIT_CODE`, and the launcher starts it again with the same
//! command, sending a `game-restart` event instead of `game-exit`. The game stays a child of the
//! launcher, so its output and its end keep being followed.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::gpus::LaunchChoice;

/// Name of the runtime executable inside the runtime folder.
#[cfg(not(windows))]
pub const RUNTIME_EXECUTABLE: &str = "azura";
/// Name of the runtime executable inside the runtime folder.
#[cfg(windows)]
pub const RUNTIME_EXECUTABLE: &str = "azura.exe";

/// Event carrying one line of the game's output.
pub const OUTPUT_EVENT: &str = "game-output";

/// Event sent once when the game process ends.
pub const EXIT_EVENT: &str = "game-exit";

/// Event sent when the game exits asking to be started again, right before it is.
pub const RESTART_EVENT: &str = "game-restart";

/// Environment variable that tells the runtime the launcher restarts it (`runtime/src/relaunch.h`).
pub const LAUNCHER_ENV: &str = "AZURA_LAUNCHER";

/// Exit code with which the runtime asks to be started again (`kRestartExitCode` in
/// `runtime/src/relaunch.h`; EX_TEMPFAIL in sysexits.h).
pub const RESTART_EXIT_CODE: i32 = 75;

/// One line of output; `stream` is "stdout" or "stderr".
#[derive(Debug, Clone, Serialize)]
pub struct OutputLine {
    pub stream: &'static str,
    pub line: String,
}

/// How the game ended. `code` is missing when the process was killed by a signal.
#[derive(Debug, Clone, Serialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub success: bool,
}

/// Why the game could not start. Serialized as `{"kind": "...", "detail": "..."}` so the
/// interface can show a translated message for each kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum LaunchError {
    /// The runtime executable is not in place yet.
    RuntimeMissing,
    /// A game started by this launcher is still running.
    AlreadyRunning,
    /// The saves folder could not be prepared (see `saves.rs`).
    SavesUnavailable(String),
    /// The operating system refused to start the process.
    StartFailed(String),
}

/// Tracks whether a game started by this launcher is running. Shared with the threads that
/// watch the process.
#[derive(Default)]
pub struct GameProcess {
    running: Arc<AtomicBool>,
}

/// Path of the runtime executable inside a runtime folder.
pub fn executable_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(RUNTIME_EXECUTABLE)
}

/// Folders the runtime is started with.
pub struct LaunchFolders<'a> {
    /// Holds the runtime executable and its `azura.toml`.
    pub runtime: &'a Path,
    /// The game files (`game_data_root`), when one is chosen.
    pub game: Option<&'a Path>,
    /// The player's saves (`user_data_root`).
    pub saves: &'a Path,
    /// The shader cache (`cache_root`).
    pub cache: &'a Path,
}

/// Command-line arguments that point the runtime at the saves and cache folders, and at the chosen
/// graphics card (see `gpus.rs`).
pub fn launch_arguments(folders: &LaunchFolders, gpu: &LaunchChoice) -> Vec<String> {
    let mut args = folder_arguments(folders);
    // RAAM's Shadow, when it is installed and can be played (see dlc.rs).
    args.extend(crate::dlc::launch_arguments(folders.runtime, folders.game, folders.saves));
    if let Some(uuid) = &gpu.device_uuid {
        args.push(format!("--vulkan_device_uuid={uuid}"));
    }
    args
}

/// Command-line arguments that point the runtime at the saves and cache folders.
pub fn folder_arguments(folders: &LaunchFolders) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(game) = folders.game {
        args.push(format!("--game_data_root={}", game.display()));
    }
    args.push(format!("--user_data_root={}", folders.saves.display()));
    args.push(format!("--cache_root={}", folders.cache.display()));
    args
}

/// Whether a game that ended with `exit` asks to be started again.
pub fn asks_for_restart(exit: &ExitInfo) -> bool {
    exit.code == Some(RESTART_EXIT_CODE)
}

/// Where the game's output, restarts and end go: the interface (as events) in the app, a recorder
/// in the tests.
pub trait GameEvents: Clone + Send + 'static {
    fn output(&self, line: OutputLine);
    fn restart(&self);
    fn exit(&self, exit: ExitInfo);
}

impl GameEvents for AppHandle {
    fn output(&self, line: OutputLine) {
        let _ = self.emit(OUTPUT_EVENT, line);
    }

    fn restart(&self) {
        let _ = self.emit(RESTART_EVENT, ());
    }

    fn exit(&self, exit: ExitInfo) {
        let _ = self.emit(EXIT_EVENT, exit);
        // The launcher minimized itself for the game (launch_game in lib.rs): it comes back in front.
        use tauri::Manager;
        if let Some(window) = self.get_webview_window("main") {
            if window.is_minimized().unwrap_or(false) {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
    }
}

/// Reads `source` line by line on its own thread and sends each line to `events`.
/// When `source` ends (the game closed it, normally by exiting), sends on `done`.
fn relay_lines<E: GameEvents>(
    events: E,
    stream: &'static str,
    source: impl Read + Send + 'static,
    done: mpsc::Sender<()>,
) {
    thread::spawn(move || {
        for line in BufReader::new(source).lines().map_while(Result::ok) {
            events.output(OutputLine { stream, line });
        }
        let _ = done.send(());
    });
}

/// Longest wait, once the game has exited, for its last output lines to be sent. Its output ends
/// as soon as it exits, unless a process it started keeps the output open; the end is then
/// reported anyway after this delay.
#[cfg(not(test))]
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Tests wait much longer: how late a busy machine runs the output relays must not decide whether
/// a test passes, and a wait that is not needed ends as soon as the relays do.
#[cfg(test)]
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(60);

/// A started game: its process and the threads relaying its output.
struct RunningGame {
    child: Child,
    /// Receives one message per output relay that has finished.
    relays_done: mpsc::Receiver<()>,
    relays: usize,
}

impl RunningGame {
    /// Waits for the game to exit, then for its output relays to send its last lines (up to
    /// OUTPUT_DRAIN_TIMEOUT), and returns how it ended.
    fn wait(&mut self) -> ExitInfo {
        let exit = exit_info(&mut self.child);
        let deadline = Instant::now() + OUTPUT_DRAIN_TIMEOUT;
        for _ in 0..self.relays {
            let left = deadline.saturating_duration_since(Instant::now());
            if self.relays_done.recv_timeout(left).is_err() {
                break;
            }
        }
        exit
    }
}

/// Starts the runtime with `args` from `runtime_dir`, relaying its output. `prime_offload` runs it
/// on the NVIDIA card of a Linux laptop with hybrid graphics (see `gpus.rs`).
fn spawn_game<E: GameEvents>(
    events: &E,
    executable: &Path,
    args: &[String],
    runtime_dir: &Path,
    prime_offload: bool,
) -> std::io::Result<RunningGame> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(runtime_dir)
        .env(LAUNCHER_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // On Linux laptops with an NVIDIA GPU next to an integrated one, PRIME render offload runs the
    // game on the NVIDIA one (and hides the others from it).
    crate::gpus::set_prime_offload(&mut command, prime_offload);
    // The game loads its own libraries and the system's, never the AppImage's (see host_env.rs).
    crate::host_env::use_host_env(&mut command);
    let mut child = command.spawn()?;
    let (done, relays_done) = mpsc::channel();
    let mut relays = 0;
    if let Some(stdout) = child.stdout.take() {
        relay_lines(events.clone(), "stdout", stdout, done.clone());
        relays += 1;
    }
    if let Some(stderr) = child.stderr.take() {
        relay_lines(events.clone(), "stderr", stderr, done);
        relays += 1;
    }
    Ok(RunningGame { child, relays_done, relays })
}

/// How a process waited on with `child.wait()` ended.
fn exit_info(child: &mut Child) -> ExitInfo {
    match child.wait() {
        Ok(status) => ExitInfo { code: status.code(), success: status.success() },
        Err(_) => ExitInfo { code: None, success: false },
    }
}

/// Waits for the started `game` to end, and starts it again with the same command for as long as
/// it asks to be. Returns how the last run ended.
fn watch_game<E: GameEvents>(
    events: &E,
    mut game: RunningGame,
    executable: &Path,
    args: &[String],
    runtime_dir: &Path,
    prime_offload: bool,
) -> ExitInfo {
    loop {
        let exit = game.wait();
        if !asks_for_restart(&exit) {
            return exit;
        }
        events.restart();
        match spawn_game(events, executable, args, runtime_dir, prime_offload) {
            Ok(next) => game = next,
            Err(error) => {
                let line = format!("Cannot restart the game: {error}");
                events.output(OutputLine { stream: "stderr", line });
                return ExitInfo { code: None, success: false };
            }
        }
    }
}

impl GameProcess {
    /// Whether a game started by this launcher is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Starts the runtime from its folder, so it finds `azura.toml` and writes its logs there, on
    /// the graphics card of `gpu`. Returns once the process has started; output, restarts and exit
    /// arrive later as events. `extra` arguments go after the usual ones (Prepare shaders runs the
    /// runtime with `shader_precompile_only`).
    pub fn launch_with<E: GameEvents>(
        &self,
        events: E,
        folders: &LaunchFolders,
        gpu: &LaunchChoice,
        extra: &[&str],
    ) -> Result<(), LaunchError> {
        let runtime_dir = folders.runtime;
        let executable = executable_path(runtime_dir);
        if !executable.is_file() {
            return Err(LaunchError::RuntimeMissing);
        }
        // Claim the slot atomically, so two quick clicks cannot start two games.
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(LaunchError::AlreadyRunning);
        }

        let mut args = launch_arguments(folders, gpu);
        args.extend(extra.iter().map(|arg| arg.to_string()));
        let prime_offload = gpu.prime_offload;
        let runtime_dir = runtime_dir.to_path_buf();
        let game = match spawn_game(&events, &executable, &args, &runtime_dir, prime_offload) {
            Ok(game) => game,
            Err(error) => {
                self.running.store(false, Ordering::SeqCst);
                return Err(LaunchError::StartFailed(error.to_string()));
            }
        };

        // The slot stays claimed while the game restarts, so Play cannot start a second game.
        let running = Arc::clone(&self.running);
        thread::spawn(move || {
            let exit = watch_game(&events, game, &executable, &args, &runtime_dir, prime_offload);
            running.store(false, Ordering::SeqCst);
            events.exit(exit);
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    /// The game's own choice of graphics card, without PRIME render offload.
    fn automatic_gpu() -> LaunchChoice {
        LaunchChoice { device_uuid: None, prime_offload: false, chosen_missing: false }
    }

    #[test]
    fn passes_the_chosen_graphics_card() {
        let runtime = TempFolder::new("runtime-gpu-args");
        let folders = LaunchFolders {
            runtime: &runtime,
            game: None,
            saves: Path::new("/saves"),
            cache: Path::new("/cache"),
        };
        let chosen = LaunchChoice {
            device_uuid: Some("0123456789abcdef0123456789abcdef".into()),
            prime_offload: false,
            chosen_missing: false,
        };
        assert_eq!(
            launch_arguments(&folders, &chosen),
            [
                "--user_data_root=/saves",
                "--cache_root=/cache",
                "--vulkan_device_uuid=0123456789abcdef0123456789abcdef"
            ]
        );
        assert_eq!(launch_arguments(&folders, &automatic_gpu()), folder_arguments(&folders));
    }

    #[test]
    fn passes_the_saves_and_cache_folders() {
        let runtime = TempFolder::new("runtime-args");
        let folders = LaunchFolders {
            runtime: &runtime,
            game: Some(Path::new("/Games/Gears of War 3")),
            saves: Path::new("/Users/me/Documents/Games Saves/Gears3-Azura"),
            cache: Path::new("/tmp/cache"),
        };
        assert_eq!(
            folder_arguments(&folders),
            [
                "--game_data_root=/Games/Gears of War 3",
                "--user_data_root=/Users/me/Documents/Games Saves/Gears3-Azura",
                "--cache_root=/tmp/cache"
            ]
        );
    }

    /// Everything a launch sent, in order. `line_delay` slows down the handling of each output
    /// line, like a busy interface, so the output relays fall behind the game's exit.
    #[derive(Clone, Default)]
    struct Recorder(Arc<std::sync::Mutex<Vec<String>>>, Duration);

    impl GameEvents for Recorder {
        fn output(&self, line: OutputLine) {
            thread::sleep(self.1);
            self.0.lock().unwrap().push(format!("{}: {}", line.stream, line.line));
        }

        fn restart(&self) {
            self.0.lock().unwrap().push("restart".into());
        }

        fn exit(&self, exit: ExitInfo) {
            self.0.lock().unwrap().push(format!("exit {:?}", exit.code));
        }
    }

    impl Recorder {
        /// Waits up to five seconds for the `exit` event, then returns everything recorded.
        fn until_exit(&self) -> Vec<String> {
            for _ in 0..500 {
                if self.0.lock().unwrap().iter().any(|event| event.starts_with("exit")) {
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(10));
            }
            // Keep listening a little: an output line arriving after the exit event would be
            // recorded after it, which the tests check against.
            thread::sleep(std::time::Duration::from_millis(100));
            self.0.lock().unwrap().clone()
        }
    }

    /// Writes a stand-in runtime: a shell script run as `azura` from `runtime`.
    #[cfg(unix)]
    fn fake_runtime(runtime: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let executable = executable_path(runtime);
        std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn starts_the_game_again_while_it_asks_for_a_restart() {
        let runtime = TempFolder::new("runtime-restart");
        // The first run asks for a restart, the second one ends normally. Each prints the variable
        // that tells it the launcher restarts it.
        fake_runtime(
            &runtime,
            &format!(
                "if [ -f started ]; then echo \"second $AZURA_LAUNCHER\"; exit 0; fi\n\
                 touch started; echo \"first $AZURA_LAUNCHER\"; exit {RESTART_EXIT_CODE}"
            ),
        );
        let folders =
            LaunchFolders { runtime: &runtime, game: None, saves: &runtime, cache: &runtime };
        let process = GameProcess::default();
        let recorder = Recorder::default();
        process.launch_with(recorder.clone(), &folders, &automatic_gpu(), &[]).unwrap();

        let events = recorder.until_exit();
        let position = |event: &str| events.iter().position(|e| e == event);
        assert_eq!(events.iter().filter(|e| *e == "restart").count(), 1, "{events:?}");
        assert_eq!(events.iter().filter(|e| e.starts_with("exit")).count(), 1, "{events:?}");
        assert_eq!(events.last().map(String::as_str), Some("exit Some(0)"), "{events:?}");
        assert!(position("stdout: first 1") < position("restart"), "{events:?}");
        assert!(position("restart") < position("stdout: second 1"), "{events:?}");
        assert!(!process.is_running());
    }

    #[cfg(unix)]
    #[test]
    fn reports_the_end_after_the_last_output_line() {
        let runtime = TempFolder::new("runtime-drain");
        // The game prints its lines and exits at once, while each line takes 2 ms to handle: the
        // output relay is still sending them when the process is already gone.
        fake_runtime(
            &runtime,
            "i=1; while [ $i -le 50 ]; do echo \"line $i\"; i=$((i+1)); done; exit 0",
        );
        let folders =
            LaunchFolders { runtime: &runtime, game: None, saves: &runtime, cache: &runtime };
        let process = GameProcess::default();
        let recorder = Recorder(Default::default(), Duration::from_millis(2));
        process.launch_with(recorder.clone(), &folders, &automatic_gpu(), &[]).unwrap();

        let events = recorder.until_exit();
        assert_eq!(events.len(), 51, "every line and then the exit: {events:?}");
        assert_eq!(events[49], "stdout: line 50");
        assert_eq!(events.last().map(String::as_str), Some("exit Some(0)"));
    }

    #[cfg(unix)]
    #[test]
    fn reports_any_other_exit_without_restarting() {
        let runtime = TempFolder::new("runtime-exit");
        fake_runtime(&runtime, "echo crashed >&2; exit 3");
        let folders =
            LaunchFolders { runtime: &runtime, game: None, saves: &runtime, cache: &runtime };
        let process = GameProcess::default();
        let recorder = Recorder::default();
        process.launch_with(recorder.clone(), &folders, &automatic_gpu(), &[]).unwrap();

        let events = recorder.until_exit();
        assert!(!events.contains(&"restart".to_string()), "{events:?}");
        assert!(events.contains(&"stderr: crashed".to_string()), "{events:?}");
        assert!(events.contains(&"exit Some(3)".to_string()), "{events:?}");
        assert!(!process.is_running());
    }

    #[test]
    fn only_the_restart_exit_code_asks_for_a_restart() {
        let exit = |code| ExitInfo { code, success: code == Some(0) };
        assert!(asks_for_restart(&exit(Some(RESTART_EXIT_CODE))));
        assert!(!asks_for_restart(&exit(Some(0))));
        assert!(!asks_for_restart(&exit(Some(1))));
        assert!(!asks_for_restart(&exit(None)));
    }

    #[test]
    fn launch_errors_serialize_with_their_kind() {
        let json = |error: LaunchError| serde_json::to_string(&error).unwrap();
        assert_eq!(json(LaunchError::RuntimeMissing), r#"{"kind":"runtimeMissing"}"#);
        assert_eq!(
            json(LaunchError::StartFailed("denied".into())),
            r#"{"kind":"startFailed","detail":"denied"}"#
        );
    }
}
