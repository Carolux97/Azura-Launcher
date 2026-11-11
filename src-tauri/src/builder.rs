//! Builds the runtime on the player's machine from their own default.xex
//! (alboria-notes:azura/player-runtime-build.md, sections 5 and 8).
//!
//! The recompiled game cannot be distributed, so the launcher builds it here, with the player kit
//! (`scripts/build-kit.sh`) and the machine's C++ compiler (`toolchain.rs`). The kit's `kit.json`
//! holds the commands, recorded from the project's own CMake build with placeholders this module
//! fills in. In order:
//!
//! 1. **Preparing**: the compiler, the kit, the game build (its manifest), free disk space.
//! 2. **Translating**: the kit's code generator turns the game's PowerPC code into C++, in a work
//!    folder under the app data folder (`build/<game build>/`). It only rewrites the files whose
//!    content changes.
//! 3. **Compiling**: the generated precompiled header, then every generated file, one compiler per
//!    core. Each object has a key beside it: a hash of its source, the command, the precompiled
//!    header and every generated header the compiler reported it included (`-MD`, as Ninja
//!    tracks them). A file whose object has the same key is not compiled again, so an interrupted
//!    build resumes where it stopped and a rebuild only compiles what codegen changed. Keys go by
//!    content, not time: the generated code is deleted after a build, and codegen writes every
//!    file again the next time.
//! 4. **Linking**: our compiled code (in the kit) with the generated objects and the SDK
//!    libraries, into a new runtime folder with the staged files (libraries, assets, settings).
//! 5. **Installing**: the new folder replaces the runtime folder by two renames, keeping the
//!    player's `azura.toml` and logs. A build that fails before this step leaves the installed
//!    runtime as it was; an interruption between the renames is repaired by `recover`.
//!
//! Every command's output goes to `build.log` in the work folder, and progress goes to the
//! interface as `runtime-build-progress` events.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::runtime::executable_path;
use crate::toolchain;

/// Layout of kit.json this launcher reads (`KIT_FORMAT` in scripts/kit/make_kit.py).
pub const KIT_FORMAT: u32 = 1;

/// Environment variable that points at a kit folder, overriding the bundled one.
pub const KIT_ENV: &str = "AZURA_KIT";

/// The kit folder inside the launcher's bundled resources.
pub const BUNDLED_KIT: &str = "kit";

/// The project's kit build folder (`scripts/build-kit.sh`), relative to the project root, for a
/// launcher run from a checkout.
fn project_kit() -> String {
    format!("runtime/out/kit/{}/kit", platform())
}

/// Free space a build needs in the app data folder: the generated code, its objects and a second
/// runtime folder while it is installed (about 600 MB, measured), with room to spare.
pub const REQUIRED_SPACE_BYTES: u64 = 1024 * 1024 * 1024;

/// Event with the build's progress.
pub const PROGRESS_EVENT: &str = "runtime-build-progress";

/// File the work folder records the installed build in (read by the update check).
pub const STATE_FILE: &str = "state.json";

/// The platform name the kit is built for, as scripts/sdk_prepare.sh names it.
pub fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "mac-arm64",
        ("linux", "x86_64") => "linux-amd64",
        ("linux", "aarch64") => "linux-arm64",
        ("windows", "x86_64") => "win-amd64",
        _ => "unsupported",
    }
}

/// kit.json, as scripts/kit/make_kit.py writes it.
#[derive(Debug, Clone, Deserialize)]
pub struct Kit {
    pub format: u32,
    pub platform: String,
    pub revision: String,
    pub keys: KitKeys,
    pub codegen: KitCodegen,
    pub compile: KitCompile,
    pub link: KitLink,
    pub stage: Vec<KitStage>,
}

/// What the generated code depends on (`build`) and what only the link does (`link`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct KitKeys {
    pub build: String,
    pub link: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KitCodegen {
    pub generator: String,
    pub manifests: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KitCompile {
    pub precompile: Vec<String>,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KitLink {
    pub command: Vec<String>,
    pub host_objects: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KitStage {
    pub kind: String,
    pub from: String,
    pub to: String,
}

/// What the work folder records about the runtime it built and installed.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BuildState {
    pub kit_revision: String,
    pub keys: Option<KitKeys>,
    pub xex_sha256: String,
    pub compiler: String,
    /// The generated code's objects the runtime was linked from, relative to the work folder.
    #[serde(default)]
    pub objects: Vec<String>,
}

/// What it takes to bring the installed runtime up to date with a kit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateKind {
    /// Nothing: the runtime was built from this kit, for this game, with this compiler.
    None,
    /// Linking again: only what the link uses changed (our code, the SDK libraries, the staged
    /// files), and the generated code's objects are still there. Under a second.
    Link,
    /// Building again: the generated code depends on what changed (the code generator, the
    /// manifest, the headers, the compile commands), or the game or the compiler changed, or
    /// nothing was built yet. About two minutes, less when codegen leaves most files unchanged.
    Build,
}

/// What it takes to bring a runtime built as `state` (or none) up to date with a kit's `keys`,
/// for the game executable `xex_sha256` and the compiler `compiler`. `installed` says whether the
/// runtime folder still has its executable; `objects_kept` whether every object `state` lists is
/// still in the work folder.
pub fn update_kind(
    state: Option<&BuildState>,
    keys: &KitKeys,
    xex_sha256: &str,
    compiler: &str,
    installed: bool,
    objects_kept: bool,
) -> UpdateKind {
    let Some(state) = state else { return UpdateKind::Build };
    let Some(built) = state.keys.as_ref() else { return UpdateKind::Build };
    if built.build != keys.build || state.xex_sha256 != xex_sha256 || state.compiler != compiler || !objects_kept {
        return UpdateKind::Build;
    }
    if built.link != keys.link || !installed {
        return UpdateKind::Link;
    }
    UpdateKind::None
}

/// Whether every object a build state lists is still in the work folder.
pub fn objects_kept(work: &Path, state: &BuildState) -> bool {
    !state.objects.is_empty() && state.objects.iter().all(|object| work.join(object).is_file())
}

/// A step of the build, for the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Preparing,
    Translating,
    Compiling,
    Linking,
    Installing,
}

