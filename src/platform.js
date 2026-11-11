/*
 * Bridge between the interface and the operating system.
 *
 * Inside the Tauri app these functions call the Rust commands in src-tauri/src/lib.rs. Opened in
 * a plain browser (for design work) there is no native side, so they return simulated results (or
 * use localStorage for settings) and the interface keeps working.
 *
 * A folder check is an object with a `status`:
 *   { status: "ok", folder, titleId, mediaId, versionId, versionName, languages }
 *                                                a known build of the game: playable
 *   { status: "unsupportedVersion", folder, titleId, mediaId, languages, xexSha256, imageSha256 }
 *                                                the game, in a build with no manifest yet; the
 *                                                hashes of default.xex (whole, and after its
 *                                                header) are what the player reports
 *   { status: "wrongGame", folder, titleId }     an Xbox 360 executable of another game
 *   { status: "notADirectory", folder }          the path is missing, or neither a folder nor a file
 *   { status: "notADiscImage", folder }          a file that is not an Xbox 360 disc image
 *   { status: "missingXex", folder }             no default.xex in it or its subfolders
 *   { status: "missingXexInImage", folder }      a disc image without default.xex
 *   { status: "invalidXex", folder }             default.xex is not an Xbox 360 executable
 * The game is a folder or a disc image (.iso). `folder` is where the executable was found (the
 * disc image itself for one), or the path that was checked when it was not.
 * `languages` lists the engine language codes found in the folder (see gameLanguages.js).
 */

/** Tauri's global API, present only inside the app (app.withGlobalTauri in tauri.conf.json). */
const tauri = globalThis.__TAURI__;

/** True when running inside the Tauri app rather than a browser. */
export const isNativeApp = Boolean(tauri);

/**
 * Result the browser simulation returns for any folder: the World edition (see versions.toml), or
 * with "?unsupported" in the address an edition the project does not know.
 */
const simulatedCheck = (folder) =>
  simulatedQuery.has("unsupported")
    ? {
        status: "unsupportedVersion",
        folder,
        titleId: "4D5308AB",
        mediaId: "52AC485A",
        languages: ["DEU", "INT"],
        xexSha256: "0f".repeat(32),
        imageSha256: "387bdfa99c137afdbf86a337754822d59743950fa703f6c52f8dcf32c12ebbb6",
      }
    : simulatedWorldCheck(folder);

/** The World edition's check (see versions.toml). */
const simulatedWorldCheck = (folder) => ({
  status: "ok",
  folder,
  titleId: "4D5308AB",
  mediaId: "58122AD6",
  versionId: "world",
  versionName: "World",
  languages: ["CHN", "CZE", "FRA", "HUN", "INT", "KOR", "POL", "PTB", "RUS"],
});

/**
 * Opens the system folder picker and checks the chosen folder. Resolves to null when the user
 * cancels.
 */
export const pickGameFolder = async () => {
  if (!isNativeApp) return simulatedCheck("~/Games/Gears of War 3");
  return tauri.core.invoke("pick_game_folder");
};

/**
 * Opens the system file picker for a disc image (.iso) of the game and checks it. Resolves to null
 * when the user cancels.
 */
export const pickGameImage = async () => {
  if (!isNativeApp) return simulatedCheck("~/Games/Gears of War 3.iso");
  return tauri.core.invoke("pick_game_image");
};

/**
 * Calls `onDrop(path)` with the path of a file or folder dropped on the window. Only in the app: a
 * browser gives pages no paths.
 */
export const onPathDropped = (onDrop) => {
  if (!isNativeApp) return;
  tauri.webview.getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "drop" && event.payload.paths.length === 1) onDrop(event.payload.paths[0]);
  });
};

/** Checks a folder or disc image chosen in an earlier session, or dropped on the window. */
export const checkGameFolder = async (path) => {
  if (!isNativeApp) return simulatedCheck(path);
  return tauri.core.invoke("check_game_folder", { path });
};

/** localStorage key of the settings in the browser simulation. */
const BROWSER_SETTINGS_KEY = "azura.settings";

/**
 * Loads the settings (see src-tauri/src/settings.rs for their shape). In the app they come from
 * azura.toml and launcher.toml; in a browser, from localStorage, which can be unavailable.
 */
export const loadSettings = async () => {
  if (isNativeApp) return tauri.core.invoke("load_settings");
  try {
    return JSON.parse(localStorage.getItem(BROWSER_SETTINGS_KEY) ?? "null");
  } catch {
    return null;
  }
};

