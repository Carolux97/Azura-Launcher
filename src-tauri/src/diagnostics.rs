//! The diagnostic file: one .zip a player attaches to a problem report, with what we need to
//! understand the problem and nothing that identifies the player.
//!
//! Two rules keep private data out:
//! - **A whitelist of what goes in.** Only the entries `plan` lists can enter the zip: a summary,
//!   the system, the settings keys of `RUNTIME_KEYS` and `LAUNCHER_KEYS`, the newest game logs, the
//!   build log, the game output the launcher showed and, when the player ticks them, the system's
//!   crash reports. Saves, achievements, the shader cache, the game files and the generated code
//!   are never read.
//! - **Every text is cleaned before it is written** (`Redactor`): the values the launcher knows
//!   are private (home folder, user name, gamertag, game and saves folders, machine name) and
//!   anything shaped like an address, an account, a room code or a key are replaced by markers
//!   such as `<home>`. Nothing uncleaned is written to disk: the zip is built from cleaned text.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use regex::{Captures, Regex, RegexBuilder};
use serde::Serialize;

use crate::system::SystemInfo;

/// How many of the newest game logs go in.
const GAME_LOGS: usize = 3;
/// The most of one game log that goes in: its start (the start-up, what the game found) and its
/// end (where a problem usually is).
const GAME_LOG_CAP: u64 = 4 * 1024 * 1024;
const LOG_HEAD: u64 = 256 * 1024;
/// The most of the build log and of each crash report.
const BUILD_LOG_CAP: u64 = 2 * 1024 * 1024;
const CRASH_REPORT_CAP: u64 = 2 * 1024 * 1024;
/// How many crash reports of the system go in.
const CRASH_REPORTS: usize = 2;
/// How many of the newest game log's error lines the summary repeats.
const SUMMARY_ERROR_LINES: usize = 15;
/// How long a system command that reads crash reports may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// The `azura.toml` keys that go in: graphics, performance, input, language and how the game
/// plays online. Identity, folders, addresses and join codes are not in it.
const RUNTIME_KEYS: &[&str] = &[
    "gpu_plugin", "gpu_backend", "fullscreen", "render_resolution", "upscaling_preset",
    "upscaling_sharpness", "present_effect", "swap_post_effect", "anisotropic_override",
    "field_of_view", "film_grain", "motion_blur", "depth_of_field", "dynamic_shadows", "bloom",
    "lens_flares", "ambient_occlusion", "anti_aliasing", "frame_rate_limit", "performance_overlay",
    "performance_overlay_position", "latency_overlay", "vulkan_allow_present_mode_immediate",
    "vulkan_allow_present_mode_mailbox", "vulkan_async_skip_incomplete_frames",
    "execute_unclipped_draw_vs_on_cpu", "edram_sample_space", "ppc_interpreter_fallback",
    "texture_cache_memory_limit_soft", "texture_cache_memory_limit_hard", "draw_resolution_scale_x",
    "draw_resolution_scale_y", "user_language", "network_mode", "alboria_direct", "alboria_client",
    "alboria_live", "startup_party", "versus_online_modes", "host_migration", "online_idle_kick",
    "dlc_campaign", "audio_mute", "alboria_menu_volume", "alboria_notice_volume", "pad_custom",
    "controller_icons", "mnk_mode", "mnk_mouse", "mnk_sensitivity", "mnk_aim_sensitivity",
    "mnk_zoom_sensitivity", "mnk_run_sensitivity", "mnk_invert_y", "mnk_mouse_acceleration",
    "bind_alboria_overlay",
];

/// The `launcher.toml` keys that go in. `gpu` (a device ID) goes in only as whether one was chosen.
const LAUNCHER_KEYS: &[&str] = &[
    "resolution_auto", "ui_language", "game_language", "game_language_chosen", "achievements_view",
    "seen_release_notes", "minimize_while_playing", "account",
];

/// What the diagnostic is made from: the launcher's folders and the private values to clean.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// The launcher's data folder (`launcher.toml`, `runtime/`, `build/`).
    pub data_dir: PathBuf,
    /// The kit the game is built from, when there is one.
    pub kit_dir: Option<PathBuf>,
    pub launcher_version: String,
    /// The system as the launcher describes it (`system.rs`).
    pub system: SystemInfo,
    /// The graphics cards as the runtime lists them: (name, kind).
    pub gpus: Vec<(String, String)>,
    /// The C++ compiler found, as it names itself.
    pub compiler: Option<String>,
    /// Folders where the system keeps crash reports (macOS); empty elsewhere.
    pub crash_report_dirs: Vec<PathBuf>,
    pub private: Private,
}

/// Values that identify the player, replaced wherever they appear.
#[derive(Debug, Clone, Default)]
pub struct Private {
    pub home: Option<PathBuf>,
    /// The account's user names (the home folder's name and the system's user variables).
    pub users: Vec<String>,
    pub gamertags: Vec<String>,
    pub game_folder: Option<PathBuf>,
    pub saves_folder: Option<PathBuf>,
    /// The machine's names.
    pub hosts: Vec<String>,
}

/// One file the zip would hold, as the interface lists it before the player creates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    /// Its path in the zip; the interface sends it back to say which entries to leave out.
    pub name: String,
    /// "summary", "system", "settings", "gameLog", "buildLog", "gameOutput" or "crashReport".
    pub kind: &'static str,
    /// About how many bytes of it go in, before compression.
    pub bytes: u64,
    /// Ticked when the list opens. Crash reports are not: they say more about the machine.
    pub included: bool,
}

/// The zip that was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub path: String,
    pub bytes: u64,
    pub files: Vec<String>,
}