/// The build's progress: its step, and for compiling the files done out of all of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    pub done: usize,
    pub total: usize,
}

/// Why the build stopped, sent as `{"kind": "...", "detail": ...}` so the interface can explain each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum BuildError {
    /// No working C++ compiler (the Installation view installs one).
    NoCompiler,
    /// No kit for this platform: the launcher was not packaged with one.
    NoKit,
    /// The kit is not one this launcher can use (another platform, or a newer format).
    #[serde(rename_all = "camelCase")]
    WrongKit { reason: String },
    /// The game folder does not hold a supported build.
    UnsupportedGame,
    /// Not enough free disk space.
    #[serde(rename_all = "camelCase")]
    NotEnoughSpace { needed_mb: u64, free_mb: u64 },
    /// A step failed; `log` is the build log, `summary` its first error lines.
    #[serde(rename_all = "camelCase")]
    StepFailed { phase: Phase, summary: String, log: String },
    /// A build is already running.
    AlreadyRunning,
    /// Reading or writing files failed.
    Io(String),
}

impl From<std::io::Error> for BuildError {
    fn from(error: std::io::Error) -> Self {
        BuildError::Io(error.to_string())
    }
}

/// Where the build's progress goes: the interface (as events) in the app, a recorder in tests.
pub trait BuildEvents: Send + Sync {
    fn progress(&self, progress: Progress);
}

impl BuildEvents for tauri::AppHandle {
    fn progress(&self, progress: Progress) {
        use tauri::Emitter;
        let _ = self.emit(PROGRESS_EVENT, progress);
    }
}

/// The folders a build uses.
pub struct BuildFolders<'a> {
    /// The kit.
    pub kit: &'a Path,
    /// The player's game: the folder where default.xex is, or a disc image.
    pub game: &'a Path,
    /// This game build's work folder (`build/<id>/` under the app data folder).
    pub work: &'a Path,
    /// The runtime folder the launcher starts the game from.
    pub runtime: &'a Path,
}

/// Finds the kit: the folder `AZURA_KIT` names, the one bundled with the launcher, or the
/// project's kit build when the launcher runs from a checkout (found by walking up from the
/// launcher executable).
pub fn find_kit(resources: Option<&Path>) -> Option<PathBuf> {
    let valid = |dir: PathBuf| dir.join("kit.json").is_file().then_some(without_verbatim_prefix(dir));
    if let Some(dir) = std::env::var_os(KIT_ENV).map(PathBuf::from) {
        return valid(dir);
    }
    if let Some(dir) = resources.map(|dir| dir.join(BUNDLED_KIT)).and_then(valid) {
        return Some(dir);
    }
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().skip(1).find_map(|dir| valid(dir.join(project_kit())))
}

/// `path` without the `\\?\` prefix of a Windows verbatim path, when it is a plain drive or network
/// path short enough not to need it. The kit's command templates add its subfolders with `/`
/// (`{kit}/sdk/include`), and Windows only takes `/` as a separator in paths without that prefix:
/// Tauri gives the installed launcher's resource folder as a verbatim path. Unchanged elsewhere.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        if let Some(plain) = path.to_str().and_then(plain_windows_path) {
            return PathBuf::from(plain);
        }
    }
    path
}

/// The plain form of the verbatim Windows path `path` (`\\?\C:\x` to `C:\x`, `\\?\UNC\server\share`
/// to `\\server\share`), or `None` when it is not one or is too long for a plain path.
fn plain_windows_path(path: &str) -> Option<String> {
    // MAX_PATH, the longest path Windows takes without the prefix, including the ending zero.
    const MAX_PLAIN: usize = 259;
    let plain = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        let rest = path.strip_prefix(r"\\?\")?;
        let bytes = rest.as_bytes();
        if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
            return None;
        }
        rest.to_owned()
    };
    (plain.len() <= MAX_PLAIN).then_some(plain)
}

/// Reads a kit and checks it is one this launcher can use here.
pub fn read_kit(dir: &Path) -> Result<Kit, BuildError> {
    let text = fs::read_to_string(dir.join("kit.json")).map_err(|_| BuildError::NoKit)?;
    let kit: Kit = serde_json::from_str(&text).map_err(|e| BuildError::WrongKit { reason: e.to_string() })?;
    if kit.format != KIT_FORMAT {
        return Err(BuildError::WrongKit { reason: format!("kit format {} (this launcher reads {KIT_FORMAT})", kit.format) });
    }
    if kit.platform != platform() {
        return Err(BuildError::WrongKit { reason: format!("a kit for {} on {}", kit.platform, platform()) });
    }
    Ok(kit)
}

/// The record of the runtime the work folder built, if any.
pub fn read_state(work: &Path) -> Option<BuildState> {
    serde_json::from_str(&fs::read_to_string(work.join(STATE_FILE)).ok()?).ok()
}

/// A command template with its placeholders filled in. `{cxx}` and `{objects}` expand to several
/// arguments; the others are replaced inside each argument.
pub fn fill(template: &[String], cxx: &[String], objects: &[String], values: &[(&str, String)]) -> Vec<String> {
    let mut out = Vec::with_capacity(template.len() + objects.len());
    for token in template {
        match token.as_str() {
            "{cxx}" => out.extend(cxx.iter().cloned()),
            "{objects}" => out.extend(objects.iter().cloned()),
            _ => {
                let mut token = token.clone();
                for (key, value) in values {
                    token = token.replace(key, value);
                }
                out.push(token);
            }
        }
    }
    out
}

