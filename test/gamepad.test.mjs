/*
 * Tests for controller navigation: where a direction moves the focus, how selects step, and how a
 * held direction repeats.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { REPEAT_DELAY_MS, REPEAT_INTERVAL_MS, createRepeater, nextInDirection, stepOption } from "../src/gamepad.js";
import { TEXT_RULES } from "../src/textRules.js";
import { applyKey } from "../src/virtualKeyboard.js";

const box = (left, top, width = 100, height = 30) => ({ left, top, width, height });

test("a direction moves to the nearest control that way", () => {
  // A row of three buttons, and a wide field below the first two.
  const controls = [box(0, 0), box(120, 0), box(240, 0), box(0, 60, 220)];
  assert.equal(nextInDirection(controls[0], controls, "right"), 1);
  assert.equal(nextInDirection(controls[1], controls, "right"), 2);
  assert.equal(nextInDirection(controls[2], controls, "right"), -1);
  assert.equal(nextInDirection(controls[1], controls, "down"), 3);
  assert.equal(nextInDirection(controls[3], controls, "up"), 0);
  assert.equal(nextInDirection(controls[0], controls, "up"), -1);
});

test("left and right stay in the row", () => {
  const origin = box(0, 0);
  const nextRow = box(200, 40, 300);
  const sameRow = box(600, 10);
  assert.equal(nextInDirection(origin, [nextRow], "right"), -1);
  assert.equal(nextInDirection(origin, [nextRow, sameRow], "right"), 1);
});

test("with anyRow, left and right reach a column whose controls start lower down", () => {
  const origin = box(0, 0);
  const lowerColumn = box(200, 300);
  const behind = box(-200, 300);
  assert.equal(nextInDirection(origin, [lowerColumn, behind], "right"), -1);
  assert.equal(nextInDirection(origin, [lowerColumn, behind], "right", { anyRow: true }), 0);
  assert.equal(nextInDirection(origin, [lowerColumn, behind], "left", { anyRow: true }), 1);
});

test("the focus prefers to stay in its column", () => {
  // Below the origin: one far down in the same column, one nearer but well to the side.
  const origin = box(0, 0);
  const sameColumn = box(0, 200);
  const aside = box(400, 80);
  assert.equal(nextInDirection(origin, [sameColumn, aside], "down"), 0);
});

test("a control beside the focused one is never above or below it", () => {
  const origin = box(0, 0, 100, 40);
  const beside = box(110, 5, 100, 30);
  assert.equal(nextInDirection(origin, [beside], "down"), -1);
  assert.equal(nextInDirection(origin, [beside], "up"), -1);
  assert.equal(nextInDirection(origin, [beside], "right"), 0);
});

test("selects step over disabled options and stop at the ends", () => {
  const options = [{ disabled: false }, { disabled: true }, { disabled: false }];
  assert.equal(stepOption(options, 0, 1), 2);
  assert.equal(stepOption(options, 2, 1), 2);
  assert.equal(stepOption(options, 2, -1), 0);
  assert.equal(stepOption(options, 0, -1), 0);
});

test("a held direction acts at once, then repeats until released", () => {
  let now = 0;
  const pending = [];
  const timers = {
    setTimeout: (fn, ms) => {
      const timer = { at: now + ms, fn };
      pending.push(timer);
      return timer;
    },
    clearTimeout: (timer) => {
      const index = pending.indexOf(timer);
      if (index >= 0) pending.splice(index, 1);
    },
  };
  // Fires the timers due within `ms`, earliest first, each at its own time.
  const advance = (ms) => {
    const end = now + ms;
    for (;;) {
      const due = pending.filter((t) => t.at <= end).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      pending.splice(pending.indexOf(due), 1);
      now = due.at;
      due.fn();
    }
    now = end;
  };
  const acts = [];
  const repeater = createRepeater((direction) => acts.push(direction), timers);
  repeater.press("down");
  assert.deepEqual(acts, ["down"]);
  advance(REPEAT_DELAY_MS - 1);
  assert.equal(acts.length, 1);
  advance(1);
  assert.equal(acts.length, 2);
  advance(REPEAT_INTERVAL_MS * 3);
  assert.equal(acts.length, 5);
  repeater.release("down");
  advance(REPEAT_INTERVAL_MS * 3);
  assert.equal(acts.length, 5);
  repeater.press("left");
  repeater.releaseAll();
  advance(REPEAT_DELAY_MS * 2);
  assert.deepEqual(acts.slice(5), ["left"]);
});

test("the on-screen keyboard types, deletes and keeps to the length limit", () => {
  const options = { shift: false, maxLength: 5 };
  assert.equal(applyKey("", "a", options), "a");
  assert.equal(applyKey("", "a", { ...options, shift: true }), "A");
  assert.equal(applyKey("ab", "7", options), "ab7");
  assert.equal(applyKey("ab", "space", options), "ab ");
  assert.equal(applyKey("abc", "delete", options), "ab");
  assert.equal(applyKey("", "delete", options), "");
  assert.equal(applyKey("abcde", "f", options), "abcde");
  // Accented letters follow the shift state; one whose capital is two letters stays as it is.
  assert.equal(applyKey("", "ñ", { ...options, shift: true }), "Ñ");
  assert.equal(applyKey("", "ß", { ...options, shift: true }), "ß");
});

test("a gamertag holds ASCII letters, digits and single spaces, starting with a letter", () => {
  const gamertag = TEXT_RULES.gamertag;
  assert.equal(gamertag.clean("Marcus Fenix"), "Marcus Fenix");
  assert.equal(gamertag.clean("  3Dom  Santiago/2"), "Dom Santiago2");
  assert.equal(gamertag.clean("Añya Stroud"), "Aya Stroud");
  assert.equal(gamertag.clean("ABCDEFGHIJKLMNOPQ"), "ABCDEFGHIJKLMNO");
  assert.equal(gamertag.accepts("é"), false);
  assert.equal(gamertag.accepts("/"), false);
  // Through the keyboard: a digit cannot start it, and a second space in a row does nothing.
  const keys = { shift: false, maxLength: 15, rule: gamertag };
  assert.equal(applyKey("", "7", keys), "");
  assert.equal(applyKey("Dom ", "space", keys), "Dom ");
  assert.equal(applyKey("Dom", "é", keys), "Dom");
  assert.equal(applyKey("Dom", "2", keys), "Dom2");
});
