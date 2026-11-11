//! The C++ compiler the launcher builds the runtime with, on the player's machine
//! (alboria-notes:azura/player-runtime-build.md, section 6).
//!
//! The runtime is the player's own game translated to C++ and compiled there, so the machine needs
//! a compiler. This module finds one and checks that it works (it compiles and links a small C++23
//! program, which needs the C++ standard library headers and the linker too), and when there is
//! none it installs one, asking the player only what the system itself asks:
//!
//! - macOS: Apple's Command Line Tools. `xcode-select --install` opens Apple's own dialog, where
//!   the player clicks Install and accepts Apple's license; the launcher then checks again until
//!   the compiler is there. On a Mac without them, running `clang++` or `xcrun` opens that dialog
//!   by itself, so they are only run once `xcode-select -p` (which never does) names a folder.
//! - Linux: clang from the distribution's package manager, through `pkexec`, so the system asks
//!   for the password in its own dialog. Systems whose base image cannot take packages that way
//!   (Fedora Atomic spins such as Bazzite, SteamOS) get the manual instructions instead, which install
//!   on the system itself: a compiler inside a container is not on the launcher's PATH. On SteamOS
//!   those install clang on the system itself with pacman: its read-only base image is opened for
//!   that, and the packages that ship headers (stripped from SteamOS' image) are installed again.
//!   An update of SteamOS replaces the image, so the player runs them again after one.
//! - Windows: LLVM's clang, with Microsoft's C++ headers and libraries from Visual Studio's Build
//!   Tools (clang finds them by itself), both through `winget` in a console window of its own,
//!   where the player accepts the packages' agreements and Windows asks for permission to install.
//!
//! The manual command for the player's system is always offered as well.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Serialize;

/// Oldest clang major version accepted on Linux. The generated code is C++23 (`-std=gnu++23`),
/// which clang compiles from version 17; the SDK's own Linux builds use clang 20. The exact floor
/// for this game's code is still to be measured on Linux.
pub const MIN_CLANG_MAJOR: u32 = 18;

/// A small program that needs what the generated code needs from the compiler: C++23, the C++
/// standard library headers, and a link against the standard library.
const PROBE_PROGRAM: &str = "#include <atomic>\n#include <string_view>\n#include <unordered_map>\n\
    int main() { std::atomic<int> a{0}; std::unordered_map<std::string_view, int> m; \
    m[\"x\"] = a.load(); return m.size() == 1 ? 0 : 1; }\n";

/// How the compiler can be installed on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InstallMethod {
    /// Apple's Command Line Tools, through Apple's dialog.
    AppleCommandLineTools,
    /// The distribution's package manager, through `pkexec`: `command` is what runs as root.
    #[serde(rename_all = "camelCase")]
    PackageManager { manager: String, command: Vec<String> },
    /// Windows' package manager: `commands` run one after another (LLVM, then the Build Tools).
    #[serde(rename_all = "camelCase")]
    Winget { commands: Vec<Vec<String>> },
    /// None the launcher can run; the player follows `manual`.
    Manual,
}

/// The compiler's state, sent to the interface as JSON (`{"status": "ready", ...}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum CompilerStatus {
    /// A working compiler: `version` as it names itself ("Apple clang 21.0.0", "clang 20.1.8").
    #[serde(rename_all = "camelCase")]
    Ready { version: String },
    /// No working compiler. `found` is the version of one that is there but too old or broken
    /// (`problem` says which), `None` when there is none at all.
    #[serde(rename_all = "camelCase")]
    Missing {
        found: Option<String>,
        problem: Option<String>,
        install: InstallMethod,
        /// The command the player can run in a terminal instead.
        manual: String,
        /// What the interface adds to the manual command for this system: "steamos" (where to
        /// run it, the password sudo needs, and running it again after a SteamOS update).
        manual_note: Option<String>,
        /// Apple's Command Line Tools installer is open (macOS), so they may be on their way.
        installer_running: bool,
    },
}

/// A compiler found on this machine: the command that runs it and the version it reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiler {
    pub command: Vec<String>,
    pub version: String,
    pub major: u32,
}

/// The version an `--version` output names: ("Apple clang 21.0.0", 21) or ("clang 20.1.8", 20).
pub fn parse_version(output: &str) -> Option<(String, u32)> {
    let line = output.lines().next()?;
    let (vendor, rest) = if let Some(rest) = line.split("Apple clang version ").nth(1) {
        ("Apple clang", rest)
    } else {
        ("clang", line.split("clang version ").nth(1)?)
    };
    let version = rest.split_whitespace().next()?;
    let major = version.split('.').next()?.parse().ok()?;
    Some((format!("{vendor} {version}"), major))
}