/// The generated sources, as codegen lists them in sources.cmake (`${CMAKE_CURRENT_LIST_DIR}/<file>`).
pub fn generated_sources(sources_cmake: &str) -> Vec<String> {
    let mut sources: Vec<String> = sources_cmake
        .split_whitespace()
        .filter_map(|token| token.strip_prefix("${CMAKE_CURRENT_LIST_DIR}/"))
        .filter(|file| file.ends_with(".cpp"))
        .map(str::to_owned)
        .collect();
    sources.dedup();
    sources
}

/// Free space on the disk that holds `path`, in bytes.
#[cfg(unix)]
fn free_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid C string and `stat` a writable statvfs.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Unknown elsewhere: the build then goes ahead without checking the free space.
#[cfg(not(unix))]
fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// The key an output is built for: a hash of its input's content and of everything else given.
fn key(input: &Path, extra: &[&str]) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    digest.update(fs::read(input)?);
    for part in extra {
        digest.update([0]);
        digest.update(part.as_bytes());
    }
    Ok(digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// A filled command as it goes into a key: with the kit's folder written as `{kit}` again. The
/// kit's content is in its build key, which clears the objects when it changes, so where the kit
/// is does not change what it compiles. It moves with the launcher: a macOS app moved or opened
/// from a translocated copy, and an AppImage, mounted in a new folder at every start.
fn key_command(command: &[String], kit_dir: &str) -> String {
    command.iter().map(|arg| arg.replace(kit_dir, "{kit}")).collect::<Vec<_>>().join(" ")
}

/// The file beside `output` that holds the key it was built for.
fn key_file(output: &Path) -> PathBuf {
    let mut name = output.as_os_str().to_owned();
    name.push(".key");
    PathBuf::from(name)
}

/// Whether `output` exists and was built for `key`.
fn up_to_date(output: &Path, key: &str) -> bool {
    output.is_file() && fs::read_to_string(key_file(output)).is_ok_and(|built| built == key)
}

/// The build log: every command and its output, in the work folder.
struct Log {
    path: PathBuf,
    file: Mutex<fs::File>,
}

impl Log {
    fn create(work: &Path) -> Result<Self, BuildError> {
        let path = work.join("build.log");
        let file = fs::File::create(&path)?;
        Ok(Self { path, file: Mutex::new(file) })
    }

    fn write(&self, text: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(text.as_bytes());
        }
    }

    /// Runs `command` from `dir`, logging it and its output; the output's first error lines when
    /// it fails.
    fn run(&self, command: &[String], dir: &Path) -> Result<(), String> {
        self.write(&format!("$ {}\n", command.join(" ")));
        let output = crate::host_env::use_host_env(&mut Command::new(&command[0]))
            .args(&command[1..])
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {}: {e}", command[0]))?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
        self.write(&text);
        if output.status.success() {
            return Ok(());
        }
        let errors: Vec<&str> = text.lines().filter(|line| line.contains("error")).take(5).collect();
        Err(if errors.is_empty() { text.lines().rev().take(5).collect::<Vec<_>>().join("\n") } else { errors.join("\n") })
    }
}

/// Builds the runtime and installs it. Only one build runs at a time (`running`).
pub fn build(
    folders: &BuildFolders,
    manifest: &str,
    xex_sha256: &str,
    events: &dyn BuildEvents,
    running: &AtomicBool,
) -> Result<(), BuildError> {
    if running.swap(true, Ordering::SeqCst) {
        return Err(BuildError::AlreadyRunning);
    }
    let result = build_inner(folders, manifest, xex_sha256, events);
    running.store(false, Ordering::SeqCst);
    result
}

