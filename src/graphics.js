/*
 * Graphics settings the launcher shows before the game starts, kept free of DOM access so they can be
 * unit tested with Node. Everything else (frame rate limit, anti-aliasing, motion blur, filtering,
 * image scaling, full screen) is in the game's Options menu.
 *
 * Setting written to the runtime (a cvar, read at start-up):
 *   renderResolution  render_resolution: "auto" or a render size ("2560x1440"). It is the same
 *                     setting as the Render resolution row of the game's Video menu; the runtime
 *                     turns it into the draw resolution scale and the texture memory, and resolves
 *                     "auto" with the same rule as autoResolutionScale below
 *                     (runtime/src/menu/pc_options.cpp).
 *
 * The automatic resolution is a first proposal, to be tuned by measuring frame rates on real Apple
 * Silicon hardware.
 */

/** Base render size of the game on the console; resolutionScale multiplies it. */
export const BASE_RENDER_SIZE = { width: 1280, height: 720 };

/**
 * Resolution scales the launcher offers, in order. 1.5 (1920 × 1080) is drawn at 3/2 by the Metal
 * renderer (macOS) and at 2 by Vulkan (Linux, Windows), which scales the image down to the window.
 */
export const RESOLUTION_SCALES = [1, 1.5, 2, 3];

/** The resolution scales offered on an operating system ("macos", "linux" or "windows"): all. */
export const resolutionScalesFor = (_os) => RESOLUTION_SCALES;

/**
 * Resolution scale "Automatic" stands for on a machine described by `system` (see
 * src-tauri/src/system.rs: { chip, memoryGb, arch }):
 *   - Pro, Max and Ultra chips with at least 16 GB of memory: 2 (more GPU cores and memory
 *     bandwidth);
 *   - anything else, or an unknown machine: 1, the console's own resolution.
 */
export const autoResolutionScale = (system) => {
  if (!system || (system.arch && system.arch !== "aarch64")) return 1;
  if (system.memoryGb != null && system.memoryGb < 16) return 1;
  return /\b(Pro|Max|Ultra)\b/.test(system.chip ?? "") ? 2 : 1;
};

/** render_resolution value of a resolution scale: "2560x1440" for 2. */
export const resolutionValue = (scale) => {
  const { width, height } = renderSize(scale);
  return `${width}x${height}`;
};

/**
 * Scale of a render_resolution value: 2 for "2560x1440", null for "auto" and for anything the
 * game's menu does not offer.
 */
export const scaleOfResolution = (value) => RESOLUTION_SCALES.find((scale) => resolutionValue(scale) === value) ?? null;

/** Machine the launcher runs on, once detected; null until then (and in a browser). */
let detectedSystem = null;

/** Records the detected machine, so "Automatic" resolves for it from then on. */
export const setDetectedSystem = (system) => {
  detectedSystem = system;
};

/** Resolution scale "Automatic" stands for on this machine. */
export const resolveAutoScale = () => autoResolutionScale(detectedSystem);

/** Render size that a resolution scale produces, for display ("2560 × 1440"). */
export const renderSize = (scale) => ({
  width: BASE_RENDER_SIZE.width * scale,
  height: BASE_RENDER_SIZE.height * scale,
});

/**
 * Graphics cards the player can choose from (see src-tauri/src/gpus.rs: { devices: [{ name, uuid,
 * type, vendorId }], automatic }), only those the runtime can be pointed at (with a UUID).
 */
export const choosableGpus = (list) => (list?.devices ?? []).filter((gpu) => gpu.uuid);

/** The choice is offered only with two or more cards: with one there is nothing to choose. */
export const gpuChoiceVisible = (list) => choosableGpus(list).length >= 2;

/**
 * Text of a card: its name, with "dedicated" or "integrated" when the computer has one of each, so
 * two cards of the same make can be told apart. `kindLabels` gives the translated words.
 */
export const gpuLabel = (gpu, kindLabels) => {
  const kind = kindLabels[gpu.type];
  return kind ? `${gpu.name} (${kind})` : gpu.name;
};

/** The stored choice if that card is present, otherwise "auto" (the game chooses). */
export const selectedGpu = (stored, list) =>
  stored && choosableGpus(list).some((gpu) => gpu.uuid === stored) ? stored : "auto";