/// The fields of /etc/os-release that pick the package manager.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsRelease {
    pub id: String,
    pub id_like: Vec<String>,
}

/// Parses /etc/os-release (`KEY=value` lines, values optionally quoted).
pub fn parse_os_release(text: &str) -> OsRelease {
    let mut release = OsRelease::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"').trim_matches('\'').to_ascii_lowercase();
        match key.trim() {
            "ID" => release.id = value,
            "ID_LIKE" => release.id_like = value.split_whitespace().map(str::to_owned).collect(),
            _ => {}
        }
    }
    release
}

/// The packages that give a working clang with the C++ standard library headers, and the command
/// that installs them, for a distribution. `None` for one whose base image cannot take packages
/// (`image_based`: an ostree system such as Bazzite or Silverblue, or SteamOS) or one not known.
pub fn package_install(release: &OsRelease, image_based: bool) -> Option<(String, Vec<String>)> {
    if image_based || release.id == "steamos" {
        return None;
    }
    let family = |names: &[&str]| {
        names.contains(&release.id.as_str()) || release.id_like.iter().any(|like| names.contains(&like.as_str()))
    };
    let (manager, command): (&str, &[&str]) = if family(&["debian", "ubuntu"]) {
        ("apt", &["apt-get", "install", "-y", "clang", "build-essential"])
    } else if family(&["fedora", "rhel", "centos"]) {
        ("dnf", &["dnf", "install", "-y", "clang", "gcc-c++"])
    } else if family(&["arch"]) {
        ("pacman", &["pacman", "-S", "--needed", "--noconfirm", "clang", "base-devel"])
    } else if family(&["suse", "opensuse"]) || release.id.starts_with("opensuse") {
        ("zypper", &["zypper", "install", "-y", "clang", "gcc-c++"])
    } else {
        return None;
    };
    Some((manager.to_owned(), command.iter().map(|part| part.to_string()).collect()))
}

/// The command the player can run by hand: the package command with sudo, or the instructions
/// for a system the launcher cannot install on.
pub fn manual_command(install: &InstallMethod, release: Option<&OsRelease>) -> String {
    match install {
        InstallMethod::AppleCommandLineTools => "xcode-select --install".into(),
        InstallMethod::PackageManager { command, .. } => format!("sudo {}", command.join(" ")),
        // PowerShell, Windows' terminal, separates commands with ";".
        InstallMethod::Winget { commands } => {
            commands.iter().map(|command| quoted_command(command)).collect::<Vec<_>>().join("; ")
        }
        InstallMethod::Manual => match release {
            // On the system itself: the launcher runs the compiler from the host, and the game must
            // link with the host's libraries. SteamOS' repositories have clang 18 from 3.7 on.
            // glibc, linux-api-headers and gcc are installed again for their headers (C, Linux and
            // the C++ standard library), which SteamOS' image leaves out.
            Some(release) if release.id == "steamos" => STEAMOS_MANUAL.into(),
            Some(_) => "rpm-ostree install clang gcc-c++  (then restart)".into(),
            None if cfg!(windows) => "Install LLVM (clang 18 or newer) and Visual Studio 2022 Build Tools with the C++ workload".into(),
            None => "Install clang 18 or newer with your distribution's package manager".into(),
        },
    }
}

/// The manual command on SteamOS, one line for Konsole.
const STEAMOS_MANUAL: &str = "sudo steamos-readonly disable && sudo pacman-key --init && \
    sudo pacman-key --populate archlinux holo && sudo pacman -Sy --noconfirm clang glibc linux-api-headers gcc && \
    sudo steamos-readonly enable";

/// The note the interface shows with the manual command on `release`.
fn manual_note(release: Option<&OsRelease>) -> Option<String> {
    release.filter(|release| release.id == "steamos").map(|_| "steamos".to_string())
}

/// The packages that give a working clang on Windows: LLVM, and the Build Tools with their C++
/// workload, the headers and libraries clang compiles and links with there.
pub fn winget_commands() -> Vec<Vec<String>> {
    let install = |id: &str, extra: &[&str]| {
        let mut command: Vec<String> = ["winget", "install", "--id", id, "--exact"].iter().map(|part| part.to_string()).collect();
        command.extend(extra.iter().map(|part| part.to_string()));
        command
    };
    vec![
        install("LLVM.LLVM", &[]),
        install(
            "Microsoft.VisualStudio.2022.BuildTools",
            &["--override", "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"],
        ),
    ]
}

