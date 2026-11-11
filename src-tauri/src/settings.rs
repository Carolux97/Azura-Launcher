//! Reads and writes the launcher's settings.
//!
//! They live in two files under the app data folder
//! (`~/Library/Application Support/com.gears3azura.launcher/` on macOS):
//!
//! - `runtime/azura.toml`: the runtime's own config file, which the runtime reads from beside its
//!   executable. It holds every setting that changes how the game runs, under the runtime's
//!   setting (cvar) names. The in-game settings menu (F4) writes to it too, so the launcher only
//!   ever rewrites its own keys and keeps everything else, comments included.
//! - `launcher.toml`: what only the launcher uses (interface language, chosen preset, which game
//!   language was picked, how the Achievements section shows them, and the saves folder, which the runtime receives on the command line;
//!   see `saves.rs`). The game language also reaches the runtime as `user_language`, and as
//!   `user_country`, the console's country, which the game weighs with the language to pick a
//!   regional variant (Spanish from Spain, ESN, or Latin American, ESM).
//!
//! Graphics values are passed through as found: a key missing from `azura.toml` is sent as
//! `null`, and the interface fills it from the selected preset (the preset table lives in
//! `src/graphics.js`, so it is not duplicated here).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml_edit::{value, DocumentMut, Item};

/// Runtime config file, relative to the app data folder.
const RUNTIME_CONFIG: &str = "runtime/azura.toml";

/// Launcher-only config file, relative to the app data folder.
const LAUNCHER_CONFIG: &str = "launcher.toml";

/// Header written at the top of a new `azura.toml`.
const RUNTIME_CONFIG_HEADER: &str = "\
# Azura runtime settings.
#
# Written by the launcher and by the in-game settings menu (F4). Every key is a runtime setting
# (cvar) name; azura.default.toml, beside the runtime, documents all of them.
";

// Runtime setting names the launcher writes. They are defined by the ReXGlue SDK.
const KEY_GAME_DATA_ROOT: &str = "game_data_root";
const KEY_USER_NAME: &str = "user_name";
const KEY_USER_LANGUAGE: &str = "user_language";
const KEY_USER_COUNTRY: &str = "user_country";
const KEY_GPU_PLUGIN: &str = "gpu_plugin";
const KEY_FULLSCREEN: &str = "fullscreen";
/// How the game plays with others: "online" (Alboria, with System Link too) for a player with an
/// account, "system_link" (the local network only) for a local account.
const KEY_NETWORK_MODE: &str = "network_mode";
/// The key that opens the Alboria overlay in the game (a keybind of the runtime).
const KEY_OVERLAY_KEY: &str = "bind_alboria_overlay";
/// The keys the overlay can take: the function keys the game and the runtime leave free (F3, F4 and
/// F7 open the runtime's own overlays). The first is the default.
pub const OVERLAY_KEYS: [&str; 9] = ["F1", "F2", "F5", "F6", "F8", "F9", "F10", "F11", "F12"];

/// `key` when the overlay can take it, else the default.
fn overlay_key(key: Option<&str>) -> &'static str {
    key.and_then(|key| OVERLAY_KEYS.iter().find(|allowed| allowed.eq_ignore_ascii_case(key)))
        .copied()
        .unwrap_or(OVERLAY_KEYS[0])
}

/// `account` in `launcher.toml`: the player signed in to an Alboria account, or chose to play with
/// a local account. With neither, the welcome screen asks.
pub const ACCOUNT_ALBORIA: &str = "alboria";
pub const ACCOUNT_LOCAL: &str = "local";

/// The `network_mode` an account choice gives the game, or `None` to leave the key as it is.
fn network_mode(account: Option<&str>) -> Option<&'static str> {
    match account {
        Some(ACCOUNT_ALBORIA) => Some("online"),
        Some(ACCOUNT_LOCAL) => Some("system_link"),
        _ => None,
    }
}
/// Render resolution of the game's Video menu ("auto", "1280x720", "1920x1080" on macOS, "2560x1440"
/// or "3840x2160").
/// Defined by the runtime, which sets the draw resolution scale and texture memory from it.
const KEY_RENDER_RESOLUTION: &str = "render_resolution";
// What older launchers wrote in its place; the runtime now sets them itself at start-up.
const KEY_SCALE_X: &str = "draw_resolution_scale_x";
const LEGACY_KEYS: [&str; 4] = [
    KEY_SCALE_X,
    "draw_resolution_scale_y",
    "texture_cache_memory_limit_soft",
    "texture_cache_memory_limit_hard",
];

