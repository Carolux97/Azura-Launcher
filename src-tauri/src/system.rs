//! Describes the computer the launcher runs on, for status lines and the "Auto" graphics preset.

#[cfg(target_os = "macos")]
use std::process::Command;

use serde::Serialize;

/// Hardware summary sent to the interface as JSON.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    /// Processor name, for example "Apple M4" or "AMD Ryzen 7".
    pub chip: Option<String>,
    /// Graphics cards on Linux and Windows, for example ["NVIDIA GeForce RTX 3050 Laptop GPU"], in
    /// the order the system numbers them. Empty on macOS, where the chip is also the GPU.
    pub gpus: Vec<String>,
    /// Installed memory in whole gigabytes.
    pub memory_gb: Option<u64>,
    /// CPU architecture the launcher was built for ("x86_64" or "aarch64").
    pub arch: String,
    /// Operating system the launcher is running on ("linux", "macos" or "windows").
    pub os: String,
}

/// Reads one `sysctl` value on macOS, or `None` when the key does not exist.
#[cfg(target_os = "macos")]
fn sysctl(key: &str) -> Option<String> {
    let output = Command::new("/usr/sbin/sysctl").args(["-n", key]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

/// Converts a byte count to whole gibibytes (GiB).
fn bytes_to_gb(bytes: u64) -> u64 {
    bytes / (1024 * 1024 * 1024)
}

/// Reads the processor description from Linux's CPU information file.
#[cfg(target_os = "linux")]
fn linux_cpu_name() -> Option<String> {
    let contents = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        matches!(key.trim(), "model name" | "Hardware")
            .then(|| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

/// Reads total memory from Linux's memory information file.
#[cfg(target_os = "linux")]
fn linux_memory_gb() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_linux_memory_gb(&contents)
}

#[cfg(target_os = "linux")]
fn parse_linux_memory_gb(contents: &str) -> Option<u64> {
    let kilobytes = contents.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key == "MemTotal").then(|| value.split_whitespace().next()?.parse::<u64>().ok())?
    })?;
    Some(bytes_to_gb(kilobytes * 1024))
}

/// Short vendor names for the PCI vendor IDs of graphics cards.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn gpu_vendor_name(vendor: u16) -> Option<&'static str> {
    match vendor {
        0x10de => Some("NVIDIA"),
        0x1002 => Some("AMD"),
        0x8086 => Some("Intel"),
        _ => None,
    }
}

/// The model in the NVIDIA driver's description of a GPU
/// (/proc/driver/nvidia/gpus/<PCI address>/information), such as
/// "NVIDIA GeForce RTX 3050 Laptop GPU".
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_nvidia_model(information: &str) -> Option<String> {
    information.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "Model")
            .then(|| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

/// The name of a PCI device in a pci.ids database (as /usr/share/hwdata/pci.ids). Device names are
/// written as "chip [marketing name]", as in "GA107M [GeForce RTX 3050 Mobile]": the marketing
/// name is kept, after the vendor's short name ("NVIDIA GeForce RTX 3050 Mobile").
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_pci_ids_name(database: &str, vendor: u16, device: u16) -> Option<String> {
    let vendor_prefix = format!("{vendor:04x}  ");
    let device_prefix = format!("\t{device:04x}  ");
    let mut vendor_name = None;
    for line in database.lines() {
        if let Some(name) = vendor_name {
            if !line.starts_with('\t') && !line.is_empty() && !line.starts_with('#') {
                // The next vendor: the device is not listed.
                break;
            }
            if let Some(device_name) = line.strip_prefix(&device_prefix) {
                let model = match (device_name.find('['), device_name.rfind(']')) {
                    (Some(open), Some(close)) if open < close => &device_name[open + 1..close],
                    _ => device_name,
                };
                let vendor_short = gpu_vendor_name(vendor).unwrap_or(name);
                return Some(format!("{vendor_short} {}", model.trim()));
            }
        } else if let Some(name) = line.strip_prefix(&vendor_prefix) {
            vendor_name = Some(name.trim());
        }
    }
    None
}

/// Reads a hexadecimal PCI ID from a sysfs file ("0x10de").
#[cfg(target_os = "linux")]
fn read_pci_id(path: &std::path::Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    u16::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

/// The graphics cards the kernel's display drivers know (/sys/class/drm/card<N>), named by the
/// NVIDIA driver, the system's pci.ids database, or their vendor alone.
#[cfg(target_os = "linux")]
fn linux_gpu_names() -> Vec<String> {
    use std::path::{Path, PathBuf};
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    // card0, card1...; their connectors (card0-HDMI-A-1) are skipped.
    let mut cards: Vec<(u32, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let number = name.strip_prefix("card")?.parse().ok()?;
            Some((number, entry.path().join("device")))
        })
        .collect();
    cards.sort();
    let database = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok());
    let mut seen = Vec::new();
    let mut names = Vec::new();
    for (_, device) in cards {
        // One PCI device can have several card nodes.
        let Ok(address) = std::fs::canonicalize(&device) else { continue };
        if seen.contains(&address) {
            continue;
        }
        seen.push(address.clone());
        let (Some(vendor), Some(id)) = (read_pci_id(&device.join("vendor")), read_pci_id(&device.join("device")))
        else {
            continue;
        };
        let pci_address = address.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let nvidia_information = Path::new("/proc/driver/nvidia/gpus").join(pci_address).join("information");
        let name = std::fs::read_to_string(nvidia_information)
            .ok()
            .and_then(|information| parse_nvidia_model(&information))
            .or_else(|| database.as_deref().and_then(|database| parse_pci_ids_name(database, vendor, id)))
            .or_else(|| gpu_vendor_name(vendor).map(|vendor| format!("{vendor} GPU")));
        if let Some(name) = name {
            names.push(name);
        }
    }
    names
}