/// A command as the player types it: arguments with spaces in double quotes.
fn quoted_command(command: &[String]) -> String {
    command
        .iter()
        .map(|part| if part.contains(' ') { format!("\"{part}\"") } else { part.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where LLVM's installer puts clang: a launcher started before it was installed does not see the
/// new PATH.
#[cfg(windows)]
fn llvm_install_dirs() -> Vec<std::path::PathBuf> {
    ["ProgramFiles", "ProgramW6432"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| std::path::PathBuf::from(dir).join("LLVM").join("bin"))
        .collect()
}

/// Apple's Command Line Tools installer, the app `xcode-select --install` opens. Matched on its full
/// path: macOS cuts process names to 16 characters, so the name alone would not match.
const APPLE_INSTALLER_APP: &str = "/System/Library/CoreServices/Install Command Line Developer Tools.app";

/// Whether Apple's Command Line Tools installer is open.
fn apple_installer_running() -> bool {
    cfg!(target_os = "macos")
        && Command::new("pgrep")
            .args(["-f", APPLE_INSTALLER_APP])
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

/// Runs `command` with `args`, returning its output when it succeeds.
fn run(command: &[String], args: &[&str]) -> Option<String> {
    let output = crate::host_env::use_host_env(&mut Command::new(&command[0]))
        .args(&command[1..])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr))
}

/// Compiles and links the probe program with `compiler` in a temporary folder; the error output
/// when that fails.
fn probe(compiler: &[String]) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("azura-compiler-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let output = dir.join(if cfg!(windows) { "probe.exe" } else { "probe" });
    let mut child = crate::host_env::use_host_env(&mut Command::new(&compiler[0]))
        .args(&compiler[1..])
        .args(["-std=gnu++23", "-x", "c++", "-", "-o"])
        .arg(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child.stdin.take().map(|mut stdin| stdin.write_all(PROBE_PROGRAM.as_bytes()));
    let result = child.wait_with_output().map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&dir);
    let result = result?;
    if result.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&result.stderr).lines().take(3).collect::<Vec<_>>().join(" "))
    }
}

/// Apple's compiler, if the Command Line Tools (or Xcode) are installed. Never opens Apple's
/// install dialog: `xcrun` runs only once `xcode-select -p` names an existing folder.
#[cfg(target_os = "macos")]
fn find_compiler() -> Option<Compiler> {
    let developer_dir = run(&["xcode-select".into()], &["-p"])?;
    if !Path::new(developer_dir.trim()).is_dir() {
        return None;
    }
    let command = vec!["xcrun".to_string(), "clang++".to_string()];
    let (version, major) = parse_version(&run(&command, &["--version"])?)?;
    Some(Compiler { command, version, major })
}

/// The newest clang++ on the PATH (clang++, or a versioned clang++-N as Debian names them; on
/// Windows clang++.exe, also where LLVM installs it).
#[cfg(not(target_os = "macos"))]
fn find_compiler() -> Option<Compiler> {
    #[allow(unused_mut)]
    let mut dirs: Vec<std::path::PathBuf> = std::env::split_paths(&std::env::var_os("PATH")?).collect();
    #[cfg(windows)]
    dirs.extend(llvm_install_dirs());
    let suffix = std::env::consts::EXE_SUFFIX;
    let mut names = vec![format!("clang++{suffix}")];
    names.extend((MIN_CLANG_MAJOR..=30).rev().map(|major| format!("clang++-{major}{suffix}")));
    let mut found: Vec<Compiler> = Vec::new();
    for name in names {
        let Some(program) = dirs.iter().map(|dir| dir.join(&name)).find(|file| file.is_file()) else {
            continue;
        };
        let command = vec![program.to_string_lossy().into_owned()];
        if let Some((version, major)) = run(&command, &["--version"]).as_deref().and_then(parse_version) {
            found.push(Compiler { command, version, major });
        }
    }
    found.into_iter().max_by_key(|compiler| compiler.major)
}