/// Console language ID (the SDK's `XLanguage`) for an engine language code. Czech and Hungarian
/// have none on the console: as on an Xbox 360 set to English in the Czech Republic or Hungary,
/// they are English, and the country (`console_country`) picks them.
fn console_language(code: &str) -> Option<i64> {
    Some(match code {
        "INT" | "CZE" | "HUN" => 1,
        "JPN" => 2,
        "DEU" => 3,
        "FRA" => 4,
        "ESN" | "ESM" => 5,
        "ITA" => 6,
        "KOR" => 7,
        "CHN" => 8,
        "PTB" => 9,
        "POL" => 11,
        "RUS" => 12,
        _ => return None,
    })
}

/// Console country ID (the SDK's country table in `xam_locale.cpp`) for an engine language code: the
/// country of that language's edition, so the game picks the variant the player chose rather than
/// another one of the same language (without a country the SDK reports the United States, and the
/// game looks for Latin American Spanish, ESM, before Spain's, ESN). English (INT) keeps the SDK's
/// default, the United States. Czech and Hungarian, which the console only has as countries, are
/// English with their country, and the runtime shows its own menus in them from that pair.
fn console_country(code: &str) -> Option<i64> {
    Some(match code {
        "ESN" => 31,  // Spain
        "ESM" => 71,  // Mexico
        "FRA" => 34,  // France
        "DEU" => 24,  // Germany
        "ITA" => 50,  // Italy
        "PTB" => 13,  // Brazil
        "POL" => 82,  // Poland
        "RUS" => 88,  // Russia
        "JPN" => 53,  // Japan
        "KOR" => 56,  // Korea
        "CHN" => 101, // Taiwan (traditional Chinese)
        "CZE" => 23,  // Czech Republic
        "HUN" => 42,  // Hungary
        _ => return None,
    })
}

/// GPU plugin the runtime needs to draw anything; written on every save.
const GPU_PLUGIN: &str = "xenos";

/// Graphics values as stored in `azura.toml`; `None` when the key is absent or has another type.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphicsValues {
    /// `render_resolution`, the same setting as the game's Video menu.
    pub render_resolution: Option<String>,
}

fn default_true() -> bool {
    true
}

/// How the Achievements section shows them when the player has not chosen: as a list.
const DEFAULT_ACHIEVEMENTS_VIEW: &str = "list";

fn default_achievements_view() -> String {
    DEFAULT_ACHIEVEMENTS_VIEW.into()
}

/// `render_resolution` for what an older launcher stored with the draw resolution `scale`: "auto"
/// when its render resolution followed the Mac, else the size of that scale (`None` for a scale
/// the menu lacks).
fn legacy_render_resolution(resolution_auto: bool, scale: i64) -> Option<String> {
    if resolution_auto {
        return Some("auto".into());
    }
    match scale {
        1 => Some("1280x720".into()),
        2 => Some("2560x1440".into()),
        3 => Some("3840x2160".into()),
        _ => None,
    }
}

/// Everything the interface reads and writes, exchanged with it as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Game folder (`game_data_root` in `azura.toml`).
    pub game_folder: Option<String>,
    /// Gamertag / player name (`user_name` in `azura.toml`).
    pub user_name: Option<String>,
    /// Display mode (`fullscreen` in `azura.toml`).
    pub fullscreen: bool,
    /// Graphics values found in `azura.toml`.
    #[serde(default)]
    pub graphics: GraphicsValues,
    /// Interface language: "auto" (follow the system) or a launcher locale code.
    pub ui_language: String,
    /// Engine language code of the game ("INT", "ESN", ...), when one is selected.
    pub game_language: Option<String>,
    /// Whether the user picked `game_language` (an automatic choice is recomputed per folder).
    pub game_language_chosen: bool,
    /// How the Achievements section shows them: "list", "badges" or "acts" (see
    /// `src/achievements.js`).
    #[serde(default = "default_achievements_view")]
    pub achievements_view: String,
    /// Launcher version whose "What's new" panel the player dismissed.
    #[serde(default)]
    pub seen_release_notes: Option<String>,
    /// Graphics card the game runs on, by its Vulkan UUID (see `gpus.rs`); `None` lets the game
    /// choose, as it always did.
    #[serde(default)]
    pub gpu: Option<String>,
    /// Folder that holds the saves. Read-only here: `save` keeps the stored one, and only moving
    /// the saves (`SettingsPaths::set_saves_folder`) changes it, so the interface may leave it out.
    #[serde(default)]
    pub saves_folder: String,
    /// Whether the launcher minimizes itself while the game runs, so its window costs the game
    /// nothing to draw, and comes back when the game ends.
    #[serde(default = "default_true")]
    pub minimize_while_playing: bool,
    /// The account the player chose on the welcome screen: [`ACCOUNT_ALBORIA`] or
    /// [`ACCOUNT_LOCAL`]; `None` until they choose, also after updating from a launcher without
    /// accounts. It sets the game's `network_mode`.
    #[serde(default)]
    pub account: Option<String>,
    /// The key that opens the Alboria overlay in the game (`bind_alboria_overlay`): one of
    /// [`OVERLAY_KEYS`].
    #[serde(default = "default_overlay_key")]
    pub overlay_key: String,
}