fn build_inner(folders: &BuildFolders, manifest: &str, xex_sha256: &str, events: &dyn BuildEvents) -> Result<(), BuildError> {
    let progress = |phase, done, total| events.progress(Progress { phase, done, total });
    progress(Phase::Preparing, 0, 0);
    let compiler = toolchain::compiler().ok_or(BuildError::NoCompiler)?;
    let kit = read_kit(folders.kit)?;
    let manifest_file = Path::new(manifest).file_name().ok_or(BuildError::UnsupportedGame)?;
    let kit_manifest = folders.kit.join(&kit.codegen.manifests).join(manifest_file);
    if !kit_manifest.is_file() {
        return Err(BuildError::UnsupportedGame);
    }
    let work = folders.work;
    fs::create_dir_all(work)?;
    let free = free_space(work).unwrap_or(u64::MAX);
    if free < REQUIRED_SPACE_BYTES {
        return Err(BuildError::NotEnoughSpace { needed_mb: REQUIRED_SPACE_BYTES >> 20, free_mb: free >> 20 });
    }
    let log = Log::create(work)?;
    log.write(&format!("Kit {} ({}), {}\n", kit.revision, kit.platform, compiler.version));

    let kit_dir = folders.kit.to_string_lossy().into_owned();
    let generated = work.join("generated/default");
    let values = |extra: &[(&'static str, String)]| -> Vec<(&'static str, String)> {
        let mut values = vec![
            ("{kit}", kit_dir.clone()),
            ("{generated}", generated.to_string_lossy().into_owned()),
            ("{workdir}", work.to_string_lossy().into_owned()),
        ];
        values.extend(extra.iter().cloned());
        values
    };

    // When only what the link uses changed, the generated code's objects are linked again as they
    // are: no codegen, no compile.
    let state = read_state(work);
    let kept = state.as_ref().is_some_and(|state| objects_kept(work, state));
    let kind = update_kind(state.as_ref(), &kit.keys, xex_sha256, &compiler.version, true, kept);
    let objects = if kind == UpdateKind::Build {
        log.write("Building the generated code\n");
        translate_and_compile(folders, &kit, &kit_manifest, &compiler, &values, &log, &progress)?
    } else {
        log.write("Linking the kept objects again\n");
        state.as_ref().map(|state| state.objects.iter().map(|object| work.join(object).to_string_lossy().into_owned()).collect()).unwrap_or_default()
    };

    progress(Phase::Linking, 0, 0);
    let next = folders.runtime.with_extension("next");
    let _ = fs::remove_dir_all(&next);
    fs::create_dir_all(&next)?;
    let mut all_objects = objects.clone();
    all_objects.extend(kit.link.host_objects.iter().map(|path| folders.kit.join(path).to_string_lossy().into_owned()));
    let output = executable_path(&next);
    let link_objects = link_inputs(work, &all_objects)?;
    let command = fill(&kit.link.command, &compiler.command, &link_objects, &values(&[(
        "{output}",
        output.to_string_lossy().into_owned(),
    )]));
    log.run(&command, work).map_err(|summary| step_failed(Phase::Linking, summary, &log))?;
    for entry in &kit.stage {
        let from = folders.kit.join(&entry.from);
        let to = next.join(&entry.to);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        if entry.kind == "directory" {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }

    progress(Phase::Installing, 0, 0);
    install(&next, folders.runtime)?;
    let state = BuildState {
        kit_revision: kit.revision.clone(),
        keys: Some(kit.keys.clone()),
        xex_sha256: xex_sha256.to_owned(),
        compiler: compiler.version.clone(),
        objects: objects
            .iter()
            .filter_map(|object| Path::new(object).strip_prefix(work).ok())
            .map(|object| object.to_string_lossy().into_owned())
            .collect(),
    };
    fs::write(work.join(STATE_FILE), serde_json::to_string_pretty(&state).unwrap_or_default())?;
    // The generated C++ is only needed to compile; codegen writes it again in half a minute.
    let _ = fs::remove_dir_all(&generated);
    log.write("Installed.\n");
    Ok(())
}

/// The objects as the link command takes them. On Windows a command line holds at most 32767
/// characters, which the generated code's objects alone come close to, so they go in a response
/// file the compiler reads (`@file`), one quoted path per line; elsewhere they are passed as they
/// are.
fn link_inputs(work: &Path, objects: &[String]) -> std::io::Result<Vec<String>> {
    if !cfg!(windows) {
        return Ok(objects.to_vec());
    }
    let file = work.join("link-objects.rsp");
    let text: String = objects.iter().map(|object| format!("\"{}\"\n", object.replace('\\', "/"))).collect();
    fs::write(&file, text)?;
    Ok(vec![format!("@{}", file.to_string_lossy())])
}

/// File in the objects folder with the build key of the kit its objects were compiled with.
const OBJECTS_BUILD_KEY_FILE: &str = "kit-build-key";

/// Codegen and the compile: the generated code's objects, in source order. A kit whose generated
/// code compiles differently (another build key) leaves nothing to reuse, so its objects go first.
/// The objects folder records that key itself, written before the first compile, so a build that
/// was interrupted (before any state.json, on a first build) resumes with the objects it had
/// compiled.
fn translate_and_compile(
    folders: &BuildFolders,
    kit: &Kit,
    kit_manifest: &Path,
    compiler: &toolchain::Compiler,
    values: &(dyn Fn(&[(&'static str, String)]) -> Vec<(&'static str, String)> + Sync),
    log: &Log,
    progress: &(dyn Fn(Phase, usize, usize) + Sync),
) -> Result<Vec<String>, BuildError> {
    let work = folders.work;
    let objects_dir = work.join("obj");
    let objects_key_file = objects_dir.join(OBJECTS_BUILD_KEY_FILE);
    if fs::read_to_string(&objects_key_file).ok().as_deref() != Some(kit.keys.build.as_str()) {
        let _ = fs::remove_dir_all(&objects_dir);
    }
    fs::create_dir_all(&objects_dir)?;
    fs::write(&objects_key_file, &kit.keys.build)?;

    // Codegen reads its manifest and the game's executable from the work folder: the manifest as
    // the kit has it, and game/default.xex, the only file of the game it reads. game/ is a link to
    // the player's game folder, or, for a disc image, a folder with the executable copied out of it.
    // On Windows a link to a folder needs Developer Mode or administrator rights, so the
    // executable is copied there too.
    fs::copy(kit_manifest, work.join("azura_manifest.toml"))?;
    let game_dir = work.join("game");
    match fs::symlink_metadata(&game_dir) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&game_dir)?,
        Ok(_) => fs::remove_file(&game_dir)?,
        Err(_) => {}
    }
    if folders.game.is_file() {
        fs::create_dir_all(&game_dir)?;
        crate::game_folder::copy_xex_from_image(folders.game, &game_dir.join(crate::game_folder::XEX_FILE_NAME))
            .map_err(|error| step_failed(Phase::Translating, format!("could not read the disc image: {error}"), log))?;
    } else {
        #[cfg(unix)]
        std::os::unix::fs::symlink(folders.game, &game_dir)?;
        #[cfg(windows)]
        {
            fs::create_dir_all(&game_dir)?;
            fs::copy(
                folders.game.join(crate::game_folder::XEX_FILE_NAME),
                game_dir.join(crate::game_folder::XEX_FILE_NAME),
            )?;
        }
    }

    progress(Phase::Translating, 0, 0);
    let generator = folders.kit.join(&kit.codegen.generator).to_string_lossy().into_owned();
    log.run(&[generator, "codegen".into(), "azura_manifest.toml".into()], work)
        .map_err(|summary| step_failed(Phase::Translating, summary, log))?;
    let generated = work.join("generated/default");
    let sources = generated_sources(&fs::read_to_string(generated.join("sources.cmake"))?);
    if sources.is_empty() {
        return Err(step_failed(Phase::Translating, "codegen listed no sources".into(), log));
    }

    let total = sources.len();
    progress(Phase::Compiling, 0, total);
    let header = generated.join("azura_pch.h");
    let pch = objects_dir.join("azura_pch.pch");
    let pch_command = fill(&kit.compile.precompile, &compiler.command, &[], &values(&[
        ("{header}", header.to_string_lossy().into_owned()),
        ("{pch}", pch.to_string_lossy().into_owned()),
    ]));
    // The header includes only the SDK's headers, which come with the kit (whose build key clears
    // the objects when they change), so its content, the command and the compiler decide its key:
    // a compiler rejects a precompiled header another version wrote. The precompiled header keeps
    // the absolute paths of the headers it read, so its own key has the kit's folder in it and it
    // is built again when the kit moves. The objects' keys take the precompiled header's key
    // without that folder (`pch_identity`): a new compiler compiles everything again, a moved kit
    // does not.
    let kit_dir = folders.kit.to_string_lossy().into_owned();
    let pch_key = key(&header, &[&pch_command.join(" "), &compiler.version])?;
    let pch_identity = key(&header, &[&key_command(&pch_command, &kit_dir), &compiler.version])?;
    if !up_to_date(&pch, &pch_key) {
        log.run(&pch_command, work).map_err(|summary| step_failed(Phase::Compiling, summary, log))?;
        fs::write(key_file(&pch), &pch_key)?;
    }
    compile_all(&sources, &generated, &objects_dir, &pch_identity, &kit_dir, |source, object| {
        fill(&kit.compile.command, &compiler.command, &[], &values(&[
            ("{pch}", pch.to_string_lossy().into_owned()),
            ("{source}", source.to_string_lossy().into_owned()),
            ("{object}", object.to_string_lossy().into_owned()),
        ]))
    }, log, work, |done| progress(Phase::Compiling, done, total))
}

fn step_failed(phase: Phase, summary: String, log: &Log) -> BuildError {
    BuildError::StepFailed { phase, summary, log: log.path.to_string_lossy().into_owned() }
}

/// The generated headers a compile reported it included, from its dependency file (Make syntax,
/// as `-MD -MF` writes it): every path inside `generated` other than the source itself.
fn generated_dependencies(depfile: &Path, generated: &Path, source: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(depfile) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = depfile_paths(&text)
        .into_iter()
        .skip(1)
        .map(PathBuf::from)
        .filter(|path| path.starts_with(generated) && path != source)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// The paths of a Make-syntax dependency file, the target first: separated by spaces and by
/// backslash-newline continuations, with a space inside a path written as `\ ` (the launcher's
/// own folder on macOS is under "Application Support") and the target ending in a colon.
fn depfile_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&' ') => {
                current.push(' ');
                chars.next();
            }
            '\\' if chars.peek() == Some(&'\n') => {
                chars.next();
            }
            ' ' | '\n' | '\t' | '\r' => {
                if !current.is_empty() {
                    paths.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        paths.push(current);
    }
    if let Some(target) = paths.first_mut() {
        if let Some(stripped) = target.strip_suffix(':') {
            *target = stripped.to_owned();
        }
    }
    paths
}

/// The key of a generated file's object: its source, command (`key_command`) and precompiled
/// header, and the generated headers its last compile included. `None` while there is no
/// dependency file (the file has not been compiled yet), so it is compiled.
fn object_key(source: &Path, depfile: &Path, generated: &Path, command: &str, pch_key: &str) -> Option<String> {
    if !depfile.is_file() {
        return None;
    }
    let mut digest = Sha256::new();
    digest.update(fs::read(source).ok()?);
    digest.update(command.as_bytes());
    digest.update(pch_key.as_bytes());
    for header in generated_dependencies(depfile, generated, source) {
        digest.update(header.to_string_lossy().as_bytes());
        digest.update(fs::read(&header).ok()?);
    }
    Some(digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Compiles every source whose object is missing or was built for another key (see `object_key`),
/// one compiler per core, and returns every object in source order. The compiler is asked for its
/// dependency file beside the object.
#[allow(clippy::too_many_arguments)]
fn compile_all(
    sources: &[String],
    generated: &Path,
    objects_dir: &Path,
    pch_key: &str,
    kit_dir: &str,
    command: impl Fn(&Path, &Path) -> Vec<String> + Sync,
    log: &Log,
    work: &Path,
    report: impl Fn(usize) + Sync,
) -> Result<Vec<String>, BuildError> {
    let jobs: Vec<(PathBuf, PathBuf)> =
        sources.iter().map(|name| (generated.join(name), objects_dir.join(format!("{name}.o")))).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let workers = thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if failure.lock().map(|f| f.is_some()).unwrap_or(true) {
                    return;
                }
                let index = next.fetch_add(1, Ordering::SeqCst);
                let Some((source, object)) = jobs.get(index) else { return };
                let mut depfile = object.as_os_str().to_owned();
                depfile.push(".d");
                let depfile = PathBuf::from(depfile);
                let mut command = command(source, object);
                command.extend(["-MD".to_string(), "-MF".to_string(), depfile.to_string_lossy().into_owned()]);
                let command_key = key_command(&command, kit_dir);
                let key_now = object_key(source, &depfile, generated, &command_key, pch_key);
                let result = if key_now.as_deref().is_some_and(|key| up_to_date(object, key)) {
                    Ok(())
                } else {
                    log.run(&command, work).and_then(|()| {
                        let key = object_key(source, &depfile, generated, &command_key, pch_key)
                            .ok_or_else(|| format!("the compiler wrote no dependency file for {}", source.display()))?;
                        fs::write(key_file(object), key).map_err(|e| e.to_string())
                    })
                };
                if let Err(summary) = result {
                    if let Ok(mut failure) = failure.lock() {
                        failure.get_or_insert(summary);
                    }
                    return;
                }
                report(done.fetch_add(1, Ordering::SeqCst) + 1);
            });
        }
    });
    if let Some(summary) = failure.lock().ok().and_then(|f| f.clone()) {
        return Err(step_failed(Phase::Compiling, summary, log));
    }
    Ok(jobs.into_iter().map(|(_, object)| object.to_string_lossy().into_owned()).collect())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Files of the runtime folder that belong to the player, not to the runtime: they move to the
