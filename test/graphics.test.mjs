/*
 * Tests for the launcher's graphics settings.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  RESOLUTION_SCALES,
  resolutionScalesFor,
  autoResolutionScale,
  choosableGpus,
  gpuChoiceVisible,
  gpuLabel,
  renderSize,
  resolutionValue,
  scaleOfResolution,
  selectedGpu,
} from "../src/graphics.js";

test("render size multiplies the console's 1280x720", () => {
  assert.deepEqual(renderSize(1), { width: 1280, height: 720 });
  assert.deepEqual(renderSize(2), { width: 2560, height: 1440 });
  assert.deepEqual(renderSize(3), { width: 3840, height: 2160 });
  assert.deepEqual(renderSize(1.5), { width: 1920, height: 1080 });
});

test("automatic resolution doubles on Pro, Max and Ultra chips", () => {
  assert.equal(autoResolutionScale({ chip: "Apple M3 Pro", memoryGb: 18, arch: "aarch64" }), 2);
  assert.equal(autoResolutionScale({ chip: "Apple M2 Max", memoryGb: 32, arch: "aarch64" }), 2);
  assert.equal(autoResolutionScale({ chip: "Apple M4", memoryGb: 24, arch: "aarch64" }), 1);
});

test("automatic resolution stays at the console's on small, Intel or unknown machines", () => {
  assert.equal(autoResolutionScale({ chip: "Apple M1 Pro", memoryGb: 8, arch: "aarch64" }), 1);
  assert.equal(autoResolutionScale({ chip: "Intel(R) Core(TM) i7", memoryGb: 32, arch: "x86_64" }), 1);
  assert.equal(autoResolutionScale(null), 1);
  assert.equal(autoResolutionScale({ chip: null, memoryGb: null, arch: "aarch64" }), 1);
});

test("every offered resolution is one the automatic choice can produce or larger", () => {
  assert.ok(RESOLUTION_SCALES.includes(1) && RESOLUTION_SCALES.includes(2));
});

test("resolutions are written as the game's Video menu names them", () => {
  assert.deepEqual(RESOLUTION_SCALES.map(resolutionValue), [
    "1280x720",
    "1920x1080",
    "2560x1440",
    "3840x2160",
  ]);
  assert.equal(scaleOfResolution("2560x1440"), 2);
  assert.equal(scaleOfResolution("1920x1080"), 1.5);
  assert.equal(scaleOfResolution("auto"), null);
  assert.equal(scaleOfResolution("1600x900"), null);
  assert.equal(scaleOfResolution(undefined), null);
});

test("1920 x 1080 is offered on every operating system", () => {
  assert.deepEqual(resolutionScalesFor("macos"), [1, 1.5, 2, 3]);
  assert.deepEqual(resolutionScalesFor("linux"), [1, 1.5, 2, 3]);
  assert.deepEqual(resolutionScalesFor("windows"), [1, 1.5, 2, 3]);
  assert.deepEqual(resolutionScalesFor(undefined), [1, 1.5, 2, 3]);
});

const hybrid = {
  devices: [
    { name: "AMD Radeon 780M", uuid: "aaaa", type: "integrated", vendorId: 0x1002 },
    { name: "NVIDIA GeForce RTX 4060 Laptop GPU", uuid: "bbbb", type: "discrete", vendorId: 0x10de },
  ],
  automatic: "bbbb",
};

test("the graphics card choice is offered only with two or more cards", () => {
  assert.equal(gpuChoiceVisible(hybrid), true);
  assert.equal(gpuChoiceVisible({ devices: [hybrid.devices[0]], automatic: "aaaa" }), false);
  assert.equal(gpuChoiceVisible({ devices: [], automatic: null }), false);
  assert.equal(gpuChoiceVisible(null), false);
});

test("cards without a UUID cannot be chosen", () => {
  const list = { devices: [...hybrid.devices, { name: "llvmpipe", uuid: "", type: "cpu", vendorId: 0x10005 }] };
  assert.deepEqual(
    choosableGpus(list).map((gpu) => gpu.uuid),
    ["aaaa", "bbbb"],
  );
});

test("a card is named with its kind when known", () => {
  const kinds = { discrete: "dedicada", integrated: "integrada" };
  assert.equal(gpuLabel(hybrid.devices[1], kinds), "NVIDIA GeForce RTX 4060 Laptop GPU (dedicada)");
  assert.equal(gpuLabel({ name: "Virtual GPU", uuid: "cccc", type: "virtual" }, kinds), "Virtual GPU");
});

test("a stored card that is gone shows the automatic choice", () => {
  assert.equal(selectedGpu("aaaa", hybrid), "aaaa");
  assert.equal(selectedGpu("cccc", hybrid), "auto");
  assert.equal(selectedGpu(null, hybrid), "auto");
});