fn default_overlay_key() -> String {
    OVERLAY_KEYS[0].to_string()
}

/// The part of [`Settings`] stored in `launcher.toml`. Every field has a default, so an old or
/// partial file still loads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct LauncherFile {
    /// Written by older launchers (the render resolution followed the Mac); read only to carry
    /// that choice over to `render_resolution`.
    #[serde(default = "default_true", skip_serializing)]
    resolution_auto: bool,
    ui_language: String,
    game_language: Option<String>,
    game_language_chosen: bool,
    achievements_view: String,
    seen_release_notes: Option<String>,
    gpu: Option<String>,
    saves_folder: Option<String>,
    minimize_while_playing: bool,
    account: Option<String>,
}

impl Default for LauncherFile {
    fn default() -> Self {
        Self {
            resolution_auto: true,
            ui_language: "auto".into(),
            game_language: None,
            game_language_chosen: false,
            achievements_view: default_achievements_view(),
            seen_release_notes: None,
            gpu: None,
            saves_folder: None,
            minimize_while_playing: true,
            account: None,
        }
    }
}

/// Reads a file that may not exist yet; a missing file reads as empty.
fn read_optional(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        result => result,
    }
}

/// Writes a file by writing a sibling temporary file and renaming it over the target, so a crash
/// or a full disk never leaves a half-written config behind.
fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path)
}

fn get_string(document: &DocumentMut, key: &str) -> Option<String> {
    document.get(key)?.as_str().map(str::to_owned)
}

fn get_integer(document: &DocumentMut, key: &str) -> Option<i64> {
    document.get(key)?.as_integer()
}

/// Sets `key` to `item`, or removes it when `item` is `None` (a value the interface left unset).
fn set_or_remove(document: &mut DocumentMut, key: &str, item: Option<Item>) {
    match item {
        Some(item) => document[key] = item,
        None => {
            document.remove(key);
        }
    }
}

/// Paths of the two config files under an app data folder.
pub struct SettingsPaths {
    runtime_config: PathBuf,
    launcher_config: PathBuf,
}

impl SettingsPaths {
    pub fn new(app_data_dir: &Path) -> Self {
        Self {
            runtime_config: app_data_dir.join(RUNTIME_CONFIG),
            launcher_config: app_data_dir.join(LAUNCHER_CONFIG),
        }
    }

    fn load_launcher_file(&self) -> Result<LauncherFile, String> {
        let text = read_optional(&self.launcher_config).map_err(|e| e.to_string())?;
        Ok(toml::from_str(&text).unwrap_or_default())
    }

    fn save_launcher_file(&self, file: &LauncherFile) -> Result<(), String> {
        let text = toml::to_string(file).map_err(|e| e.to_string())?;
        write_atomically(&self.launcher_config, &text).map_err(|e| e.to_string())
    }

    /// Folder for the saves: the one chosen, or the default.
    pub fn saves_folder(&self) -> Result<PathBuf, String> {
        Ok(self
            .load_launcher_file()?
            .saves_folder
            .map(PathBuf::from)
            .unwrap_or_else(crate::saves::default_folder))
    }