/// How to install the compiler here.
fn install_method() -> (InstallMethod, Option<OsRelease>) {
    if cfg!(target_os = "macos") {
        return (InstallMethod::AppleCommandLineTools, None);
    }
    if cfg!(windows) {
        let has_winget = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join("winget.exe").is_file()))
            .unwrap_or(false);
        let method = if has_winget { InstallMethod::Winget { commands: winget_commands() } } else { InstallMethod::Manual };
        return (method, None);
    }
    let release = std::fs::read_to_string("/etc/os-release").ok().map(|text| parse_os_release(&text));
    let image_based = Path::new("/run/ostree-booted").exists();
    let has_pkexec = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join("pkexec").is_file()))
        .unwrap_or(false);
    let method = release
        .as_ref()
        .and_then(|release| package_install(release, image_based))
        .filter(|_| has_pkexec)
        .map(|(manager, command)| InstallMethod::PackageManager { manager, command })
        .unwrap_or(InstallMethod::Manual);
    (method, release)
}

/// The oldest major version accepted: any Apple clang that compiles the probe (Apple ships one
/// compiler per macOS), clang MIN_CLANG_MAJOR elsewhere.
fn minimum_major() -> u32 {
    if cfg!(target_os = "macos") {
        0
    } else {
        MIN_CLANG_MAJOR
    }
}

/// The version of the compiler there is, without checking that it works (cheap, for the update
/// check: a different version means the runtime is built again).
pub fn compiler_version() -> Option<String> {
    find_compiler().map(|compiler| compiler.version)
}

/// The compiler the runtime is built with, when there is a working one.
pub fn compiler() -> Option<Compiler> {
    let compiler = find_compiler()?;
    judge(&compiler, minimum_major(), probe).ok().map(|_| compiler)
}

/// What a compiler that is there means: its version when it can build the runtime, otherwise
/// the problem. `probe` compiles and links the test program with it; it only runs for a compiler
/// new enough, since an old one is reported as such either way.
fn judge(compiler: &Compiler, minimum: u32, probe: impl Fn(&[String]) -> Result<(), String>) -> Result<String, String> {
    if compiler.major < minimum {
        return Err(format!("{} is older than clang {}", compiler.version, minimum));
    }
    probe(&compiler.command)?;
    Ok(compiler.version.clone())
}

/// The compiler's state for the interface.
pub fn status() -> CompilerStatus {
    let found = find_compiler();
    let problem = match found.as_ref().map(|compiler| judge(compiler, minimum_major(), probe)) {
        Some(Ok(version)) => return CompilerStatus::Ready { version },
        Some(Err(problem)) => Some(problem),
        None => None,
    };
    let (install, release) = install_method();
    let manual = manual_command(&install, release.as_ref());
    CompilerStatus::Missing {
        found: found.map(|compiler| compiler.version),
        problem,
        install,
        manual,
        manual_note: manual_note(release.as_ref()),
        installer_running: apple_installer_running(),
    }
}

