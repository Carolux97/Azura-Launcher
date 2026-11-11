/*
 * Tests for the launcher's update button.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { canStartUpdate, updateButtonState } from "../src/update.js";

const update = { version: "0.6.0" };

test("no update: the button is hidden", () => {
  assert.deepEqual(updateButtonState(null, null, "idle"), { hidden: true, disabled: true });
  assert.equal(canStartUpdate(null, null, "idle"), false);
});

test("an update with the game closed can start", () => {
  assert.deepEqual(updateButtonState(update, null, "idle"), {
    hidden: false,
    disabled: false,
    key: "update.available",
    vars: { version: "0.6.0" },
  });
  assert.equal(canStartUpdate(update, null, "idle"), true);
});

test("while the game starts or runs the button waits and says why", () => {
  for (const game of ["starting", "running", "preparing"]) {
    const view = updateButtonState(update, null, game);
    assert.equal(view.hidden, false);
    assert.equal(view.disabled, true);
    assert.equal(view.hint, "update.closeGame");
    assert.equal(canStartUpdate(update, null, game), false);
  }
});

test("while downloading the button shows the progress and cannot start again", () => {
  assert.deepEqual(updateButtonState(update, 42, "idle"), {
    hidden: false,
    disabled: true,
    key: "update.downloading",
    vars: { percent: 42 },
  });
  assert.equal(updateButtonState(update, -1, "idle").vars.percent, -1);
  assert.equal(canStartUpdate(update, 42, "idle"), false);
});