    /// Records `folder` as the saves folder (after the saves have been moved there).
    pub fn set_saves_folder(&self, folder: &Path) -> Result<(), String> {
        let mut file = self.load_launcher_file()?;
        file.saves_folder = Some(folder.to_string_lossy().into_owned());
        self.save_launcher_file(&file)
    }

    /// Loads the settings; missing files and missing keys load as defaults.
    pub fn load(&self) -> Result<Settings, String> {
        let runtime_text = read_optional(&self.runtime_config).map_err(|e| e.to_string())?;
        let runtime: DocumentMut = runtime_text
            .parse()
            .map_err(|e| format!("{} is not valid TOML: {e}", self.runtime_config.display()))?;

        let launcher = self.load_launcher_file()?;
        let saves_folder = launcher
            .saves_folder
            .clone()
            .unwrap_or_else(|| crate::saves::default_folder().to_string_lossy().into_owned());

        Ok(Settings {
            game_folder: get_string(&runtime, KEY_GAME_DATA_ROOT),
            user_name: get_string(&runtime, KEY_USER_NAME),
            fullscreen: runtime
                .get(KEY_FULLSCREEN)
                .and_then(Item::as_bool)
                .unwrap_or(true),
            graphics: GraphicsValues {
                render_resolution: get_string(&runtime, KEY_RENDER_RESOLUTION).or_else(|| {
                    get_integer(&runtime, KEY_SCALE_X)
                        .and_then(|scale| legacy_render_resolution(launcher.resolution_auto, scale))
                }),
            },
            ui_language: launcher.ui_language,
            game_language: launcher.game_language,
            game_language_chosen: launcher.game_language_chosen,
            achievements_view: launcher.achievements_view,
            seen_release_notes: launcher.seen_release_notes,
            gpu: launcher.gpu,
            saves_folder,
            minimize_while_playing: launcher.minimize_while_playing,
            account: launcher.account,
            overlay_key: overlay_key(get_string(&runtime, KEY_OVERLAY_KEY).as_deref()).to_string(),
        })
    }

    /// Writes `render_resolution` to `azura.toml` when only an older launcher's scale is there, so
    /// the game starts at the resolution chosen before the Video menu had it. Called before every
    /// launch; a file that already has the key, or none to carry over, is left untouched.
    pub fn migrate_render_resolution(&self) -> Result<(), String> {
        let runtime_text = read_optional(&self.runtime_config).map_err(|e| e.to_string())?;
        let runtime: DocumentMut = runtime_text
            .parse()
            .map_err(|e| format!("{} is not valid TOML: {e}", self.runtime_config.display()))?;
        if runtime.contains_key(KEY_RENDER_RESOLUTION) || !runtime.contains_key(KEY_SCALE_X) {
            return Ok(());
        }
        let settings = self.load()?;
        self.save(&settings)
    }