/** Saves the settings; in a browser a failure to store them is ignored. */
export const saveSettings = async (settings) => {
  if (isNativeApp) return tauri.core.invoke("save_settings", { settings });
  try {
    localStorage.setItem(BROWSER_SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // Not stored: the settings still apply until the page closes.
  }
};

/** Describes this Mac ({ chip, memoryGb, arch }); null in a browser, where it is unknown. */
export const getSystemInfo = async () => (isNativeApp ? tauri.core.invoke("get_system_info") : null);

/**
 * What this launcher version brings ({ version, items }, from CHANGELOG.md), or null when
 * the changelog has no section for it. A browser shows a sample.
 */
export const getReleaseNotes = async () =>
  isNativeApp
    ? tauri.core.invoke("get_release_notes")
    : { version: "0.1.0", items: ["Builds the game for this computer from your own copy of Gears of War 3."] };

/**
 * Lists the graphics cards the game can run on: { devices, automatic } (see src-tauri/src/gpus.rs).
 * Empty on macOS and in a browser.
 */
export const listGpus = async () =>
  isNativeApp ? tauri.core.invoke("list_gpus") : { devices: [], automatic: null };

/** Calls `listener` when the chosen graphics card is gone and the game starts with the automatic one. */
export const onGpuMissing = (listener) => {
  if (isNativeApp) return tauri.event.listen("gpu-missing", () => listener());
};

/**
 * State of the runtime (see src-tauri/src/install.rs): { installed, buildFound, updateAvailable,
 * path, source, updateKind }. `source` is "build" (a project build, copied), "kit" (built here) or
 * "none"; `updateKind`, for the kit, "link" (seconds) or "build" (minutes). A browser has none
 * installed and pretends a build is available.
 */
export const getRuntimeStatus = async () =>
  isNativeApp
    ? tauri.core.invoke("get_runtime_status")
    : { installed: false, buildFound: true, updateAvailable: true, path: "" };

/**
 * Installs or updates the runtime: copies the project's build, or builds it from the player kit
 * and the player's default.xex (`source` "kit" in the state), which takes minutes and reports its
 * progress through onRuntimeBuildProgress. Resolves to the new state; rejects with { kind, detail }
 * (see BuildError in src-tauri/src/builder.rs).
 */
export const installRuntime = async () =>
  isNativeApp
    ? tauri.core.invoke("install_runtime")
    : { installed: true, buildFound: true, updateAvailable: false, path: "" };

/** Opens the folder picker for the saves; resolves to the path, or null when cancelled. */
export const pickSavesFolder = async () =>
  isNativeApp ? tauri.core.invoke("pick_saves_folder") : "~/Documents/Games Saves/Gears3-Azura";

/**
 * Moves the saves to `folder` and makes it the saves folder. Rejects with { kind, detail }:
 * "destinationHasSaves" (call again with `replace` to replace them), "nestedFolders" or "io".
 */
export const changeSavesFolder = async (folder, replace = false) =>
  isNativeApp ? tauri.core.invoke("change_saves_folder", { folder, replace }) : folder;

/** Opens the file picker for a downloadable content package; null when cancelled. */
export const pickDlcPackage = async () => (isNativeApp ? tauri.core.invoke("pick_dlc_package") : null);

/** Installs a downloadable content package into the saves folder; resolves with its name. */
export const installDlc = async (pkg) => (isNativeApp ? tauri.core.invoke("install_dlc", { package: pkg }) : "RAAM's Shadow");

/** Removes an installed downloadable content by its key in the catalogue. */
export const removeDlc = async (id) => (isNativeApp ? tauri.core.invoke("remove_dlc", { id }) : undefined);

/**
 * The installed downloadable content: { packages, raamsShadow, raamsShadowPlayable }.
 */
export const dlcState = async () =>
  isNativeApp ? tauri.core.invoke("dlc_state") : { packages: [], raamsShadow: false, raamsShadowPlayable: false };

/** Listeners for the simulated game in a browser; the app uses Tauri events instead. */
const simulated = { output: new Set(), exit: new Set() };

/** Lines the browser simulation prints, to design the log panel without a runtime. */
const SIMULATED_OUTPUT = [
  ["stdout", "azura: loading azura.toml"],
  ["stdout", "azura: GPU plugin xenos (Vulkan on Metal)"],
  ["stderr", "azura: simulated run, no game started"],
];

/**
 * Starts the game. Rejects with { kind, detail } when it cannot start (see runtime.rs:
 * "runtimeMissing", "alreadyRunning", "startFailed"). Its output, restarts and end arrive
 * through onGameOutput, onGameRestart and onGameExit.
 */
export const launchGame = async (joinCode = null) => {
  if (isNativeApp) return tauri.core.invoke("launch_game", { joinCode });
  SIMULATED_OUTPUT.forEach(([stream, line], index) => {
    setTimeout(() => simulated.output.forEach((listener) => listener({ stream, line })), 300 * (index + 1));
  });
  setTimeout(() => simulated.exit.forEach((listener) => listener({ code: 0, success: true })), 1200);
};

/**
 * Prepares the shaders of earlier sessions: the runtime makes them with the game suspended and
 * closes. Rejects like launchGame. Its progress arrives through onGameOutput as lines
 * "AZURA_SHADERS <done> <total>" and "AZURA_SHADERS_DONE <total>", and its end through onGameExit.
 */
export const prepareShaders = async () => {
  if (isNativeApp) return tauri.core.invoke("prepare_shaders");
  const total = 1200;
  [0, 300, 700, 1200].forEach((done, index) => {
    setTimeout(
      () => simulated.output.forEach((listener) => listener({ stream: "stdout", line: `AZURA_SHADERS ${done} ${total}` })),
      300 * (index + 1),
    );
  });
  setTimeout(() => simulated.output.forEach((listener) => listener({ stream: "stdout", line: `AZURA_SHADERS_DONE ${total}` })), 1500);
  setTimeout(() => simulated.exit.forEach((listener) => listener({ code: 0, success: true })), 1700);
};

/** Bytes of the stored pipeline descriptions, to tell new ones from those already prepared. */
export const shaderStorageBytes = async () => {
  if (isNativeApp) return tauri.core.invoke("shader_storage_bytes");
  return 4096;
};

/** Calls `listener` with { stream, line } for every line the game prints. */
export const onGameOutput = (listener) => {
  if (isNativeApp) return tauri.event.listen("game-output", (event) => listener(event.payload));
  simulated.output.add(listener);
};

/** Calls `listener` with { code, success } when the game ends. */
export const onGameExit = (listener) => {
  if (isNativeApp) return tauri.event.listen("game-exit", (event) => listener(event.payload));
  simulated.exit.add(listener);
};

/**
 * Calls `listener` when the game closes to start again (to apply settings that only take effect
 * at start-up): the launcher starts it at once, so the game is still running. The browser
 * simulation never restarts.
 */
export const onGameRestart = (listener) => {
  if (isNativeApp) return tauri.event.listen("game-restart", () => listener());
};

/**
 * Asks for a newer launcher version: resolves to { version, currentVersion, notes } or null. It
 * rejects when the update endpoint cannot be reached (offline, or a repository that is still
 * private); a browser has no updater and always resolves to null.
 */
export const checkForUpdate = async () => (isNativeApp ? tauri.core.invoke("check_for_update") : null);

/** Installs the newer version and restarts the launcher; rejects with a message on failure. */
export const installUpdate = async () => tauri.core.invoke("install_update");

/** Calls `listener` with { downloaded, total } while an update downloads. */
export const onUpdateProgress = (listener) => {
  if (isNativeApp) return tauri.event.listen("update-progress", (event) => listener(event.payload));
};

/** Version of the running launcher, from its bundle; `fallback` in a browser. */
export const getAppVersion = async (fallback) => (isNativeApp ? tauri.app.getVersion() : fallback);

/**
 * The state of the game's saves (see SaveStatus in saves.rs): `status` is "ok", "corrupted" or
 * "empty", dates are milliseconds since 1970 and backups come newest first. The interface words it.
 */
export const getSaveStatus = async () => {
  if (isNativeApp) return tauri.core.invoke("get_save_status");
  const now = Date.now();
  return {
    hasSaves: true,
    checkpointCount: 3,
    isCorrupted: false,
    status: "ok",
    savePath: "~/.local/share/azura/B13EBABEBABEBABE/4D5308AB",
    lastSavedMs: now - 3_600_000,
    backups: [
      { id: "backup_preview_manual", createdMs: now - 86_400_000, kind: "manual", sizeBytes: 2_621_440 },
      { id: "backup_preview_auto_before_repair", createdMs: now - 172_800_000, kind: "beforeRepair", sizeBytes: 1_048_576 },
    ],
  };
};

/**
 * Removes broken checkpoint headers (backing the saves up first when there are any) and resolves
 * to how many were removed. Errors reject with { kind, detail }, as SavesError in saves.rs.
 */
export const repairSaves = async () => (isNativeApp ? tauri.core.invoke("repair_saves") : 0);

/** Backs the saves up and clears them for a new campaign. Rejects with { kind, detail }. */
export const resetSaves = async () => {
  if (isNativeApp) return tauri.core.invoke("reset_saves");
};

/** Backs the current saves up and resolves to the new backup. Rejects with { kind, detail }. */
export const createSaveBackup = async () => {
  if (isNativeApp) return tauri.core.invoke("create_save_backup");
  return { id: "backup_preview_manual", createdMs: Date.now(), kind: "manual", sizeBytes: 2_621_440 };
};

/** Replaces the current saves with the backup `backupId`. Rejects with { kind, detail }. */
export const restoreSaveBackup = async (backupId) => {
  if (isNativeApp) return tauri.core.invoke("restore_save_backup", { backupId });
};

/** Opens the save directory in the file manager. */
export const openSavesFolder = async () => {
  if (isNativeApp) return tauri.core.invoke("open_saves_folder");
};


/**
 * The player's achievements for the Achievements section (see src-tauri/src/achievements.rs), for
 * the game in `gameFolder`, with their texts in `gameLanguage` (an engine code), else in
 * `uiLocale`, else in English. Resolves to
 *   { status: "ok", achievements, unlockedCount, earnedGamerscore, totalGamerscore, completedAt }
 * or { status: "noGame" }, { status: "noRuntime" }, { status: "unavailable", detail }.
 * The browser simulation sends 50 placeholder achievements with a few unlocked and one hidden.
 */
export const getAchievements = async ({ gameFolder, gameLanguage, uiLocale }) => {
  if (isNativeApp) return tauri.core.invoke("get_achievements", { gameFolder, gameLanguage, uiLocale });
  if (!gameFolder) return { status: "noGame" };
  const unlockedAt = Date.parse("2026-10-21T21:16:05Z");
  const achievements = Array.from({ length: 50 }, (_, index) => {
    const id = index + 1;
    const unlocked = id <= 3;
    const hidden = id === 19;
    return {
      id,
      name: hidden ? null : `Achievement ${id}`,
      description: hidden ? null : unlocked ? "Simulated achievement, unlocked." : "Simulated achievement, still locked.",
      gamerscore: hidden ? null : 20,
      hidden,
      unlocked,
      unlockedAt: unlocked ? unlockedAt + id * 3_600_000 : null,
      icon: null,
    };
  });
  return { status: "ok", achievements, unlockedCount: 3, earnedGamerscore: 60, totalGamerscore: 1000, completedAt: null };
};

/**
 * Whether there is a working C++ compiler to build the runtime with (see src-tauri/src/toolchain.rs):
 *   { status: "ready", version }
 *   { status: "missing", found, problem, install: { kind, manager?, command? }, manual, installerRunning }
 * `install.kind` is "appleCommandLineTools", "packageManager" or "manual". The browser simulation
 * reports Apple's compiler as installed.
 */
export const getCompilerStatus = async () =>
  isNativeApp ? tauri.core.invoke("get_compiler_status") : { status: "ready", version: "Apple clang 21.0.0" };

/**
 * Installs the compiler: on macOS it opens Apple's dialog and resolves at once (check the status
 * again until it is ready); on Linux it resolves once the package manager is done. Rejects with a
 * message when it cannot install it.
 */
export const installCompiler = async () => (isNativeApp ? tauri.core.invoke("install_compiler") : undefined);

/**
 * Calls `listener` with { phase, done, total } while the runtime is built from the kit: phase is
 * "preparing", "translating", "compiling" (done of total files), "linking" or "installing".
 */
/**
 * Calls `listener({ action, pressed, family })` for every controller action the app reads
 * (src-tauri/src/gamepad.rs). Only in the app.
 */
export const onGamepadInput = (listener) => {
  if (isNativeApp) return tauri.event.listen("gamepad-input", (event) => listener(event.payload));
};

export const onRuntimeBuildProgress = (listener) => {
  if (isNativeApp) return tauri.event.listen("runtime-build-progress", (event) => listener(event.payload));
};

/** Shows a build log (the path a failed build reported) in the system's file manager. */
export const showBuildLog = async (path) => (isNativeApp ? tauri.core.invoke("show_build_log", { path }) : undefined);

// --- Diagnostic file (src-tauri/src/diagnostics.rs) -------------------------------------------

/** The files a diagnostic file would hold: [{ name, kind, bytes, included }]. */
export const diagnosticPlan = async () =>
  isNativeApp
    ? tauri.core.invoke("diagnostic_plan")
    : [
        { name: "summary.txt", kind: "summary", bytes: 2048, included: true },
        { name: "logs/azura_12.log", kind: "gameLog", bytes: 1_800_000, included: true },
        { name: "crash/azura-2026-11-08.ips", kind: "crashReport", bytes: 90_000, included: false },
      ];

/**
 * Asks where to save the diagnostic file and writes it, without the entries named in `leftOut`.
 * Resolves to { path, bytes, files }, or null when the player cancels; rejects with
 * { kind, detail } (nothingChosen, diskFull, unwritable, io).
 */
export const createDiagnostic = async (leftOut, gameOutput) =>
  isNativeApp
    ? tauri.core.invoke("create_diagnostic", { leftOut, gameOutput })
    : { path: "~/Desktop/azura-diagnostic.zip", bytes: 420_000, files: ["summary.txt"] };

/** Shows the last diagnostic file written in the system's file manager. */
export const showDiagnostic = async () => (isNativeApp ? tauri.core.invoke("show_diagnostic") : undefined);

// --- Alboria account (src-tauri/src/alboria.rs) ------------------------------------------------
//
// Each call resolves to the service's answer or rejects with { status, error, message, retryIn,
// attemptsLeft }: `error` is the service's code ("bad_code", "email_taken", "no_account", ...), or
// "network" when the service could not be reached. In a browser a small simulation answers: any
// email works, the code is always 123456, and "taken@example.com" already has an account.

/** The browser simulation's account, once created or signed in to. */
const simulatedAccount = { account: null, devices: [] };
// The service's codes in the simulation: "?codesoff" in the address leaves them off (accounts are
// made unverified, as while Amazon SES could not send them), and "?verifyby=<date>" turns them on
// again with that deadline for unverified accounts.
const simulatedQuery = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
const simulatedCodesOff = simulatedQuery.has("codesoff");
const simulatedVerifyBy = simulatedQuery.get("verifyby");
const simulatedError = (error, extra = {}) => ({ status: 400, error, message: error, retryIn: null, attemptsLeft: null, ...extra });
const SIMULATED_CODE = "123456";

/** Signs this installation in with `gamertag`; { gamertag, tag }. Rejects with no_account until it is linked. */
export const alboriaSignIn = async (gamertag) => {
  if (isNativeApp) return tauri.core.invoke("alboria_sign_in", { gamertag });
  if (!simulatedAccount.account) throw simulatedError("no_account");
  simulatedAccount.account.gamertag = gamertag;
  return { gamertag, tag: simulatedAccount.account.tag };
};

/** Asks for a code: purpose "create" (with the gamertag) or "signin"; { expiresIn, resendIn }. */
/**
 * How the service is (alboria.rs, Status): `{ emailCodes, maintenance, maintenanceUntil, motd, game,
 * warnings }`, with the game `titleId`'s own state when given. In the simulation, "?notices",
 * "?maintenance", "?gameoff", "?outdated" and "?warn" in the address show each state.
 */
export const alboriaStatus = async (titleId = null) => {
  if (isNativeApp) return tauri.core.invoke("alboria_status", { titleId });
  const inMinutes = (minutes) => new Date(Date.now() + minutes * 60_000).toISOString();
  const has = (flag) => simulatedQuery.has(flag);
  return {
    emailCodes: !simulatedCodesOff,
    maintenance: has("maintenance"),
    maintenanceUntil: has("maintenance") ? inMinutes(120) : null,
    motd: has("notices") ? "<h3>Maintenance tonight</h3><p>From <b>22:00</b> to <b>22:30</b> <span style=\"color:#d9306b\">no new rooms</span>. More on <a href=\"https://alboria.org\">alboria.org</a>.</p>" : null,
    game: titleId
      ? {
          online: !has("gameoff"),
          maintenance: false,
          maintenanceUntil: null,
          motd: has("notices") ? "Versus playlists are coming. Until then, invite friends to your room." : null,
          minLauncher: has("outdated") ? "9.0.0" : null,
        }
      : null,
    warnings: has("warn") ? [{ action: "restart", scope: "all", at: inMinutes(12) }, { action: "close_rooms", scope: "game", at: inMinutes(2) }] : [],
  };
};

/** Opens a link of an Alboria notice (https or mailto only) in the system's browser. */
export const openNoticeLink = async (url) => {
  if (isNativeApp) return tauri.core.invoke("open_notice_link", { url });
  window.open(url, "_blank", "noopener");
};

export const alboriaRequestCode = async ({ purpose, email, gamertag = null, language }) => {
  if (isNativeApp) return tauri.core.invoke("alboria_request_code", { purpose, email, gamertag, language });
  if (simulatedCodesOff && purpose === "signin") throw simulatedError("codes_unavailable");
  if (simulatedCodesOff && purpose === "create" && email.trim().toLowerCase() !== "taken@example.com") return { codeNeeded: false };
  const taken = email.trim().toLowerCase() === "taken@example.com";
  if (purpose === "create" && taken) throw simulatedError("email_taken");
  if (purpose === "signin" && !taken && !simulatedAccount.account) throw simulatedError("no_account");
  return { expiresIn: 600, resendIn: 60 };
};

/** Uses the code to create the account or link this installation; { gamertag, tag }. */
export const alboriaVerify = async ({ purpose, email, code, gamertag = "User", language = "en" }) => {
  if (isNativeApp) return tauri.core.invoke("alboria_verify", { purpose, email, code, gamertag, language });
  // With codes off an account is made with no code (null).
  if (!(simulatedCodesOff && code === null) && code !== SIMULATED_CODE) throw simulatedError("bad_code", { attemptsLeft: 4 });
  const verified = !simulatedCodesOff && !simulatedVerifyBy;
  simulatedAccount.account = {
    email, gamertag, tag: purpose === "signin" ? 97 : 3290,
    email_verified: verified, verify_by: verified ? null : simulatedVerifyBy,
  };
  simulatedAccount.devices = [{ public_key: "this", title_id: "4D5308AB", platform: "Mac", created_at: new Date().toISOString(), last_seen_at: new Date().toISOString(), current: true }];
  return {
    gamertag: simulatedAccount.account.gamertag,
    tag: simulatedAccount.account.tag,
    emailVerified: simulatedAccount.account.email_verified,
    verifyBy: simulatedAccount.account.verify_by,
  };
};

/** The account: { email, gamertag, tag, devices: [{ public_key, title_id, platform, created_at, last_seen_at, current }] }. */
export const alboriaAccount = async () => {
  if (isNativeApp) return tauri.core.invoke("alboria_account");
  if (!simulatedAccount.account) throw simulatedError("no_account");
  return { ...simulatedAccount.account, devices: simulatedAccount.devices };
};

/** Removes a device from the account (this one signs this installation out). */
export const alboriaRemoveDevice = async (publicKey) => {
  if (isNativeApp) return tauri.core.invoke("alboria_remove_device", { publicKey });
  simulatedAccount.devices = simulatedAccount.devices.filter((device) => device.public_key !== publicKey);
  if (publicKey === "this") simulatedAccount.account = null;
};

/** Asks for a code sent to `email`, to move the account there; { expiresIn, resendIn }. */
export const alboriaEmailCode = async (email, language) =>
  isNativeApp ? tauri.core.invoke("alboria_email_code", { email, language }) : { expiresIn: 600, resendIn: 60 };

/** Moves the account to `email` with the code sent there. */
export const alboriaChangeEmail = async (email, code) => {
  if (isNativeApp) return tauri.core.invoke("alboria_change_email", { email, code });
  if (code !== SIMULATED_CODE) throw simulatedError("bad_code", { attemptsLeft: 4 });
  simulatedAccount.account.email = email;
};

/** Saves the account's data in a JSON file the player picks; false when they cancel. */
export const alboriaExport = async () => (isNativeApp ? tauri.core.invoke("alboria_export") : true);

/** Asks for the code that confirms deleting the account; { expiresIn, resendIn }. */
export const alboriaDeleteCode = async (language) => {
  if (isNativeApp) return tauri.core.invoke("alboria_delete_code", { language });
  return simulatedCodesOff ? { codeNeeded: false } : { expiresIn: 600, resendIn: 60 };
};

/**
 * Which browser waits on Alboria's website with `code`: `{ browser, place, expires_in }`. In a
 * browser a simulated one answers to any code of 8 letters and digits.
 */
export const alboriaWebLink = async (code, language) => {
  if (isNativeApp) return tauri.core.invoke("alboria_web_link", { code, language });
  if (code.replace(/[^A-Za-z0-9]/g, "").length !== 8) throw simulatedError("no_link");
  return { browser: "Chrome · Windows", place: "Madrid, Comunidad de Madrid, España", expires_in: 104 };
};

/** Allows (or refuses) the browser waiting on the website with `code`. */
export const alboriaWebLinkAnswer = async (code, allow) => {
  if (isNativeApp) return tauri.core.invoke("alboria_web_link_answer", { code, allow });
};

/** Asks for a code to verify the account's email (an account made while codes were off). */
export const alboriaVerifyEmailCode = async (language) => {
  if (isNativeApp) return tauri.core.invoke("alboria_verify_email_code", { language });
  if (simulatedCodesOff) throw simulatedError("codes_unavailable");
  return { expiresIn: 600, resendIn: 60 };
};

/** Verifies the account's email with `code`. */
export const alboriaVerifyEmail = async (code) => {
  if (isNativeApp) return tauri.core.invoke("alboria_verify_email", { code });
  if (code !== SIMULATED_CODE) throw simulatedError("bad_code");
  Object.assign(simulatedAccount.account, { email_verified: true, verify_by: null });
  return true;
};

/** Deletes the account with the code. */
export const alboriaDelete = async (code) => {
  if (isNativeApp) return tauri.core.invoke("alboria_delete", { code });
  if (code !== SIMULATED_CODE) throw simulatedError("bad_code", { attemptsLeft: 4 });
  simulatedAccount.account = null;
  simulatedAccount.devices = [];
};

/** Opens alboria.org in the system browser (the address is fixed on the Rust side); a new tab in a browser. */
export const openAlboriaSite = async () => {
  if (isNativeApp) return tauri.core.invoke("open_alboria_site");
  window.open("https://alboria.org", "_blank", "noopener");
};

/** Opens Alboria's blog in `language` in the system browser (the address is built on the Rust side). */
export const openAlboriaBlog = async (language) => {
  if (isNativeApp) return tauri.core.invoke("open_alboria_blog", { language });
  window.open(`https://alboria.org/${encodeURIComponent(language)}/blog/`, "_blank", "noopener");
};

/** Opens Alboria's official Discord server in the system browser (the address is fixed on the Rust side). */
export const openAlboriaDiscord = async () => {
  if (isNativeApp) return tauri.core.invoke("open_alboria_discord");
  window.open("https://alboria.org/discord", "_blank", "noopener");
};

// --- Friends (alboria-notes:azura/alboria-relay.md, section 4.7) ---------------------------------
// In a browser a small simulation answers, with a few friends whose state changes now and then, so
// the panel can be designed without the service.

const simulatedEvents = { event: new Set(), state: new Set() };
const simulatedFriends = {
  friends: [
    { id: 2, name: "Kabosu#117", favourite: true, presence: { status: "playing", title_id: "4D5308AB", party: "joinable" } },
    { id: 3, name: "CAPI_REX#501", favourite: false, presence: { status: "playing", title_id: "4D5308AB", party: "invite_only" } },
    { id: 4, name: "Nimbus#4827", favourite: false, presence: { status: "online" } },
    { id: 5, name: "Bruma#733", favourite: false, presence: { status: "offline", last_seen: new Date(Date.now() - 30 * 3_600_000).toISOString() } },
  ],
  incoming: [{ id: 6, name: "Viento#58" }],
  outgoing: [{ id: 7, name: "Gale#902" }],
  blocked: [],
  me: { availability: "online" },
  invitations: simulatedQuery.has("invite")
    ? [{ id: "inv1", from: { id: 3, name: "CAPI_REX#501" }, title_id: "4D5308AB", expires_at: new Date(Date.now() + 900_000).toISOString() }]
    : [],
};
const simulatedFriendsAnswer = (path, body) => {
  const f = simulatedFriends;
  const named = (list) => list.find((entry) => entry.id === body.id);
  const drop = (key) => (f[key] = f[key].filter((entry) => entry.id !== body.id));
  switch (path) {
    case "/v1/friends":
      return structuredClone(f);
    case "/v1/friends/request":
      if (!f.outgoing.some((entry) => entry.name === body.name)) f.outgoing.push({ id: 100 + f.outgoing.length, name: body.name });
      return { result: "sent" };
    case "/v1/friends/accept": {
      const from = named(f.incoming);
      drop("incoming");
      if (from) f.friends.push({ ...from, favourite: false, presence: { status: "online" } });
      return {};
    }
    case "/v1/friends/availability":
      f.me.availability = body.availability;
      return { availability: body.availability };
    case "/v1/friends/decline":
      drop("incoming");
      return {};
    case "/v1/friends/cancel":
      drop("outgoing");
      return {};
    case "/v1/friends/remove":
      drop("friends");
      return {};
    case "/v1/friends/favourite": {
      const friend = named(f.friends);
      if (friend) friend.favourite = body.favourite;
      return {};
    }
    case "/v1/friends/block": {
      const friend = named(f.friends);
      drop("friends");
      if (friend) f.blocked.push({ id: friend.id, name: friend.name });
      return {};
    }
    case "/v1/friends/unblock":
      drop("blocked");
      return {};
    case "/v1/friends/room":
      if (named(f.friends)?.presence.party !== "joinable") throw simulatedError("invite_only");
      return { code: "K7QM-2XPA", title_id: "4D5308AB" };
    case "/v1/invitations/room":
      f.invitations = f.invitations.filter((entry) => entry.id !== body.id);
      return { code: "B3RT-9PLQ", title_id: "4D5308AB" };
    case "/v1/invitations/decline":
      f.invitations = f.invitations.filter((entry) => entry.id !== body.id);
      return {};
    case "/v1/invitations":
    case "/v1/reports":
      return { sent: true };
    default:
      throw simulatedError("bad_request");
  }
};

/** A friends' request (section 4.7): `path` such as "/v1/friends/accept", with its body; the answer. */
export const alboriaFriends = async (path, body = {}) =>
  isNativeApp ? tauri.core.invoke("alboria_friends", { path, body }) : simulatedFriendsAnswer(path, body);

/** Asks the running game to join the room `code`; with the game closed, launchGame(code) does. */
export const joinRoom = async (code) => {
  if (isNativeApp) return tauri.core.invoke("join_room", { code });
};

/** Opens Alboria's events channel and keeps it open; stopAlboriaEvents closes it. */
export const startAlboriaEvents = async () => {
  if (isNativeApp) return tauri.core.invoke("alboria_events_start");
  simulatedEvents.state.forEach((listener) => listener({ connected: true }));
};
export const stopAlboriaEvents = async () => {
  if (isNativeApp) return tauri.core.invoke("alboria_events_stop");
};

/** Calls `listener` with { event, data } for each event of the channel. */
export const onAlboriaEvent = (listener) => {
  if (isNativeApp) return tauri.event.listen("alboria-event", (event) => listener(event.payload));
  simulatedEvents.event.add(listener);
};
/** Calls `listener` with { connected } when the channel opens or closes. */
export const onAlboriaEventsState = (listener) => {
  if (isNativeApp) return tauri.event.listen("alboria-events-state", (event) => listener(event.payload));
  simulatedEvents.state.add(listener);
};
/** Browser simulation only: sends an event as the service would, for designing the panel. */
export const simulateAlboriaEvent = (event, data) => simulatedEvents.event.forEach((listener) => listener({ event, data }));

// --- The development launcher's Alboria server ----------------------------------------------------

/** Which Alboria server this launcher talks to: `{ dev, choice, url }` (dev false: the published launcher). */
export const alboriaServer = async () =>
  isNativeApp ? tauri.core.invoke("alboria_server") : { dev: simulatedQuery.has("dev"), choice: "local", url: "http://127.0.0.1:8080" };

/** Development launcher only: talks to `choice` (`local`, `production` or an address) from now on. */
export const setAlboriaServer = async (choice) => {
  if (isNativeApp) return tauri.core.invoke("alboria_server_set", { choice });
  return { dev: true, choice: choice.startsWith("http") ? "other" : choice, url: choice.startsWith("http") ? choice : "http://127.0.0.1:8080" };
};