/// Why the zip could not be written, worded by the interface. A file that is missing is not an
/// error: the summary names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum DiagnosticError {
    /// The player left every entry out.
    NothingChosen,
    /// The disk the zip goes to is full.
    DiskFull,
    /// The chosen place cannot be written to.
    Unwritable(String),
    /// Any other failure while writing.
    Io(String),
}

impl From<io::Error> for DiagnosticError {
    fn from(error: io::Error) -> Self {
        if is_disk_full(&error) {
            DiagnosticError::DiskFull
        } else if error.kind() == io::ErrorKind::PermissionDenied {
            DiagnosticError::Unwritable(error.to_string())
        } else {
            DiagnosticError::Io(error.to_string())
        }
    }
}

impl From<zip::result::ZipError> for DiagnosticError {
    fn from(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(error) => error.into(),
            other => DiagnosticError::Io(other.to_string()),
        }
    }
}

/// Whether `error` says the disk is full (ENOSPC on Unix, ERROR_DISK_FULL or
/// ERROR_HANDLE_DISK_FULL on Windows).
fn is_disk_full(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::StorageFull
        || match error.raw_os_error() {
            Some(code) if cfg!(windows) => code == 112 || code == 39,
            Some(code) => code == 28,
            None => false,
        }
}

// ---------------------------------------------------------------------------------------------
// Cleaning
// ---------------------------------------------------------------------------------------------