    /// Saves the settings. `azura.toml` is re-read first and only the launcher's keys change, so
    /// what the in-game menu wrote in the meantime is kept.
    pub fn save(&self, settings: &Settings) -> Result<(), String> {
        let runtime_text = read_optional(&self.runtime_config).map_err(|e| e.to_string())?;
        // A new file gets the header prepended on write: a document holding only comments keeps
        // them as trailing text, so parsing the header first would put it after the keys.
        let is_new = runtime_text.trim().is_empty();
        let mut runtime: DocumentMut = runtime_text
            .parse()
            .map_err(|e| format!("{} is not valid TOML: {e}", self.runtime_config.display()))?;

        let graphics = &settings.graphics;
        runtime[KEY_GPU_PLUGIN] = value(GPU_PLUGIN);
        runtime[KEY_FULLSCREEN] = value(settings.fullscreen);
        set_or_remove(&mut runtime, KEY_GAME_DATA_ROOT, settings.game_folder.as_deref().map(value));
        set_or_remove(
            &mut runtime,
            KEY_USER_NAME,
            settings
                .user_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(value),
        );
        set_or_remove(
            &mut runtime,
            KEY_USER_LANGUAGE,
            settings.game_language.as_deref().and_then(console_language).map(value),
        );
        set_or_remove(
            &mut runtime,
            KEY_USER_COUNTRY,
            settings.game_language.as_deref().and_then(console_country).map(value),
        );
        set_or_remove(
            &mut runtime,
            KEY_RENDER_RESOLUTION,
            graphics.render_resolution.as_deref().map(value),
        );
        if let Some(mode) = network_mode(settings.account.as_deref()) {
            runtime[KEY_NETWORK_MODE] = value(mode);
        }
        runtime[KEY_OVERLAY_KEY] = value(overlay_key(Some(&settings.overlay_key)));
        for key in LEGACY_KEYS {
            runtime.remove(key);
        }
        let runtime_out = if is_new {
            format!("{RUNTIME_CONFIG_HEADER}\n{runtime}")
        } else {
            runtime.to_string()
        };
        write_atomically(&self.runtime_config, &runtime_out).map_err(|e| e.to_string())?;

        let launcher = LauncherFile {
            resolution_auto: true,
            ui_language: settings.ui_language.clone(),
            game_language: settings.game_language.clone(),
            game_language_chosen: settings.game_language_chosen,
            achievements_view: settings.achievements_view.clone(),
            seen_release_notes: settings.seen_release_notes.clone(),
            gpu: settings.gpu.clone(),
            saves_folder: self.load_launcher_file()?.saves_folder,
            minimize_while_playing: settings.minimize_while_playing,
            account: settings.account.clone().filter(|account| account == ACCOUNT_ALBORIA || account == ACCOUNT_LOCAL),
        };
        self.save_launcher_file(&launcher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    /// Creates an empty folder for one test, removed when the test ends.
    fn temp_folder(name: &str) -> TempFolder {
        TempFolder::new(name)
    }

    fn sample_settings() -> Settings {
        Settings {
            game_folder: Some("/Games/Gears of War 3".into()),
            user_name: Some("Marcus".into()),
            fullscreen: true,
            graphics: GraphicsValues {
                render_resolution: Some("2560x1440".into()),
            },
            ui_language: "es".into(),
            game_language: Some("ESN".into()),
            game_language_chosen: true,
            achievements_view: "badges".into(),
            seen_release_notes: Some("0.1.0".into()),
            gpu: Some("0123456789abcdef0123456789abcdef".into()),
            saves_folder: crate::saves::default_folder().to_string_lossy().into_owned(),
            minimize_while_playing: false,
            account: Some(ACCOUNT_ALBORIA.into()),
            overlay_key: "F2".into(),
        }
    }

    #[test]
    fn the_overlay_key_is_written_and_only_a_free_key_is_kept() {
        let dir = temp_folder("overlay-key");
        let paths = SettingsPaths::new(&dir);
        let mut settings = sample_settings();
        paths.save(&settings).unwrap();
        assert!(fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap().contains("bind_alboria_overlay = \"F2\""));
        assert_eq!(paths.load().unwrap().overlay_key, "F2");
        // F4 opens the runtime's settings: the overlay falls back to F1.
        settings.overlay_key = "F4".into();
        paths.save(&settings).unwrap();
        assert_eq!(paths.load().unwrap().overlay_key, "F1");
    }

    #[test]
    fn missing_files_load_as_defaults() {
        let dir = temp_folder("defaults");
        let settings = SettingsPaths::new(&dir).load().unwrap();
        assert_eq!(settings.game_folder, None);
        assert_eq!(settings.user_name, None);
        assert!(settings.fullscreen);
        assert_eq!(settings.graphics, GraphicsValues::default());
        assert_eq!(settings.ui_language, "auto");
        assert_eq!(settings.achievements_view, "list");
        assert_eq!(settings.gpu, None);
        // No account chosen yet: the welcome screen asks.
        assert_eq!(settings.account, None);
    }

    #[test]
    fn the_saves_folder_changes_only_through_set_saves_folder() {
        let dir = temp_folder("saves-folder");
        let paths = SettingsPaths::new(&dir);
        assert_eq!(paths.saves_folder().unwrap(), crate::saves::default_folder());

        paths.set_saves_folder(Path::new("/Volumes/Backup/Gears")).unwrap();
        let mut settings = sample_settings();
        settings.saves_folder = "/somewhere/else".into();
        paths.save(&settings).unwrap();
        assert_eq!(paths.saves_folder().unwrap(), PathBuf::from("/Volumes/Backup/Gears"));
        assert_eq!(paths.load().unwrap().saves_folder, "/Volumes/Backup/Gears");
    }

    #[test]
    fn settings_without_a_saves_folder_still_deserialize() {
        // The interface may send its settings without the saves folder; they must still save.
        let json = r#"{"gameFolder":"/Games/Gears of War 3","userName":"Marcus","fullscreen":false,
            "graphics":{},"uiLanguage":"auto","gameLanguage":null,
            "gameLanguageChosen":false}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.game_folder.as_deref(), Some("/Games/Gears of War 3"));
        assert_eq!(settings.achievements_view, "list");
    }