/// Whether a display adapter is a graphics card: one on the PCI bus, as its hardware IDs say
/// ("PCI\VEN_10DE&DEV_2D04&..."). Leaves out the software and virtual adapters Windows lists with
/// them (Microsoft Basic Render Driver, remote desktop and streaming displays), as Linux only lists
/// the cards the kernel drives.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_pci_display_adapter(hardware_ids: &[String]) -> bool {
    hardware_ids.iter().any(|id| id.to_ascii_uppercase().starts_with("PCI\\VEN_"))
}

/// Splits a REG_MULTI_SZ value (UTF-16 strings, each ended by a zero, the list by another) into its
/// strings.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn split_multi_sz(units: &[u16]) -> Vec<String> {
    units
        .split(|&unit| unit == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// The UTF-16 form of `text` with its ending zero, for Windows calls.
#[cfg(target_os = "windows")]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The processor's name, as Windows keeps it in the registry
/// (HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0, ProcessorNameString).
#[cfg(target_os = "windows")]
fn windows_cpu_name() -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let key = wide(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0");
    let value = wide("ProcessorNameString");
    let mut buffer = [0u16; 256];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: the key and value names end in a zero, and `size` is the size of `buffer` in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    let name = String::from_utf16_lossy(&buffer[..length]).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// The installed memory, from Windows' memory status.
#[cfg(target_os = "windows")]
fn windows_memory_gb() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is a MEMORYSTATUSEX with its length set, as the call requires.
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then(|| bytes_to_gb(status.ullTotalPhys))
}

/// The graphics cards present, from SetupAPI's display adapters (the ones Device Manager lists),
/// named as their drivers name them ("NVIDIA GeForce RTX 5060 Ti").
#[cfg(target_os = "windows")]
fn windows_gpu_names() -> Vec<String> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
        DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY, SPDRP_DEVICEDESC, SPDRP_HARDWAREID, SP_DEVINFO_DATA,
    };
    // SAFETY: a class GUID and no enumerator or window, as the call allows.
    let devices = unsafe {
        SetupDiGetClassDevsW(&GUID_DEVCLASS_DISPLAY, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT)
    };
    if devices == -1 {
        return Vec::new();
    }
    // A device property as UTF-16 units, or None when it cannot be read.
    let property = |device: &SP_DEVINFO_DATA, which| -> Option<Vec<u16>> {
        let mut buffer = vec![0u16; 1024];
        // SAFETY: `devices` and `device` come from SetupAPI, and the size is the buffer's in bytes.
        let read = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                devices,
                device,
                which,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 2) as u32,
                std::ptr::null_mut(),
            )
        };
        (read != 0).then_some(buffer)
    };
    let mut names = Vec::new();
    for index in 0.. {
        let mut device = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        // SAFETY: `device` has its size set, as the call requires; it fails past the last device.
        if unsafe { SetupDiEnumDeviceInfo(devices, index, &mut device) } == 0 {
            break;
        }
        let hardware_ids = property(&device, SPDRP_HARDWAREID).map(|units| split_multi_sz(&units)).unwrap_or_default();
        if !is_pci_display_adapter(&hardware_ids) {
            continue;
        }
        let Some(description) = property(&device, SPDRP_DEVICEDESC) else { continue };
        let name = split_multi_sz(&description).into_iter().next().map(|name| name.trim().to_owned());
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            names.push(name);
        }
    }
    // SAFETY: `devices` came from SetupDiGetClassDevsW and is not used afterwards.
    unsafe { SetupDiDestroyDeviceInfoList(devices) };
    names
}