/// Replaces private data in text with markers. Known values go first, longest first (a game
/// folder inside the home folder becomes `<game>`, not `<home>/...`), then the patterns.
pub struct Redactor {
    literals: Vec<(Regex, &'static str)>,
    patterns: Vec<(Regex, &'static str)>,
}

impl Redactor {
    pub fn new(private: &Private) -> Self {
        let mut values: Vec<(String, &'static str, bool)> = Vec::new();
        let mut path = |path: &Option<PathBuf>, marker: &'static str| {
            if let Some(path) = path {
                let text = path.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
                if text.len() >= 3 {
                    values.push((text, marker, false));
                }
            }
        };
        path(&private.game_folder, "<game>");
        path(&private.saves_folder, "<saves>");
        path(&private.home, "<home>");
        for name in &private.gamertags {
            let name = name.trim();
            if name.len() >= 3 && !name.eq_ignore_ascii_case("user") {
                values.push((name.to_string(), "<gamertag>", true));
            }
        }
        for host in &private.hosts {
            if host.trim().len() >= 2 {
                values.push((host.trim().to_string(), "<host>", true));
            }
        }
        for user in &private.users {
            if user.trim().len() >= 2 {
                values.push((user.trim().to_string(), "<user>", true));
            }
        }
        // Longest first, so a value that holds another is replaced whole.
        values.sort_by_key(|value| std::cmp::Reverse(value.0.len()));
        let literals = values
            .into_iter()
            .map(|(value, marker, word)| (literal_pattern(&value, word), marker))
            .collect();
        Self { literals, patterns: patterns() }
    }

    /// `text` with every private value and pattern replaced. Email addresses and device names go
    /// first, whole: once a known name inside them is replaced, they would no longer look like one.
    pub fn clean(&self, text: &str) -> String {
        let early = |marker: &str| matches!(marker, "<email>" | "<device>");
        let mut out = text.to_string();
        for (pattern, marker) in self.patterns.iter().filter(|(_, marker)| early(marker)) {
            out = replace(pattern, marker, &out);
        }
        for (pattern, marker) in &self.literals {
            if pattern.is_match(&out) {
                out = pattern.replace_all(&out, *marker).into_owned();
            }
        }
        for (pattern, marker) in self.patterns.iter().filter(|(_, marker)| !early(marker)) {
            out = replace(pattern, marker, &out);
        }
        out
    }
}

/// `text` with the matches of `pattern` replaced by `marker`, but those `keeps` leaves. A device
/// keeps what names it ("audio endpoint <device>").
fn replace(pattern: &Regex, marker: &str, text: &str) -> String {
    pattern
        .replace_all(text, |captures: &Captures| {
            let found = &captures[0];
            if keeps(marker, found) {
                found.to_string()
            } else if marker == "<device>" {
                format!("{}{marker}", &found[..found.find('\'').unwrap_or(0)])
            } else {
                marker.to_string()
            }
        })
        .into_owned()
}

/// A case-insensitive pattern for a known value. A path matches with either separator (logs
/// print Windows paths with `/` too). A name matches as a whole word only.
fn literal_pattern(value: &str, word: bool) -> Regex {
    let mut pattern = String::new();
    for c in value.chars() {
        match c {
            '/' | '\\' => pattern.push_str(r"[/\\]"),
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
    }
    if word {
        pattern = format!(r"(?:^|\b){pattern}(?:\b|$)");
    }
    RegexBuilder::new(&pattern).case_insensitive(true).build().expect("escaped literal")
}

/// The shapes of private data the launcher cannot know in advance, in the order they apply.
fn patterns() -> Vec<(Regex, &'static str)> {
    let build = |pattern: &str| RegexBuilder::new(pattern).case_insensitive(true).build().expect("valid pattern");
    vec![
        // A known gamertag's Alboria tag number ("<gamertag>#114" once the name is replaced).
        (build(r"<gamertag>#[0-9]{1,4}\b"), "<gamertag>"),
        // Accounts and email addresses.
        (build(r"[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}"), "<email>"),
        // Audio devices, which carry their owner's name ("AirPods Pro de Ana").
        (build(r"(?:audio endpoint|audio device|output device|input device) '[^'\n]*'"), "<device>"),
        // Home folders the launcher did not know (another account's, a mounted drive's).
        (build(r"(?:/Users/|/home/|/var/home/|[a-z]:[/\\]+Users[/\\]+)[^/\\\s'\x22:]+"), "<home>"),
        // Alboria tags: a name and up to four digits ("Name#114"), the player's or another one's.
        (build(r"[^\s#@'\x22/\\()\[\]{},;:<>]{1,24}#[0-9]{1,4}\b"), "<gamertag>"),
        // Network card (MAC) addresses.
        (build(r"\b[0-9a-f]{2}(?:[:-][0-9a-f]{2}){5}\b"), "<mac>"),
        // IPv6 addresses: eight groups, or fewer with "::". Kept when it holds no digit (a C++
        // name such as "dead::beef"), see `keeps`.
        (build(r"\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|(?:\b[0-9a-f]{1,4})?(?::[0-9a-f]{1,4}){0,6}::(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,6})?(?:%[a-z0-9]+)?"), "<ip>"),
        // IPv4 addresses, with each part up to 255.
        (build(r"\b(?:(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\b"), "<ip>"),
        // Alboria room and join codes ("4DTZ-VSTX").
        (RegexBuilder::new(r"\b[A-Z0-9]{4}-[A-Z0-9]{4}\b").build().expect("valid pattern"), "<room>"),
        // Keys and tokens: 32 or more characters of hexadecimal or base64 with letters and digits.
        // "/" is not one of them, so a path's folders are never taken for a key.
        (build(r"[a-z0-9+_-]{32,}={0,2}"), "<secret>"),
    ]
}

/// Whether a match of a pattern is left as it is: an IPv6 candidate with no digit or too short
/// to be an address ("::" alone, a C++ name), and a key candidate that is not a mix of letters and
/// digits (a long word).
fn keeps(marker: &str, found: &str) -> bool {
    match marker {
        "<ip>" if found.contains(':') => !found.chars().any(|c| c.is_ascii_digit()) || found.trim_matches(':').is_empty(),
        "<secret>" => {
            !(found.chars().any(|c| c.is_ascii_digit()) && found.chars().any(|c| c.is_ascii_alphabetic()))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// What goes in
// ---------------------------------------------------------------------------------------------

/// One source of the zip: where its text comes from.
#[derive(Debug, Clone)]
enum Source {
    Summary,
    System,
    RuntimeSettings(PathBuf),
    LauncherSettings(PathBuf),
    /// A log file, with how much of it goes in.
    Log(PathBuf, u64),
    GameOutput,
    /// The output of a system command that describes crashes (Linux, Windows).
    CrashCommand(Vec<String>),
}

struct Entry {
    name: String,
    kind: &'static str,
    source: Source,
    bytes: u64,
    included: bool,
}

/// What a run that started the game writes once its module is about to launch (the runtime's
/// `OnPreLaunchModule`). A run started for a one-off task (listing the graphics cards on every Play,
/// exporting the achievements, installing a DLC) exits before it, after writing its own short log.
const GAME_RUN_MARKER: &str = "[Azura] Active gamertag";

/// The newest game logs (`runtime/logs/azura_N.log`) of runs that started the game, newest first.
/// The launcher lists the graphics cards each time Play is pressed, and each listing leaves a log of
/// the Vulkan start-up: kept by date alone, those pushed the session the player needed help with
/// out of the file. A log that is empty or of a one-off task is left out; with no game run at all
/// (an older runtime, or the game never got that far), the newest logs that hold something go in.
fn game_logs(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(data_dir.join("runtime").join("logs")) else {
        return Vec::new();
    };
    let mut logs: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            name.starts_with("azura_") && name.ends_with(".log")
        })
        .filter_map(|path| {
            let metadata = fs::metadata(&path).ok()?;
            (metadata.len() > 0).then_some(())?;
            Some((metadata.modified().ok()?, path))
        })
        .collect();
    logs.sort_by_key(|log| std::cmp::Reverse(log.0));
    let logs: Vec<PathBuf> = logs.into_iter().map(|(_, path)| path).collect();
    let game_runs: Vec<PathBuf> = logs.iter().filter(|path| started_the_game(path)).take(GAME_LOGS).cloned().collect();
    if game_runs.is_empty() {
        logs.into_iter().take(GAME_LOGS).collect()
    } else {
        game_runs
    }
}

/// Whether the log's start shows the run started the game (`GAME_RUN_MARKER`).
fn started_the_game(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut head = Vec::new();
    if file.take(LOG_HEAD).read_to_end(&mut head).is_err() {
        return false;
    }
    String::from_utf8_lossy(&head).contains(GAME_RUN_MARKER)
}

/// The newest game build's folder (`build/<game build id>`), by its build log.
fn newest_build(data_dir: &Path) -> Option<PathBuf> {
    fs::read_dir(data_dir.join("build"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|dir| Some((fs::metadata(dir.join("build.log")).ok()?.modified().ok()?, dir)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, dir)| dir)
}

/// The system's crash reports of the game on macOS (`azura*.ips` or `.crash`), newest first.
fn crash_report_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut reports: Vec<(SystemTime, PathBuf)> = dirs
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("").to_ascii_lowercase();
            name.starts_with("azura") && (name.ends_with(".ips") || name.ends_with(".crash"))
        })
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    reports.sort_by_key(|report| std::cmp::Reverse(report.0));
    reports.into_iter().take(CRASH_REPORTS).map(|(_, path)| path).collect()
}

/// The command that prints the game's last crashes on this system, when it has one.
fn crash_command() -> Option<Vec<String>> {
    if cfg!(target_os = "linux") {
        Some(["coredumpctl", "info", "--no-pager", "azura"].map(String::from).to_vec())
    } else if cfg!(windows) {
        let query = "*[System[Provider[@Name='Application Error' or @Name='Windows Error Reporting']]]";
        Some(["wevtutil", "qe", "Application", &format!("/q:{query}"), "/c:30", "/rd:true", "/f:text"].map(String::from).to_vec())
    } else {
        None
    }
}

/// Every entry the zip can hold, with its source.
fn entries(sources: &Sources) -> Vec<Entry> {
    let mut entries = vec![
        Entry { name: "summary.txt".into(), kind: "summary", source: Source::Summary, bytes: 2048, included: true },
        Entry { name: "system.txt".into(), kind: "system", source: Source::System, bytes: 1024, included: true },
    ];
    let runtime_settings = sources.data_dir.join("runtime").join("azura.toml");
    if runtime_settings.is_file() {
        entries.push(Entry {
            name: "settings/azura.toml".into(),
            kind: "settings",
            source: Source::RuntimeSettings(runtime_settings),
            bytes: 2048,
            included: true,
        });
    }
    let launcher_settings = sources.data_dir.join("launcher.toml");
    if launcher_settings.is_file() {
        entries.push(Entry {
            name: "settings/launcher.toml".into(),
            kind: "settings",
            source: Source::LauncherSettings(launcher_settings),
            bytes: 512,
            included: true,
        });
    }
    for log in game_logs(&sources.data_dir) {
        let name = log.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let size = fs::metadata(&log).map(|metadata| metadata.len()).unwrap_or(0);
        entries.push(Entry {
            name: format!("logs/{name}"),
            kind: "gameLog",
            bytes: size.min(GAME_LOG_CAP),
            source: Source::Log(log, GAME_LOG_CAP),
            included: true,
        });
    }
    if let Some(build) = newest_build(&sources.data_dir) {
        let log = build.join("build.log");
        let size = fs::metadata(&log).map(|metadata| metadata.len()).unwrap_or(0);
        entries.push(Entry {
            name: "build/build.log".into(),
            kind: "buildLog",
            bytes: size.min(BUILD_LOG_CAP),
            source: Source::Log(log, BUILD_LOG_CAP),
            included: true,
        });
    }
    entries.push(Entry {
        name: "launcher/game-output.txt".into(),
        kind: "gameOutput",
        source: Source::GameOutput,
        bytes: 0,
        included: true,
    });
    for report in crash_report_files(&sources.crash_report_dirs) {
        let name = report.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let size = fs::metadata(&report).map(|metadata| metadata.len()).unwrap_or(0);
        entries.push(Entry {
            name: format!("crash/{name}"),
            kind: "crashReport",
            bytes: size.min(CRASH_REPORT_CAP),
            source: Source::Log(report, CRASH_REPORT_CAP),
            included: false,
        });
    }
    if let Some(command) = crash_command() {
        let name = if cfg!(windows) { "crash/windows-application-errors.txt" } else { "crash/coredumpctl-info.txt" };
        entries.push(Entry {
            name: name.into(),
            kind: "crashReport",
            source: Source::CrashCommand(command),
            bytes: 0,
            included: false,
        });
    }
    entries
}

/// The files the zip would hold, for the interface to list before the player creates it.
pub fn plan(sources: &Sources) -> Vec<PlanEntry> {
    entries(sources)
        .into_iter()
        .map(|entry| PlanEntry { name: entry.name, kind: entry.kind, bytes: entry.bytes, included: entry.included })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

/// A text file, whole when it is at most `cap` bytes; otherwise its first `LOG_HEAD` bytes and
/// its last ones, cut at line ends, with a line saying how much was left out. A log the game is
/// still writing is read as far as it goes.
fn read_capped(path: &Path, cap: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size <= cap {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let head_len = LOG_HEAD.min(cap / 2);
    let mut head = vec![0; head_len as usize];
    file.read_exact(&mut head)?;
    let tail_len = cap - head_len;
    file.seek(SeekFrom::Start(size - tail_len))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let head = String::from_utf8_lossy(&head);
    let tail = String::from_utf8_lossy(&tail);
    let head = head.rfind('\n').map_or(&head[..], |end| &head[..=end]);
    let tail = tail.find('\n').map_or(&tail[..], |start| &tail[start + 1..]);
    let left_out = size - head.len() as u64 - tail.len() as u64;
    Ok(format!("{head}\n[... {left_out} bytes of this file left out ...]\n\n{tail}"))
}

/// The output of `command`, or `None` when it cannot run, fails or takes longer than
/// `COMMAND_TIMEOUT`.
fn command_output(command: &[String]) -> Option<String> {
    let (program, args) = command.split_first()?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout).take(CRASH_REPORT_CAP).read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if started.elapsed() > COMMAND_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    let text = String::from_utf8_lossy(&reader.join().ok()?).into_owned();
    let text = if cfg!(windows) { windows_game_errors(&text) } else { text };
    (!text.trim().is_empty()).then_some(text)
}

/// The events of the Windows Application log that are about the game, from `wevtutil`'s text.
fn windows_game_errors(text: &str) -> String {
    text.split("Event[")
        .filter(|event| event.to_ascii_lowercase().contains("azura.exe"))
        .take(CRASH_REPORTS + 1)
        .map(|event| format!("Event[{event}"))
        .collect()
}

/// The `keys` of a TOML file, one `key = value` per line, in the order of `keys`. Other keys are
/// not read; values are cleaned like any text.
fn settings_text(path: &Path, keys: &[&str], extra: impl Fn(&toml::Table) -> Vec<String>) -> io::Result<String> {
    let text = fs::read_to_string(path)?;
    let table: toml::Table = text.parse().unwrap_or_default();
    let mut out = String::new();
    for key in keys {
        if let Some(value) = table.get(*key) {
            out.push_str(&format!("{key} = {value}\n"));
        }
    }
    for line in extra(&table) {
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// The lines of a log that report a problem: those logged as errors, and those that say the game
/// crashed or stopped. Warnings are left out: a run prints many that are expected.
fn error_lines(log: &str) -> Vec<&str> {
    const LEVELS: [&str; 3] = ["] [error] ", "] [critical] ", "] [fatal] "];
    const WORDS: [&str; 5] = ["access violation", "panicked", "title terminated", "unhandled exception", "segmentation fault"];
    log.lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            LEVELS.iter().any(|level| lower.contains(level)) || WORDS.iter().any(|word| lower.contains(word))
        })
        .collect()
}

/// A time as "2026-11-08 18:04 UTC".
fn format_time(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0) as i64;
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", rest / 3600, rest % 3600 / 60)
}

/// The system's description.
fn system_text(sources: &Sources) -> String {
    let system = &sources.system;
    let mut out = format!(
        "Operating system: {} ({})\nOS version: {}\nProcessor: {}\nMemory: {}\n",
        system.os,
        system.arch,
        os_version().unwrap_or_else(|| "unknown".into()),
        system.chip.as_deref().unwrap_or("unknown"),
        system.memory_gb.map_or("unknown".into(), |gb| format!("{gb} GB")),
    );
    if sources.gpus.is_empty() && system.gpus.is_empty() {
        out.push_str("Graphics: the processor's\n");
    }
    for (name, kind) in &sources.gpus {
        out.push_str(&format!("Graphics: {name} ({kind})\n"));
    }
    if sources.gpus.is_empty() {
        for name in &system.gpus {
            out.push_str(&format!("Graphics: {name}\n"));
        }
    }
    out.push_str(&format!("Compiler: {}\n", sources.compiler.as_deref().unwrap_or("none found")));
    out.push_str("Graphics drivers: see the start of the newest game log.\n");
    out
}

/// The operating system's name and version.
fn os_version() -> Option<String> {
    if cfg!(target_os = "macos") {
        let output = Command::new("/usr/bin/sw_vers").arg("-productVersion").output().ok()?;
        Some(format!("macOS {}", String::from_utf8_lossy(&output.stdout).trim()))
    } else if cfg!(windows) {
        let output = Command::new("cmd").args(["/C", "ver"]).output().ok()?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let text = fs::read_to_string("/etc/os-release").ok()?;
        text.lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map(|value| value.trim_matches('"').to_string())
    }
}

/// The summary at the top of the zip: versions, the newest run and what failed in it, and what the
/// file leaves out.
fn summary_text(sources: &Sources, missing: &[String], files: &[String]) -> String {
    let mut out = String::from("Azura diagnostic file\n=====================\n\n");
    out.push_str(&format!("Created: {}\n", format_time(SystemTime::now())));
    out.push_str(&format!("Launcher: {} ({} {})\n", sources.launcher_version, sources.system.os, sources.system.arch));
    if let Some(revision) = sources.kit_dir.as_ref().and_then(|kit| crate::builder::read_kit(kit).ok()).map(|kit| kit.revision) {
        out.push_str(&format!("Kit revision: {revision}\n"));
    }
    if let Some(build) = newest_build(&sources.data_dir) {
        let id = build.file_name().unwrap_or_default().to_string_lossy().into_owned();
        out.push_str(&format!("Game build: {id}\n"));
        if let Some(state) = crate::builder::read_state(&build) {
            out.push_str(&format!("Built by kit {} with {}\n", state.kit_revision, state.compiler));
        }
    }
    if let Some(log) = game_logs(&sources.data_dir).into_iter().next() {
        let modified = fs::metadata(&log).and_then(|metadata| metadata.modified()).map(format_time).unwrap_or_default();
        let name = log.file_name().unwrap_or_default().to_string_lossy().into_owned();
        out.push_str(&format!("\nNewest game run: {name}, last written {modified}\n"));
        if let Ok(text) = read_capped(&log, GAME_LOG_CAP) {
            let errors = error_lines(&text);
            if errors.is_empty() {
                out.push_str("No error lines in it.\n");
            } else {
                out.push_str("Its last error lines:\n");
                for line in &errors[errors.len().saturating_sub(SUMMARY_ERROR_LINES)..] {
                    out.push_str(&format!("  {line}\n"));
                }
            }
        }
    } else {
        out.push_str("\nNo game run found.\n");
    }
    out.push_str("\nFiles in it:\n");
    for file in files {
        out.push_str(&format!("  {file}\n"));
    }
    if !missing.is_empty() {
        out.push_str("\nNot found or not readable:\n");
        for file in missing {
            out.push_str(&format!("  {file}\n"));
        }
    }
    out.push_str(
        "\nLeft out: saves, achievements, the shader cache, the game's files and the generated code.\n\
         Private data is replaced by <home>, <user>, <gamertag>, <game>, <saves>, <host>, <ip>, <mac>,\n\
         <email>, <room>, <device> and <secret>.\n",
    );
    out
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// Writes the zip to `destination` with every planned entry but those in `left_out`, the game
/// output the launcher showed in `game_output`. It is written beside `destination` first and
/// moved into place when complete: a failure leaves no zip, whole or partial.
pub fn create(
    sources: &Sources,
    destination: &Path,
    left_out: &[String],
    game_output: &[String],
) -> Result<Created, DiagnosticError> {
    let chosen: Vec<Entry> = entries(sources).into_iter().filter(|entry| !left_out.contains(&entry.name)).collect();
    if chosen.is_empty() {
        return Err(DiagnosticError::NothingChosen);
    }
    let redactor = Redactor::new(&sources.private);

    // Read and clean everything first: a missing file is named in the summary, not an error.
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut has_summary = false;
    for entry in &chosen {
        let text = match &entry.source {
            Source::Summary => {
                has_summary = true;
                continue;
            }
            Source::System => Ok(system_text(sources)),
            Source::RuntimeSettings(path) => settings_text(path, RUNTIME_KEYS, |_| Vec::new()),
            Source::LauncherSettings(path) => settings_text(path, LAUNCHER_KEYS, |table| {
                vec![format!("gpu = {}", if table.contains_key("gpu") { "\"chosen\"" } else { "\"automatic\"" })]
            }),
            Source::Log(path, cap) => read_capped(path, *cap),
            Source::GameOutput => {
                if game_output.is_empty() {
                    missing.push(format!("{} (no game output this session)", entry.name));
                    continue;
                }
                Ok(game_output.join("\n") + "\n")
            }
            Source::CrashCommand(command) => command_output(command).ok_or_else(|| io::Error::other("no crash found")),
        };
        match text {
            Ok(text) => texts.push((entry.name.clone(), redactor.clean(&text))),
            Err(_) => missing.push(entry.name.clone()),
        }
    }
    if has_summary {
        let files: Vec<String> = std::iter::once("summary.txt".to_string()).chain(texts.iter().map(|(name, _)| name.clone())).collect();
        texts.insert(0, ("summary.txt".into(), redactor.clean(&summary_text(sources, &missing, &files))));
    }

    let partial = destination.with_extension("zip.part");
    let written = write_zip(&partial, &texts).and_then(|()| fs::rename(&partial, destination).map_err(Into::into));
    if let Err(error) = written {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    let bytes = fs::metadata(destination).map(|metadata| metadata.len()).unwrap_or(0);
    Ok(Created {
        path: destination.to_string_lossy().into_owned(),
        bytes,
        files: texts.into_iter().map(|(name, _)| name).collect(),
    })
}

fn write_zip(path: &Path, texts: &[(String, String)]) -> Result<(), DiagnosticError> {
    let file = File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in texts {
        zip.start_file(name.as_str(), options)?;
        zip.write_all(text.as_bytes())?;
    }
    let file = zip.finish()?;
    file.sync_all()?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The machine's private values
// ---------------------------------------------------------------------------------------------

/// The home folder and the user names the system knows the player by.
pub fn home_and_users() -> (Option<PathBuf>, Vec<String>) {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let mut users: Vec<String> = ["USER", "USERNAME", "LOGNAME"].iter().filter_map(|name| std::env::var(name).ok()).collect();
    if let Some(name) = home.as_ref().and_then(|home| home.file_name()) {
        users.push(name.to_string_lossy().into_owned());
    }
    users.sort();
    users.dedup();
    (home, users)
}

/// The machine's names: its host name and, on macOS, the names shown in Sharing.
pub fn host_names() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(name) = std::env::var("COMPUTERNAME") {
        names.push(name);
    }
    if let Ok(name) = fs::read_to_string("/proc/sys/kernel/hostname") {
        names.push(name.trim().to_string());
    }
    let mut run = |program: &str, args: &[&str]| {
        if let Ok(output) = Command::new(program).args(args).stderr(Stdio::null()).output() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if output.status.success() && !name.is_empty() {
                names.push(name);
            }
        }
    };
    if cfg!(target_os = "macos") {
        run("/usr/sbin/scutil", &["--get", "ComputerName"]);
        run("/usr/sbin/scutil", &["--get", "LocalHostName"]);
    }
    if !cfg!(windows) {
        run("hostname", &[]);
    }
    // "name.local" is cleaned whole and as "name".
    let short: Vec<String> = names.iter().filter_map(|name| name.split('.').next().map(str::to_string)).collect();
    names.extend(short);
    names.retain(|name| name.len() >= 2 && name != "localhost");
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Private values that look real, on every platform the launcher runs on.
    fn private() -> Private {
        Private {
            home: Some(PathBuf::from("/Users/marisol.vega")),
            users: vec!["marisol.vega".into(), "mvega".into()],
            gamertags: vec!["NightHawk77".into()],
            game_folder: Some(PathBuf::from("/Users/marisol.vega/Games/Gears of War 3 (Spain)")),
            saves_folder: Some(PathBuf::from("/Users/marisol.vega/Documents/Games Saves/Gears3-Azura")),
            hosts: vec!["Marisols-MacBook-Pro".into(), "Marisols-MacBook-Pro.local".into()],
        }
    }

    /// Every private string of the log below; none may reach a cleaned text.
    const SECRETS: &[&str] = &[
        "marisol", "Marisol", "mvega", "NightHawk77", "Gears of War 3 (Spain)", "Games Saves",
        "MacBook", "192.168.1.37", "83.44.120.9", "fe80::1c2b:3aff:fe4d:5e6f", "2a01:4f8:c0c:1234::1",
        "a4:83:e7:12:9b:0c", "4DTZ-VSTX", "Kabosu#117", "Ortega#1", "marisol.vega@icloud.com",
        "dXNlcjpwYXNzd29yZDEyMzQ1Njc4OTBhYmNkZWY", "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        "jorge", "C:\\Users\\Jorge", "/home/deck2", "/var/home/bazzite-user", "#4821",
    ];

    const LOG: &str = "\
[info] [Azura] Application and window icons set from: /Users/marisol.vega/Library/Application Support/com.gears3azura.launcher/runtime/assets/azura-icon.png
[info] [Azura] Active gamertag NightHawk77
[info] game_data_root = /Users/marisol.vega/Games/Gears of War 3 (Spain)
[info] saves in /USERS/MARISOL.VEGA/Documents/Games Saves/Gears3-Azura/4D5308AB
[info] System Link: interface 'en0' 192.168.1.37/24, default route
[info] Alboria: signed in as NightHawk77#4821 (session for 60 minutes)
[info] Alboria: in room 4DTZ-VSTX as 83.44.120.9, hosting
[info] Alboria: friend Kabosu#117 and Ortega#1 joined from fe80::1c2b:3aff:fe4d:5e6f%en0 and 2a01:4f8:c0c:1234::1
[info] MAC a4:83:e7:12:9b:0c on Marisols-MacBook-Pro.local (Marisols-MacBook-Pro)
[info] mail marisol.vega@icloud.com, token dXNlcjpwYXNzd29yZDEyMzQ1Njc4OTBhYmNkZWY
[info] key 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
[info] another account: C:\\Users\\Jorge\\AppData and C:/Users/jorge/x, /home/deck2/.local, /var/home/bazzite-user/x
[info] mvega ran it
[info] [apu] audio endpoint 'AirPods Pro de Jorge': 6 ch, 48000 Hz
";

    #[test]
    fn no_private_string_survives_cleaning() {
        let cleaned = Redactor::new(&private()).clean(LOG);
        for secret in SECRETS {
            assert!(!cleaned.to_lowercase().contains(&secret.to_lowercase()), "{secret} survived:\n{cleaned}");
        }
    }

    #[test]
    fn cleaning_keeps_what_helps() {
        let cleaned = Redactor::new(&private()).clean(LOG);
        assert!(cleaned.contains("<home>/Library/Application Support/com.gears3azura.launcher/runtime"), "{cleaned}");
        assert!(cleaned.contains("game_data_root = <game>"), "{cleaned}");
        assert!(cleaned.contains("saves in <saves>/4D5308AB"), "{cleaned}");
        assert!(cleaned.contains("in room <room> as <ip>, hosting"), "{cleaned}");
        assert!(cleaned.contains("mail <email>, token <secret>"), "{cleaned}");
        assert!(cleaned.contains("signed in as <gamertag> (session"), "{cleaned}");
        assert!(cleaned.contains("audio endpoint <device>: 6 ch"), "{cleaned}");
        // A path's folders are not a key, however long.
        let path = Redactor::new(&private()).clean("/Applications/Azura Launcher.app/Contents/Resources/kit/obj/azura_recomp.223.cpp.o");
        assert_eq!(path, "/Applications/Azura Launcher.app/Contents/Resources/kit/obj/azura_recomp.223.cpp.o");
        // Not addresses: a time, a C++ name, a version and a shader hash stay.
        let kept = Redactor::new(&private()).clean("18:04:00.514 rex::system::Foo dead::beef v0.6.6 hash 9a2feaa0c55ad015");
        assert_eq!(kept, "18:04:00.514 rex::system::Foo dead::beef v0.6.6 hash 9a2feaa0c55ad015");
    }

    #[test]
    fn windows_paths_are_cleaned_with_either_separator() {
        let private = Private {
            home: Some(PathBuf::from(r"C:\Users\Ana Lucia")),
            users: vec!["Ana Lucia".into()],
            game_folder: Some(PathBuf::from(r"D:\Juegos\Gears 3 de Ana")),
            ..Private::default()
        };
        let cleaned = Redactor::new(&private).clean(r"C:\Users\Ana Lucia\AppData and c:/users/ana lucia/x and D:/Juegos/Gears 3 de Ana/default.xex");
        assert_eq!(cleaned, r"<home>\AppData and <home>/x and <game>/default.xex");
    }

    /// A launcher data folder with everything a player's has, private data in every text.
    fn fixture(root: &Path) -> Sources {
        let data = root.join("data");
        let logs = data.join("runtime").join("logs");
        fs::create_dir_all(&logs).unwrap();
        for n in 1..=5 {
            fs::write(logs.join(format!("azura_{n}.log")), format!("run {n}\n{LOG}")).unwrap();
            std::thread::sleep(Duration::from_millis(15));
        }
        // An empty log, and the newest runs only listed the graphics cards (each Play does): their
        // logs hold the Vulkan start-up but not the game, and are skipped.
        fs::write(logs.join("azura_6.log"), "").unwrap();
        for n in 7..=9 {
            std::thread::sleep(Duration::from_millis(15));
            fs::write(logs.join(format!("azura_{n}.log")), "[info] Vulkan: using Apple M4 (chosen automatically)\n").unwrap();
        }
        fs::write(
            data.join("runtime").join("azura.toml"),
            "user_name = \"NightHawk77\"\ngame_data_root = \"/Users/marisol.vega/Games/Gears of War 3 (Spain)\"\n\
             fullscreen = true\nnetwork_mode = \"online\"\nalboria_join_code = \"4DTZ-VSTX\"\nsystem_link_address = \"192.168.1.37\"\n",
        )
        .unwrap();
        fs::write(
            data.join("launcher.toml"),
            "ui_language = \"es\"\nsaves_folder = \"/Users/marisol.vega/Documents/Games Saves/Gears3-Azura\"\n\
             gpu = \"0123456789abcdef0123456789abcdef\"\naccount = \"alboria\"\n",
        )
        .unwrap();
        let build = data.join("build").join("spain");
        fs::create_dir_all(build.join("generated")).unwrap();
        fs::write(build.join("build.log"), format!("clang++ -I/Users/marisol.vega/x\n{LOG}")).unwrap();
        fs::write(build.join("generated").join("azura_recomp.0.cpp"), "GENERATED CODE").unwrap();
        // What must never go in, whatever it holds.
        for folder in ["achievements", "cache/shaders", "backups"] {
            fs::create_dir_all(data.join(folder)).unwrap();
            fs::write(data.join(folder).join("never.bin"), "NEVER IN THE ZIP").unwrap();
        }
        Sources {
            data_dir: data,
            launcher_version: "0.6.7".into(),
            system: SystemInfo { chip: Some("Apple M4".into()), memory_gb: Some(16), arch: "aarch64".into(), os: "macos".into(), ..SystemInfo::default() },
            gpus: Vec::new(),
            compiler: Some("Apple clang 21.0.0".into()),
            private: private(),
            ..Sources::default()
        }
    }

    fn unzip(path: &Path) -> Vec<(String, String)> {
        let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut file = archive.by_index(i).unwrap();
                let mut text = String::new();
                file.read_to_string(&mut text).unwrap();
                (file.name().to_string(), text)
            })
            .collect()
    }

    #[test]
    fn the_zip_holds_only_the_whitelist_and_no_private_string() {
        let root = crate::test_support::TempFolder::new("diagnostics");
        let sources = fixture(&root);
        let destination = root.join("azura-diagnostic.zip");
        // As the panel sends it: the system's crash reports, unticked, are left out.
        let crash_reports: Vec<String> =
            plan(&sources).into_iter().filter(|entry| entry.kind == "crashReport").map(|entry| entry.name).collect();
        let created =
            create(&sources, &destination, &crash_reports, &["Game started".into(), "Active gamertag NightHawk77".into()]).unwrap();
        let entries = unzip(&destination);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "summary.txt", "system.txt", "settings/azura.toml", "settings/launcher.toml", "logs/azura_5.log",
                "logs/azura_4.log", "logs/azura_3.log", "build/build.log", "launcher/game-output.txt"
            ],
        );
        assert_eq!(created.files, names);
        for (name, text) in &entries {
            for secret in SECRETS {
                assert!(!text.to_lowercase().contains(&secret.to_lowercase()), "{secret} in {name}:\n{text}");
            }
            assert!(!text.contains("NEVER IN THE ZIP") && !text.contains("GENERATED CODE"), "{name}");
        }
        let settings = &entries[2].1;
        assert_eq!(settings, "fullscreen = true\nnetwork_mode = \"online\"\n");
        let launcher = &entries[3].1;
        assert_eq!(launcher, "ui_language = \"es\"\naccount = \"alboria\"\ngpu = \"chosen\"\n");
        assert!(entries[0].1.contains("Newest game run: azura_5.log"), "{}", entries[0].1);
        assert!(!root.join("azura-diagnostic.zip.part").exists());
    }

    #[test]
    fn without_a_game_run_the_newest_logs_go_in() {
        let root = crate::test_support::TempFolder::new("diagnostics-no-game-run");
        let logs = root.join("runtime").join("logs");
        fs::create_dir_all(&logs).unwrap();
        for n in 1..=4 {
            fs::write(logs.join(format!("azura_{n}.log")), format!("[error] run {n} stopped before the game\n")).unwrap();
            std::thread::sleep(Duration::from_millis(15));
        }
        let names: Vec<String> =
            game_logs(&root).iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["azura_4.log", "azura_3.log", "azura_2.log"]);
    }

    #[test]
    fn left_out_entries_and_missing_output_are_named() {
        let root = crate::test_support::TempFolder::new("diagnostics-left-out");
        let sources = fixture(&root);
        let destination = root.join("out.zip");
        create(&sources, &destination, &["build/build.log".into()], &[]).unwrap();
        let entries = unzip(&destination);
        assert!(entries.iter().all(|(name, _)| name != "build/build.log" && name != "launcher/game-output.txt"));
        assert!(entries[0].1.contains("launcher/game-output.txt (no game output this session)"), "{}", entries[0].1);
    }

    #[test]
    fn nothing_chosen_writes_nothing() {
        let root = crate::test_support::TempFolder::new("diagnostics-nothing");
        let sources = fixture(&root);
        let all: Vec<String> = plan(&sources).into_iter().map(|entry| entry.name).collect();
        let destination = root.join("none.zip");
        assert_eq!(create(&sources, &destination, &all, &[]), Err(DiagnosticError::NothingChosen));
        assert!(!destination.exists());
    }

    #[test]
    fn an_unwritable_place_leaves_no_zip() {
        let root = crate::test_support::TempFolder::new("diagnostics-unwritable");
        let sources = fixture(&root);
        let destination = root.join("no-such-folder").join("out.zip");
        assert!(create(&sources, &destination, &[], &[]).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn a_long_log_keeps_its_start_and_its_end() {
        let root = crate::test_support::TempFolder::new("diagnostics-long");
        let path = root.join("long.log");
        let mut text = String::from("START\n");
        while text.len() < 600_000 {
            text.push_str("a line in the middle of the log\n");
        }
        text.push_str("THE END\n");
        fs::write(&path, &text).unwrap();
        let read = read_capped(&path, 300_000).unwrap();
        assert!(read.starts_with("START\n") && read.ends_with("THE END\n"), "{}", &read[..100]);
        assert!(read.contains("bytes of this file left out"));
        assert!(read.len() < 310_000);
    }

    #[test]
    fn disk_full_is_recognised() {
        let code = if cfg!(windows) { 112 } else { 28 };
        assert_eq!(DiagnosticError::from(io::Error::from_raw_os_error(code)), DiagnosticError::DiskFull);
    }

    #[test]
    fn times_are_civil_dates() {
        assert_eq!(format_time(UNIX_EPOCH + Duration::from_secs(1_794_160_440)), "2026-11-08 17:54 UTC");
    }
}


#[cfg(test)]
mod error_line_tests {
    use super::error_lines;

    #[test]
    fn only_errors_and_crashes_are_error_lines() {
        let log = "[t1] [warning] [krnl] [t2] __imp__XamMediaVerificationFailedBlocks STUB\n\
                   [t1] [error] [krnl] [t2] Unimplemented XGI message\n\
                   [t1] [info] [core] [t2] Title terminated with an access violation\n\
                   [t1] [warning] [krnl] [t2] [NtCreateFile] FAILED: path='D:\\x'\n";
        assert_eq!(
            error_lines(log),
            ["[t1] [error] [krnl] [t2] Unimplemented XGI message", "[t1] [info] [core] [t2] Title terminated with an access violation"]
        );
    }
}