/// new folder when it replaces the old one.
const PLAYER_ENTRIES: [&str; 2] = ["azura.toml", "logs"];

/// Replaces `runtime` with `next`: `runtime` is renamed out of the way, `next` into its place,
/// then the player's files move from the old folder to the new one, and the old one is removed.
/// The player's files stay in a folder that is kept until they are moved, so an interruption at
/// any point loses none of them (`recover` finishes the job).
fn install(next: &Path, runtime: &Path) -> Result<(), BuildError> {
    let old = runtime.with_extension("old");
    if old.exists() {
        // Left by an earlier interrupted install: its player files come first.
        recover(runtime);
    }
    if runtime.exists() {
        fs::rename(runtime, &old)?;
    }
    fs::rename(next, runtime)?;
    move_player_files(&old, runtime);
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

/// Moves the player's files from `from` to `to`, unless `to` already has them.
fn move_player_files(from: &Path, to: &Path) {
    for name in PLAYER_ENTRIES {
        let source = from.join(name);
        if source.exists() && !to.join(name).exists() {
            let _ = fs::rename(&source, to.join(name));
        }
    }
}

/// Repairs a runtime folder an interrupted install left behind: when the runtime folder is gone
/// but the new one is complete, it takes its place; the player's files are brought back from the
/// old one either way. Run when the launcher starts.
pub fn recover(runtime: &Path) {
    let next = runtime.with_extension("next");
    let old = runtime.with_extension("old");
    if !runtime.exists() && executable_path(&next).is_file() {
        let _ = fs::rename(&next, runtime);
    } else if !runtime.exists() && old.exists() {
        let _ = fs::rename(&old, runtime);
    }
    if runtime.exists() && old.exists() {
        move_player_files(&old, runtime);
        let _ = fs::remove_dir_all(&old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// A kit file or folder somewhere else: a link to it, or on Windows (where a link needs
    /// Developer Mode) a copy.
    fn link_or_copy(from: &Path, to: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(from, to).unwrap();
        #[cfg(windows)]
        if from.is_dir() {
            copy_tree(from, to).unwrap();
        } else {
            fs::copy(from, to).unwrap();
        }
    }

    /// Builds a real runtime from a kit and a game, with the launcher's own code, into temporary
    /// folders. Needs both, so it runs only on demand:
    /// `AZURA_TEST_KIT=<kit> AZURA_TEST_GAME=<game folder> cargo test builds_a_runtime -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn builds_a_runtime_from_a_kit_and_a_game() {
        struct Print;
        impl BuildEvents for Print {
            fn progress(&self, progress: Progress) {
                if progress.phase != Phase::Compiling || progress.done % 50 == 0 || progress.done == progress.total {
                    println!("{:?} {}/{}", progress.phase, progress.done, progress.total);
                }
            }
        }
        let kit = PathBuf::from(std::env::var("AZURA_TEST_KIT").expect("AZURA_TEST_KIT"));
        let game = PathBuf::from(std::env::var("AZURA_TEST_GAME").expect("AZURA_TEST_GAME"));
        let registry = crate::versions::Registry::bundled();
        let crate::game_folder::FolderCheck::Ok { version_id, .. } = crate::game_folder::check_folder(&game, &registry) else {
            panic!("{} is not a supported game folder", game.display());
        };
        let version = registry.versions.iter().find(|v| v.id == version_id).unwrap().clone();
        let dir = TempFolder::new("real-build");
        let (work, runtime) = (dir.join("work"), dir.join("runtime"));
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("azura.toml"), "fullscreen = false\n").unwrap();
        let running = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let folders = BuildFolders { kit: &kit, game: &game, work: &work, runtime: &runtime };
        build(&folders, &version.manifest, &version.xex_sha256, &Print, &running).unwrap();
        println!("built in {:.1} s", started.elapsed().as_secs_f64());
        assert!(executable_path(&runtime).is_file());
        assert_eq!(fs::read_to_string(runtime.join("azura.toml")).unwrap(), "fullscreen = false\n");
        assert_eq!(read_state(&work).unwrap().xex_sha256, version.xex_sha256);
        // A second build reuses every object: only codegen and the link run again.
        let started = std::time::Instant::now();
        build(&folders, &version.manifest, &version.xex_sha256, &Print, &running).unwrap();
        println!("rebuilt in {:.1} s", started.elapsed().as_secs_f64());
        // A first build that was interrupted (no state.json yet), resumed with the kit somewhere
        // else (a moved app, an AppImage mounted again): codegen runs and the precompiled header
        // is built again, but no object is compiled again.
        let objects_written = |work: &Path| -> Vec<(PathBuf, std::time::SystemTime)> {
            let mut objects: Vec<_> = fs::read_dir(work.join("obj"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "o"))
                .map(|path| {
                    let written = fs::metadata(&path).unwrap().modified().unwrap();
                    (path, written)
                })
                .collect();
            objects.sort();
            objects
        };
        let before = objects_written(&work);
        assert!(!before.is_empty());
        fs::remove_file(work.join(STATE_FILE)).unwrap();
        let moved_kit = dir.join("kit-moved");
        fs::create_dir_all(&moved_kit).unwrap();
        for entry in fs::read_dir(&kit).unwrap() {
            let entry = entry.unwrap();
            link_or_copy(&entry.path(), &moved_kit.join(entry.file_name()));
        }
        let started = std::time::Instant::now();
        let moved = BuildFolders { kit: &moved_kit, game: &game, work: &work, runtime: &runtime };
        build(&moved, &version.manifest, &version.xex_sha256, &Print, &running).unwrap();
        println!("resumed with the kit moved in {:.1} s", started.elapsed().as_secs_f64());
        assert_eq!(objects_written(&work), before);
        // A kit whose link changed (our code, the SDK libraries): the kept objects are linked
        // again, without codegen or compiling. Its other files are links to the real kit's.
        let relinked_kit = dir.join("kit-new-link");
        fs::create_dir_all(&relinked_kit).unwrap();
        for entry in fs::read_dir(&kit).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != "kit.json" {
                link_or_copy(&entry.path(), &relinked_kit.join(entry.file_name()));
            }
        }
        let text = fs::read_to_string(kit.join("kit.json")).unwrap();
        let link = read_kit(&kit).unwrap().keys.link;
        fs::write(relinked_kit.join("kit.json"), text.replace(&link, "another link key")).unwrap();
        let state = read_state(&work).unwrap();
        let new_keys = read_kit(&relinked_kit).unwrap().keys;
        assert_eq!(update_kind(Some(&state), &new_keys, &version.xex_sha256, &state.compiler, true, objects_kept(&work, &state)), UpdateKind::Link);
        let started = std::time::Instant::now();
        let folders = BuildFolders { kit: &relinked_kit, game: &game, work: &work, runtime: &runtime };
        build(&folders, &version.manifest, &version.xex_sha256, &Print, &running).unwrap();
        println!("relinked in {:.1} s", started.elapsed().as_secs_f64());
        assert!(fs::read_to_string(work.join("build.log")).unwrap().contains("Linking the kept objects again"));
        assert_eq!(read_state(&work).unwrap().keys.unwrap().link, "another link key");
        if let Ok(keep) = std::env::var("AZURA_TEST_KEEP") {
            let _ = fs::remove_dir_all(&keep);
            fs::rename(&runtime, &keep).unwrap();
        }
    }

    #[test]
    fn keeps_the_generated_headers_of_a_dependency_file() {
        let dir = TempFolder::new("depfile");
        let generated = dir.join("generated");
        let depfile = dir.join("a.o.d");
        fs::write(&depfile, format!(
            "{o}: {g}/azura_recomp.1.cpp \\\n  {g}/azura_funcs.1.h /sdk/include/rex/ppc.h \\\n  {g}/azura_pch.h\n",
            o = dir.join("a.o").display(), g = generated.display())).unwrap();
        let headers = generated_dependencies(&depfile, &generated, &generated.join("azura_recomp.1.cpp"));
        assert_eq!(headers, vec![generated.join("azura_funcs.1.h"), generated.join("azura_pch.h")]);
    }

    #[test]
    fn reads_paths_with_spaces_in_a_dependency_file() {
        let text = "/a/Application\\ Support/x.o: /a/Application\\ Support/g/x.cpp \\\n  /a/Application\\ Support/g/x.h\n";
        assert_eq!(depfile_paths(text), strings(&[
            "/a/Application Support/x.o",
            "/a/Application Support/g/x.cpp",
            "/a/Application Support/g/x.h",
        ]));
    }

    #[test]
    fn says_what_an_update_takes() {
        let keys = KitKeys { build: "b1".into(), link: "l1".into() };
        let state = BuildState {
            kit_revision: "r".into(),
            keys: Some(keys.clone()),
            xex_sha256: "x".into(),
            compiler: "Apple clang 21.0.0".into(),
            objects: vec!["obj/a.o".into()],
        };
        let kind = |keys: &KitKeys, xex: &str, compiler: &str, installed: bool, kept: bool| {
            update_kind(Some(&state), keys, xex, compiler, installed, kept)
        };
        assert_eq!(kind(&keys, "x", "Apple clang 21.0.0", true, true), UpdateKind::None);
        let new_link = KitKeys { build: "b1".into(), link: "l2".into() };
        assert_eq!(kind(&new_link, "x", "Apple clang 21.0.0", true, true), UpdateKind::Link);
        // The runtime folder lost its executable: linking again brings it back.
        assert_eq!(kind(&keys, "x", "Apple clang 21.0.0", false, true), UpdateKind::Link);
        let new_build = KitKeys { build: "b2".into(), link: "l1".into() };
        assert_eq!(kind(&new_build, "x", "Apple clang 21.0.0", true, true), UpdateKind::Build);
        assert_eq!(kind(&keys, "another game", "Apple clang 21.0.0", true, true), UpdateKind::Build);
        assert_eq!(kind(&keys, "x", "Apple clang 22.0.0", true, true), UpdateKind::Build);
        // The objects to link again are gone.
        assert_eq!(kind(&new_link, "x", "Apple clang 21.0.0", true, false), UpdateKind::Build);
        assert_eq!(update_kind(None, &keys, "x", "Apple clang 21.0.0", true, true), UpdateKind::Build);
    }

    #[test]
    fn verbatim_windows_paths_become_plain() {
        assert_eq!(
            plain_windows_path(r"\\?\C:\Users\milci\AppData\Local\Azura Launcher\kit").as_deref(),
            Some(r"C:\Users\milci\AppData\Local\Azura Launcher\kit")
        );
        assert_eq!(plain_windows_path(r"\\?\UNC\server\share\kit").as_deref(), Some(r"\\server\share\kit"));
        // Not verbatim, or not a drive path after the prefix: left alone.
        assert_eq!(plain_windows_path(r"C:\kit"), None);
        assert_eq!(plain_windows_path(r"\\?\Volume{0}\kit"), None);
        assert_eq!(plain_windows_path("/opt/kit"), None);
        // Too long for a plain path: it keeps the prefix.
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(plain_windows_path(&long), None);
    }

    #[test]
    fn fills_the_kit_placeholders() {
        let template = strings(&["{cxx}", "-I{generated}", "{kit}/sdk/lib/x.a", "{objects}", "-o", "{output}"]);
        let filled = fill(&template, &strings(&["xcrun", "clang++"]), &strings(&["a.o", "b.o"]), &[
            ("{generated}", "/w/gen".into()),
            ("{kit}", "/k".into()),
            ("{output}", "/r/azura".into()),
        ]);
        assert_eq!(filled, strings(&["xcrun", "clang++", "-I/w/gen", "/k/sdk/lib/x.a", "a.o", "b.o", "-o", "/r/azura"]));
    }

    #[test]
    fn the_kits_folder_does_not_change_a_commands_key() {
        let template = strings(&["{cxx}", "-I{kit}/sdk/include", "-include-pch", "{pch}"]);
        let filled_in = |kit: &str| {
            fill(&template, &strings(&["clang++"]), &[], &[("{kit}", kit.into()), ("{pch}", "/w/obj/azura_pch.pch".into())])
        };
        let mounted = filled_in("/tmp/.mount_AzuraA1/usr/lib/kit");
        let mounted_again = filled_in("/tmp/.mount_AzuraB2/usr/lib/kit");
        assert_ne!(mounted, mounted_again);
        assert_eq!(
            key_command(&mounted, "/tmp/.mount_AzuraA1/usr/lib/kit"),
            key_command(&mounted_again, "/tmp/.mount_AzuraB2/usr/lib/kit")
        );
        assert_eq!(
            key_command(&mounted, "/tmp/.mount_AzuraA1/usr/lib/kit"),
            "clang++ -I{kit}/sdk/include -include-pch /w/obj/azura_pch.pch"
        );
    }

    #[test]
    fn reads_the_sources_codegen_lists() {
        let text = "set(GENERATED_SOURCES\n    ${CMAKE_CURRENT_LIST_DIR}/azura_init.cpp\n    ${CMAKE_CURRENT_LIST_DIR}/azura_register.cpp\n    ${CMAKE_CURRENT_LIST_DIR}/azura_recomp.0.cpp\n)\n";
        assert_eq!(generated_sources(text), strings(&["azura_init.cpp", "azura_register.cpp", "azura_recomp.0.cpp"]));
    }

    #[test]
    fn a_kit_for_another_platform_or_format_is_refused() {
        let dir = TempFolder::new("kit");
        let kit = |format: u32, platform: &str| format!(
            r#"{{"format":{format},"platform":"{platform}","revision":"r","config":"RelWithDebInfo",
            "keys":{{"build":"b","link":"l"}},"codegen":{{"generator":"sdk/bin/rexglue","manifests":"manifests"}},
            "compile":{{"precompile":[],"command":[]}},"link":{{"command":[],"host_objects":[]}},"stage":[]}}"#);
        fs::write(dir.join("kit.json"), kit(KIT_FORMAT, platform())).unwrap();
        assert!(read_kit(&dir).is_ok());
        fs::write(dir.join("kit.json"), kit(KIT_FORMAT + 1, platform())).unwrap();
        assert!(matches!(read_kit(&dir), Err(BuildError::WrongKit { .. })));
        // A platform no launcher runs on, so the test means the same on every one.
        fs::write(dir.join("kit.json"), kit(KIT_FORMAT, "plan9-mips")).unwrap();
        assert!(matches!(read_kit(&dir), Err(BuildError::WrongKit { .. })));
    }

    #[test]
    fn installing_keeps_the_players_files_and_replaces_the_rest() {
        let dir = TempFolder::new("install");
        let runtime = dir.join("runtime");
        let next = dir.join("runtime.next");
        fs::create_dir_all(runtime.join("logs")).unwrap();
        fs::write(runtime.join("azura.toml"), "fullscreen = false").unwrap();
        fs::write(executable_path(&runtime), "old").unwrap();
        fs::write(runtime.join("logs/azura_001.log"), "log").unwrap();
        fs::create_dir_all(&next).unwrap();
        fs::write(executable_path(&next), "new").unwrap();
        install(&next, &runtime).unwrap();
        assert_eq!(fs::read_to_string(executable_path(&runtime)).unwrap(), "new");
        assert_eq!(fs::read_to_string(runtime.join("azura.toml")).unwrap(), "fullscreen = false");
        assert!(runtime.join("logs/azura_001.log").is_file());
        assert!(!next.exists() && !dir.join("runtime.old").exists());
    }

    #[test]
    fn an_install_interrupted_between_the_renames_is_repaired() {
        let dir = TempFolder::new("recover");
        let runtime = dir.join("runtime");
        // Interrupted after the old folder moved away, before the new one took its place.
        let old = dir.join("runtime.old");
        let next = dir.join("runtime.next");
        fs::create_dir_all(old.join("logs")).unwrap();
        fs::write(old.join("azura.toml"), "fullscreen = false").unwrap();
        fs::create_dir_all(&next).unwrap();
        fs::write(executable_path(&next), "new").unwrap();
        recover(&runtime);
        assert_eq!(fs::read_to_string(executable_path(&runtime)).unwrap(), "new");
        assert_eq!(fs::read_to_string(runtime.join("azura.toml")).unwrap(), "fullscreen = false");
        assert!(runtime.join("logs").is_dir());
        assert!(!old.exists());
    }
}