/// Starts installing the compiler: opens Apple's dialog on macOS (it returns at once; the
/// interface checks the status again until the tools are there), or runs the package manager
/// through pkexec on Linux and waits for it. Fails when there is no way to install it here, or
/// when the player cancels or the package manager fails.
pub fn install() -> Result<(), String> {
    match install_method().0 {
        InstallMethod::AppleCommandLineTools => {
            // Exits with an error when the tools are already installed or the dialog is already
            // open; either way the status check that follows tells the interface what to show.
            Command::new("xcode-select").arg("--install").stdin(Stdio::null()).status().map_err(|e| e.to_string())?;
            Ok(())
        }
        InstallMethod::PackageManager { command, .. } => {
            let status = crate::host_env::use_host_env(&mut Command::new("pkexec"))
                .args(&command)
                .stdin(Stdio::null())
                .status()
                .map_err(|e| e.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("{} exited with {:?}", command[0], status.code()))
            }
        }
        InstallMethod::Winget { commands } => {
            // Each in its own console window (not use_host_env, which hides it), where winget asks
            // the player to accept the package's agreements. A package already installed makes
            // winget exit with an error, so the exit codes say nothing: the status check that
            // follows tells whether the compiler works now.
            for command in &commands {
                Command::new(&command[0]).args(&command[1..]).status().map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        InstallMethod::Manual => Err("the compiler cannot be installed from the launcher on this system".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_apple_and_llvm_versions() {
        assert_eq!(
            parse_version("Apple clang version 21.0.0 (clang-2100.3.34.2)\nTarget: arm64-apple-darwin27.0.0\n"),
            Some(("Apple clang 21.0.0".into(), 21))
        );
        assert_eq!(
            parse_version("Ubuntu clang version 18.1.3 (1ubuntu1)\nTarget: x86_64-pc-linux-gnu\n"),
            Some(("clang 18.1.3".into(), 18))
        );
        assert_eq!(parse_version("g++ (GCC) 14.2.1\n"), None);
    }

    #[test]
    fn reads_os_release() {
        let release = parse_os_release("NAME=\"Bazzite\"\nID=bazzite\nID_LIKE=\"fedora\"\nVARIANT_ID=bazzite-deck\n");
        assert_eq!(release.id, "bazzite");
        assert_eq!(release.id_like, vec!["fedora"]);
    }

    #[test]
    fn picks_the_package_manager_of_each_family() {
        let manager = |text: &str| package_install(&parse_os_release(text), false).map(|(manager, _)| manager);
        assert_eq!(manager("ID=ubuntu\nID_LIKE=debian\n").as_deref(), Some("apt"));
        assert_eq!(manager("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n").as_deref(), Some("apt"));
        assert_eq!(manager("ID=fedora\n").as_deref(), Some("dnf"));
        assert_eq!(manager("ID=endeavouros\nID_LIKE=arch\n").as_deref(), Some("pacman"));
        assert_eq!(manager("ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n").as_deref(), Some("zypper"));
        assert_eq!(manager("ID=gentoo\n"), None);
    }

    #[test]
    fn image_based_systems_get_manual_instructions() {
        let bazzite = parse_os_release("ID=bazzite\nID_LIKE=fedora\n");
        assert_eq!(package_install(&bazzite, true), None);
        let steamos = parse_os_release("ID=steamos\nID_LIKE=arch\n");
        assert_eq!(package_install(&steamos, false), None);
        assert!(manual_command(&InstallMethod::Manual, Some(&bazzite)).starts_with("rpm-ostree install clang"));
        assert_eq!(manual_command(&InstallMethod::Manual, Some(&steamos)), STEAMOS_MANUAL);
    }

    #[test]
    fn steamos_gets_its_own_note() {
        let steamos = parse_os_release("ID=steamos\nID_LIKE=arch\n");
        let arch = parse_os_release("ID=arch\n");
        assert_eq!(manual_note(Some(&steamos)), Some("steamos".into()));
        assert_eq!(manual_note(Some(&arch)), None);
        assert_eq!(manual_note(None), None);
        // A container's compiler is never found on the host's PATH, so no instruction may lead there.
        assert!(!STEAMOS_MANUAL.contains("distrobox"));
        assert!(!manual_command(&InstallMethod::Manual, Some(&parse_os_release("ID=bazzite\n"))).contains("distrobox"));
    }

    #[test]
    fn the_manual_command_is_the_package_command_with_sudo() {
        let (manager, command) = package_install(&parse_os_release("ID=fedora\n"), false).unwrap();
        let install = InstallMethod::PackageManager { manager, command };
        assert_eq!(manual_command(&install, None), "sudo dnf install -y clang gcc-c++");
        assert_eq!(manual_command(&InstallMethod::AppleCommandLineTools, None), "xcode-select --install");
    }

    fn clang(major: u32) -> Compiler {
        Compiler { command: vec!["clang++".into()], version: format!("clang {major}.1.0"), major }
    }

    #[test]
    fn a_compiler_too_old_is_not_tried() {
        let tried = std::cell::Cell::new(false);
        let result = judge(&clang(16), MIN_CLANG_MAJOR, |_| {
            tried.set(true);
            Ok(())
        });
        assert_eq!(result, Err(format!("clang 16.1.0 is older than clang {MIN_CLANG_MAJOR}")));
        assert!(!tried.get());
    }

    #[test]
    fn a_compiler_that_cannot_build_the_probe_is_reported_with_its_error() {
        let result = judge(&clang(20), MIN_CLANG_MAJOR, |_| Err("fatal error: 'atomic' file not found".into()));
        assert_eq!(result, Err("fatal error: 'atomic' file not found".into()));
        assert_eq!(judge(&clang(20), MIN_CLANG_MAJOR, |_| Ok(())), Ok("clang 20.1.0".into()));
    }

    #[test]
    fn the_windows_commands_install_llvm_and_the_build_tools() {
        let manual = manual_command(&InstallMethod::Winget { commands: winget_commands() }, None);
        assert_eq!(
            manual,
            "winget install --id LLVM.LLVM --exact; winget install --id Microsoft.VisualStudio.2022.BuildTools --exact \
             --override \"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\""
        );
    }

    #[test]
    fn this_machine_has_a_working_compiler() {
        // The machines that run these tests build the launcher, so they have one.
        let status = status();
        assert!(matches!(status, CompilerStatus::Ready { .. }), "{status:?}");
    }
}
