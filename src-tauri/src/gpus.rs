//! The graphics cards the game can run on, so the player can choose one when there is more than
//! one (a laptop with integrated and dedicated graphics, a desktop with two cards).
//!
//! The runtime lists them itself (`azura --list_gpus=true`, `runtime/src/gpu_list.cpp`): it is the
//! list Vulkan gives the game, so a card is identified exactly as the game will look for it, by
//! the UUID the launcher passes back as `--vulkan_device_uuid` when the game starts. The UUID does
//! not depend on the order Vulkan lists the cards in, which can change between starts.
//!
//! On Linux, when NVIDIA's Vulkan driver is installed, the launcher has always started the game
//! with PRIME render offload, which puts the game on the NVIDIA card of a hybrid laptop (and hides
//! the other cards from it). That stays the automatic choice; choosing another card turns it off.
//!
//! On macOS there is nothing to choose: the runtime is built for Apple silicon, which has a single
//! GPU and no external graphics cards, so the list is never asked for.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// The environment variables of PRIME render offload. `iniciar-linux.sh` sets them for the launcher
/// itself, so its children inherit them unless they are removed.
#[cfg(target_os = "linux")]
pub const PRIME_OFFLOAD_VARIABLES: [&str; 4] =
    ["__NV_PRIME_RENDER_OFFLOAD", "__GLX_VENDOR_LIBRARY_NAME", "__VK_LAYER_NV_optimus", "VK_DRIVER_FILES"];

/// Turns PRIME render offload on or off for a process the launcher starts.
pub fn set_prime_offload(command: &mut Command, prime_offload: bool) {
    #[cfg(target_os = "linux")]
    {
        if prime_offload {
            command
                .env("__NV_PRIME_RENDER_OFFLOAD", "1")
                .env("__GLX_VENDOR_LIBRARY_NAME", "nvidia")
                .env("__VK_LAYER_NV_optimus", "NVIDIA_only")
                .env("VK_DRIVER_FILES", NVIDIA_ICD);
        } else {
            for variable in PRIME_OFFLOAD_VARIABLES {
                command.env_remove(variable);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (command, prime_offload);
}

/// PCI vendor ID of NVIDIA, for PRIME render offload.
pub const NVIDIA_VENDOR_ID: u32 = 0x10DE;

/// NVIDIA's Vulkan driver manifest: its presence turns PRIME render offload on for the automatic
/// choice.
#[cfg(target_os = "linux")]
pub const NVIDIA_ICD: &str = "/usr/share/vulkan/icd.d/nvidia_icd.json";

/// One graphics card as the runtime lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    /// Vulkan device UUID, 32 hexadecimal digits; empty when the driver cannot tell it, and then
    /// the card cannot be chosen.
    pub uuid: String,
    /// "discrete", "integrated", "virtual", "cpu" or "other".
    #[serde(rename = "type")]
    pub kind: String,
    pub vendor_id: u32,
}

/// The cards, and the UUID of the one the game uses when the player chose none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuList {
    pub devices: Vec<Gpu>,
    /// The automatic choice; `None` when it is not known.
    pub automatic: Option<String>,
}

/// What the runtime prints for `--list_gpus`, before the launcher's own automatic choice is
/// applied.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RuntimeGpuList {
    pub devices: Vec<Gpu>,
    /// The card Vulkan's own automatic choice gives the game (empty if none).
    pub automatic: String,
}

/// Reads the runtime's list from its output: the last line that is the JSON object, since the
/// runtime may print log lines before it.
pub fn parse_runtime_output(output: &str) -> Option<RuntimeGpuList> {
    output
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("{\"devices\""))
        .and_then(|line| serde_json::from_str(line.trim()).ok())
}

/// The card the game uses when the player chose none: the NVIDIA card when the game starts with
/// PRIME render offload (`prime_offload_available`), otherwise the runtime's automatic choice.
pub fn automatic_choice(parsed: &RuntimeGpuList, prime_offload_available: bool) -> Option<String> {
    if prime_offload_available {
        if let Some(nvidia) = parsed.devices.iter().find(|gpu| gpu.vendor_id == NVIDIA_VENDOR_ID) {
            return Some(nvidia.uuid.clone());
        }
    }
    (!parsed.automatic.is_empty()).then(|| parsed.automatic.clone())
}

/// How the game is started for a choice: the card to pass to the runtime, and whether to use
/// PRIME render offload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchChoice {
    /// `--vulkan_device_uuid` for the runtime; `None` lets it choose.
    pub device_uuid: Option<String>,
    pub prime_offload: bool,
    /// The chosen card is not in the computer any more: the game starts with the automatic choice
    /// and the interface tells the player.
    pub chosen_missing: bool,
}