/// Collects the hardware summary.
pub fn system_info() -> SystemInfo {
    #[cfg(target_os = "macos")]
    let (chip, memory_gb) = (
        sysctl("machdep.cpu.brand_string"),
        sysctl("hw.memsize")
            .and_then(|bytes| bytes.parse().ok())
            .map(bytes_to_gb),
    );

    #[cfg(target_os = "linux")]
    let (chip, memory_gb) = (linux_cpu_name(), linux_memory_gb());

    #[cfg(target_os = "windows")]
    let (chip, memory_gb) = (windows_cpu_name(), windows_memory_gb());

    #[cfg(target_os = "linux")]
    let gpus = linux_gpu_names();
    #[cfg(target_os = "windows")]
    let gpus = windows_gpu_names();
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let gpus = Vec::new();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let (chip, memory_gb) = (None, None);

    SystemInfo {
        chip,
        gpus,
        memory_gb,
        arch: std::env::consts::ARCH.to_owned(),
        os: std::env::consts::OS.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pci_display_adapters_are_graphics_cards() {
        let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert!(is_pci_display_adapter(&ids(&["PCI\\VEN_10DE&DEV_2D04&SUBSYS_00000000", "PCI\\VEN_10DE&DEV_2D04"])));
        // Hardware IDs are case-insensitive in Windows.
        assert!(is_pci_display_adapter(&ids(&["pci\\ven_1002&dev_73bf"])));
        // Microsoft Basic Render Driver, and a virtual display of a streaming program.
        assert!(!is_pci_display_adapter(&ids(&["ROOT\\BasicRender"])));
        assert!(!is_pci_display_adapter(&ids(&["Root\\Parsec\\VDA"])));
        assert!(!is_pci_display_adapter(&[]));
    }

    #[test]
    fn splits_a_multi_string_value() {
        let units: Vec<u16> = "PCI\\VEN_10DE&DEV_2D04\0PCI\\VEN_10DE\0\0\0\0".encode_utf16().collect();
        assert_eq!(split_multi_sz(&units), vec!["PCI\\VEN_10DE&DEV_2D04".to_string(), "PCI\\VEN_10DE".to_string()]);
        // A single string, as a device description is.
        let units: Vec<u16> = "NVIDIA GeForce RTX 5060 Ti\0\0\0".encode_utf16().collect();
        assert_eq!(split_multi_sz(&units), vec!["NVIDIA GeForce RTX 5060 Ti".to_string()]);
    }

    #[test]
    fn converts_memory_to_whole_gigabytes() {
        assert_eq!(bytes_to_gb(25_769_803_776), 24);
        assert_eq!(bytes_to_gb(8 * 1024 * 1024 * 1024 + 1), 8);
    }

    #[test]
    fn missing_keys_read_as_none() {
        #[cfg(target_os = "macos")]
        assert_eq!(sysctl("azura.no.such.key"), None);
    }

    #[test]
    fn reads_this_machine() {
        let info = system_info();
        assert!(!info.arch.is_empty());
        assert!(info.memory_gb.unwrap_or(0) > 0);
    }

    #[test]
    fn names_a_gpu_from_the_nvidia_driver() {
        let information = "Model: \t\t NVIDIA GeForce RTX 3050 Laptop GPU\nIRQ:   \t\t 180\n";
        assert_eq!(parse_nvidia_model(information).as_deref(), Some("NVIDIA GeForce RTX 3050 Laptop GPU"));
        assert_eq!(parse_nvidia_model("IRQ: 180\n"), None);
    }

    #[test]
    fn names_a_gpu_from_the_pci_database() {
        let database = "# comment\n\
            1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\
            \t73ff  Navi 23 [Radeon RX 6600/6600 XT/6600M]\n\
            \t\t1458 2316  Navi 23 subsystem\n\
            10de  NVIDIA Corporation\n\
            \t25a2  GA107M [GeForce RTX 3050 Mobile]\n\
            \t2504  GA106 [GeForce RTX 3060 Lite Hash Rate]\n\
            1234  Some Vendor\n\
            \t0001  Plain Device\n";
        assert_eq!(
            parse_pci_ids_name(database, 0x10de, 0x25a2).as_deref(),
            Some("NVIDIA GeForce RTX 3050 Mobile")
        );
        assert_eq!(
            parse_pci_ids_name(database, 0x1002, 0x73ff).as_deref(),
            Some("AMD Radeon RX 6600/6600 XT/6600M")
        );
        // Without a marketing name the whole name, after the vendor's own.
        assert_eq!(parse_pci_ids_name(database, 0x1234, 0x0001).as_deref(), Some("Some Vendor Plain Device"));
        // A device another vendor lists is not this vendor's.
        assert_eq!(parse_pci_ids_name(database, 0x1002, 0x25a2), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linux_memory_information() {
        let contents = "MemTotal:       16384000 kB\nMemFree:         1000000 kB\n";
        assert_eq!(parse_linux_memory_gb(contents), Some(15));
    }
}

