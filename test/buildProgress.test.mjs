/*
 * Tests for the runtime build's progress bar.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { PHASE_RANGES, TRANSLATING_MS, createBuildTracker } from "../src/buildProgress.js";

test("every step has its part of the bar, in order, from 0 to 100", () => {
  const ranges = Object.values(PHASE_RANGES);
  assert.equal(ranges[0][0], 0);
  assert.equal(ranges.at(-1)[1], 100);
  for (let i = 1; i < ranges.length; i++) assert.equal(ranges[i][0], ranges[i - 1][1]);
});

test("compiling advances with its files", () => {
  const tracker = createBuildTracker();
  tracker.update({ phase: "compiling", done: 0, total: 200 }, 0);
  assert.equal(tracker.percent(0), 20);
  tracker.update({ phase: "compiling", done: 100, total: 200 }, 1000);
  assert.equal(tracker.percent(1000), 58);
  tracker.update({ phase: "compiling", done: 200, total: 200 }, 2000);
  assert.equal(tracker.percent(2000), 95);
});

test("translating advances with time and never reaches its end", () => {
  const tracker = createBuildTracker();
  tracker.update({ phase: "translating" }, 0);
  assert.equal(tracker.percent(TRANSLATING_MS / 2), 11);
  assert.ok(tracker.percent(10 * TRANSLATING_MS) < 20);
});

test("the time left comes from the pace of the files compiled", () => {
  const tracker = createBuildTracker();
  tracker.update({ phase: "compiling", done: 0, total: 100 }, 0);
  tracker.update({ phase: "compiling", done: 2, total: 100 }, 600);
  assert.equal(tracker.remainingMs(600), null);
  tracker.update({ phase: "compiling", done: 10, total: 100 }, 3000);
  // 300 ms a file, 90 files left, and the link.
  assert.equal(tracker.remainingMs(3000), 90 * 300 + 2000);
});

test("files already up to date do not count for the pace", () => {
  const tracker = createBuildTracker();
  tracker.update({ phase: "compiling", done: 150, total: 200 }, 0);
  tracker.update({ phase: "compiling", done: 160, total: 200 }, 4000);
  assert.equal(tracker.remainingMs(4000), 40 * 400 + 2000);
});