    #[test]
    fn saved_settings_load_back_unchanged() {
        let dir = temp_folder("roundtrip");
        let paths = SettingsPaths::new(&dir);
        paths.save(&sample_settings()).unwrap();
        assert_eq!(paths.load().unwrap(), sample_settings());
    }

    #[test]
    fn writes_the_runtime_setting_names() {
        let dir = temp_folder("names");
        SettingsPaths::new(&dir).save(&sample_settings()).unwrap();
        let text = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
        for line in [
            "gpu_plugin = \"xenos\"",
            "user_name = \"Marcus\"",
            "fullscreen = true",
            "game_data_root = \"/Games/Gears of War 3\"",
            "user_language = 5",
            "user_country = 31",
            "render_resolution = \"2560x1440\"",
        ] {
            assert!(text.contains(line), "missing `{line}` in:\n{text}");
        }
        // Set in the game's Options menu, not by the launcher.
        assert!(!text.contains("anisotropic_override"));
        assert!(!text.contains("present_effect"));
    }

    #[test]
    fn a_new_runtime_config_starts_with_its_header() {
        let dir = temp_folder("header");
        SettingsPaths::new(&dir).save(&sample_settings()).unwrap();
        let text = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
        assert!(text.starts_with(RUNTIME_CONFIG_HEADER), "header not first in:\n{text}");
    }

    #[test]
    fn keeps_keys_and_comments_written_by_others() {
        let dir = temp_folder("merge");
        let config = dir.join(RUNTIME_CONFIG);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "# Set from the F4 menu.\nvsync = false\n").unwrap();