/// Works out how to start the game for the chosen card (`None`, automatic) with the cards
/// present (`None` when they could not be listed: then the choice is passed as it is).
pub fn launch_choice(
    chosen: Option<&str>,
    devices: Option<&[Gpu]>,
    prime_offload_available: bool,
) -> LaunchChoice {
    let automatic = LaunchChoice {
        device_uuid: None,
        prime_offload: prime_offload_available,
        chosen_missing: false,
    };
    let Some(chosen) = chosen.filter(|uuid| !uuid.is_empty()) else {
        return automatic;
    };
    let Some(devices) = devices else {
        return LaunchChoice {
            device_uuid: Some(chosen.to_string()),
            ..automatic
        };
    };
    match devices.iter().find(|gpu| gpu.uuid == chosen) {
        // PRIME render offload hides the other cards from the game: only for an NVIDIA card.
        Some(gpu) => LaunchChoice {
            device_uuid: Some(chosen.to_string()),
            prime_offload: prime_offload_available && gpu.vendor_id == NVIDIA_VENDOR_ID,
            chosen_missing: false,
        },
        None => LaunchChoice { chosen_missing: true, ..automatic },
    }
}

/// Whether PRIME render offload is available here: Linux with NVIDIA's Vulkan driver installed.
pub fn prime_offload_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new(NVIDIA_ICD).is_file()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Asks the runtime in `runtime_dir` for its cards. Without PRIME render offload, so every card is
/// listed. An empty list on macOS (a single GPU) and when the runtime cannot list them.
pub fn list(runtime_dir: &Path) -> Result<GpuList, String> {
    if cfg!(target_os = "macos") {
        return Ok(GpuList::default());
    }
    let parsed = list_from_runtime(runtime_dir)?;
    let automatic = automatic_choice(&parsed, prime_offload_available());
    Ok(GpuList { devices: parsed.devices, automatic })
}

/// Runs `azura --list_gpus=true` and parses its answer.
pub fn list_from_runtime(runtime_dir: &Path) -> Result<RuntimeGpuList, String> {
    let executable = crate::runtime::executable_path(runtime_dir);
    if !executable.is_file() {
        return Err("the runtime is not installed".into());
    }
    let mut command = Command::new(&executable);
    command.arg("--list_gpus=true").current_dir(runtime_dir).stdin(Stdio::null());
    // Without PRIME render offload, which hides every card but the NVIDIA one.
    set_prime_offload(&mut command, false);
    // The same libraries as the game, so the list is the one the game will see.
    crate::host_env::use_host_env(&mut command);
    let output = command
        .output()
        .map_err(|e| format!("cannot run {}: {e}", executable.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_runtime_output(&stdout)
        .ok_or_else(|| format!("the runtime did not list the graphics cards (exit {:?})", output.status.code()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(name: &str, uuid: &str, kind: &str, vendor_id: u32) -> Gpu {
        Gpu { name: name.into(), uuid: uuid.into(), kind: kind.into(), vendor_id }
    }

    fn hybrid() -> Vec<Gpu> {
        vec![
            gpu("AMD Radeon 780M", "aaaa", "integrated", 0x1002),
            gpu("NVIDIA GeForce RTX 4060 Laptop GPU", "bbbb", "discrete", NVIDIA_VENDOR_ID),
        ]
    }

    #[test]
    fn reads_the_list_after_log_lines() {
        let output = "[info] Vulkan instance created\n\
            {\"devices\":[{\"name\":\"AMD Radeon 780M\",\"uuid\":\"aaaa\",\"type\":\"integrated\",\"vendorId\":4098}],\"automatic\":\"aaaa\"}\n";
        let parsed = parse_runtime_output(output).unwrap();
        assert_eq!(parsed.devices, vec![gpu("AMD Radeon 780M", "aaaa", "integrated", 0x1002)]);
        assert_eq!(parsed.automatic, "aaaa");
    }

    #[test]
    fn output_without_a_list_reads_as_none() {
        assert_eq!(parse_runtime_output("Vulkan is not available\n"), None);
    }

    #[test]
    fn the_automatic_choice_is_the_nvidia_card_with_prime_offload() {
        let parsed = RuntimeGpuList { devices: hybrid(), automatic: "aaaa".into() };
        assert_eq!(automatic_choice(&parsed, true), Some("bbbb".into()));
        assert_eq!(automatic_choice(&parsed, false), Some("aaaa".into()));
    }

    #[test]
    fn automatic_starts_as_before() {
        let choice = launch_choice(None, Some(&hybrid()), true);
        assert_eq!(choice, LaunchChoice { device_uuid: None, prime_offload: true, chosen_missing: false });
    }

    #[test]
    fn choosing_the_nvidia_card_keeps_prime_offload() {
        let choice = launch_choice(Some("bbbb"), Some(&hybrid()), true);
        assert_eq!(choice.device_uuid.as_deref(), Some("bbbb"));
        assert!(choice.prime_offload);
    }

    #[test]
    fn choosing_another_card_turns_prime_offload_off() {
        let choice = launch_choice(Some("aaaa"), Some(&hybrid()), true);
        assert_eq!(choice.device_uuid.as_deref(), Some("aaaa"));
        assert!(!choice.prime_offload);
    }

    #[test]
    fn a_missing_card_starts_automatically_and_is_reported() {
        let choice = launch_choice(Some("cccc"), Some(&hybrid()), true);
        assert_eq!(choice, LaunchChoice { device_uuid: None, prime_offload: true, chosen_missing: true });
    }

    #[test]
    fn without_a_list_the_choice_is_passed_as_it_is() {
        let choice = launch_choice(Some("aaaa"), None, false);
        assert_eq!(choice.device_uuid.as_deref(), Some("aaaa"));
        assert!(!choice.chosen_missing);
    }
}