        SettingsPaths::new(&dir).save(&sample_settings()).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(text.contains("# Set from the F4 menu."));
        assert!(text.contains("vsync = false"));
    }

    #[test]
    fn the_runtime_sets_the_scale_and_texture_memory_itself() {
        let dir = temp_folder("legacy-keys");
        let config = dir.join(RUNTIME_CONFIG);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            "draw_resolution_scale_x = 2\ndraw_resolution_scale_y = 2\n\
             texture_cache_memory_limit_soft = 768\ntexture_cache_memory_limit_hard = 1536\n",
        )
        .unwrap();

        SettingsPaths::new(&dir).save(&sample_settings()).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        for key in LEGACY_KEYS {
            assert!(!text.contains(key), "`{key}` left in:\n{text}");
        }
    }

    #[test]
    fn an_older_launchers_resolution_carries_over() {
        let dir = temp_folder("migrate");
        let config = dir.join(RUNTIME_CONFIG);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let paths = SettingsPaths::new(&dir);

        // A chosen scale becomes its size.
        fs::write(&config, "draw_resolution_scale_x = 3\ndraw_resolution_scale_y = 3\n").unwrap();
        fs::write(dir.join(LAUNCHER_CONFIG), "resolution_auto = false\n").unwrap();
        paths.migrate_render_resolution().unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(text.contains("render_resolution = \"3840x2160\""), "in:\n{text}");
        assert!(!text.contains("draw_resolution_scale_x"));

        // Automatic (the scale it resolved to was written too) stays automatic.
        fs::write(&config, "draw_resolution_scale_x = 1\n").unwrap();
        fs::write(dir.join(LAUNCHER_CONFIG), "resolution_auto = true\n").unwrap();
        paths.migrate_render_resolution().unwrap();
        assert!(fs::read_to_string(&config).unwrap().contains("render_resolution = \"auto\""));

        // A resolution chosen in the game is never replaced.
        fs::write(&config, "render_resolution = \"1280x720\"\ndraw_resolution_scale_x = 2\n").unwrap();
        paths.migrate_render_resolution().unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(text.contains("render_resolution = \"1280x720\""));
        assert!(text.contains("draw_resolution_scale_x = 2"));
    }

    #[test]
    fn settings_chosen_in_the_game_survive_a_save() {
        let dir = temp_folder("in-game");
        let config = dir.join(RUNTIME_CONFIG);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            "swap_post_effect = \"fxaa_extreme\"\nmotion_blur = false\nmnk_sensitivity = 1.50\n\
             anisotropic_override = 1\npresent_effect = \"cas\"\n",
        )
        .unwrap();

        SettingsPaths::new(&dir).save(&sample_settings()).unwrap();
        let text = fs::read_to_string(&config).unwrap();
        assert!(text.contains("swap_post_effect = \"fxaa_extreme\""));
        assert!(text.contains("motion_blur = false"));
        assert!(text.contains("mnk_sensitivity = 1.50"));
        assert!(text.contains("anisotropic_override = 1"));
        assert!(text.contains("present_effect = \"cas\""));
    }

    #[test]
    fn unset_values_remove_their_keys() {
        let dir = temp_folder("remove");
        let paths = SettingsPaths::new(&dir);
        paths.save(&sample_settings()).unwrap();
        let mut cleared = sample_settings();
        cleared.game_folder = None;
        cleared.user_name = None;
        paths.save(&cleared).unwrap();
        let text = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
        assert!(!text.contains("game_data_root"));
        assert!(!text.contains("user_name"));
    }

    #[test]
    fn values_of_the_wrong_type_load_as_missing() {
        let dir = temp_folder("types");
        let config = dir.join(RUNTIME_CONFIG);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "render_resolution = 2\nfullscreen = 1\n").unwrap();
        let settings = SettingsPaths::new(&dir).load().unwrap();
        assert_eq!(settings.graphics.render_resolution, None);
        assert!(settings.fullscreen);
    }

    #[test]
    fn invalid_runtime_config_is_reported_not_overwritten() {
        let dir = temp_folder("invalid");
        let config = dir.join(RUNTIME_CONFIG);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "this is = = not toml").unwrap();
        let paths = SettingsPaths::new(&dir);
        assert!(paths.load().is_err());
        assert!(paths.save(&sample_settings()).is_err());
        assert_eq!(fs::read_to_string(&config).unwrap(), "this is = = not toml");
    }

    #[test]
    fn the_country_follows_the_game_language() {
        let dir = temp_folder("country");
        let paths = SettingsPaths::new(&dir);
        let mut settings = sample_settings();
        let country = |dir: &Path| {
            let text = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
            text.lines().find(|line| line.starts_with("user_country")).map(str::to_owned)
        };
        paths.save(&settings).unwrap();
        assert_eq!(country(&dir).as_deref(), Some("user_country = 31"));
        settings.game_language = Some("FRA".into());
        paths.save(&settings).unwrap();
        assert_eq!(country(&dir).as_deref(), Some("user_country = 34"));
        // English keeps the SDK's default (the United States): no country is written.
        settings.game_language = Some("INT".into());
        paths.save(&settings).unwrap();
        assert_eq!(country(&dir), None);
        // Czech and Hungarian are English on the console, with their country.
        let language = |dir: &Path| {
            let text = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
            text.lines().find(|line| line.starts_with("user_language")).map(str::to_owned)
        };
        settings.game_language = Some("CZE".into());
        paths.save(&settings).unwrap();
        assert_eq!(language(&dir).as_deref(), Some("user_language = 1"));
        assert_eq!(country(&dir).as_deref(), Some("user_country = 23"));
        settings.game_language = Some("HUN".into());
        paths.save(&settings).unwrap();
        assert_eq!(country(&dir).as_deref(), Some("user_country = 42"));
    }

    #[test]
    fn the_account_choice_sets_the_network_mode() {
        let dir = temp_folder("account");
        let paths = SettingsPaths::new(&dir);
        let mut settings = sample_settings();
        paths.save(&settings).unwrap();
        let runtime = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
        assert!(runtime.contains("network_mode = \"online\""));
        assert_eq!(paths.load().unwrap().account.as_deref(), Some(ACCOUNT_ALBORIA));

        settings.account = Some(ACCOUNT_LOCAL.into());
        paths.save(&settings).unwrap();
        let runtime = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
        assert!(runtime.contains("network_mode = \"system_link\""));

        // No choice (or an unknown one) leaves the key as it is and stores nothing.
        settings.account = Some("other".into());
        paths.save(&settings).unwrap();
        let runtime = fs::read_to_string(dir.join(RUNTIME_CONFIG)).unwrap();
        assert!(runtime.contains("network_mode = \"system_link\""));
        assert_eq!(paths.load().unwrap().account, None);
    }
}
