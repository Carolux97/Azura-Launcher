/*
 * Tests for the game language helpers.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { defaultGameLanguage, gameLanguageName } from "../src/gameLanguages.js";

/** Languages of the World edition dump (media ID 58122AD6), as found in its Coalesced bundles. */
const WORLD_EDITION = ["CHN", "CZE", "FRA", "HUN", "INT", "KOR", "POL", "PTB", "RUS"];
/** Languages of the España edition dump (media ID 6EF14DF9). */
const SPAIN_EDITION = ["ESN", "INT"];
/** Languages of the Latin American edition dump (media ID 02EEE060). */
const LATAM_EDITION = ["ESM", "INT", "PTB"];

test("the launcher's language is preselected when the game has it", () => {
  assert.equal(defaultGameLanguage(WORLD_EDITION, "fr"), "FRA");
  assert.equal(defaultGameLanguage(WORLD_EDITION, "zh-Hant"), "CHN");
});

test("Spanish preselects whichever Spanish variant the game has", () => {
  assert.equal(defaultGameLanguage(SPAIN_EDITION, "es"), "ESN");
  assert.equal(defaultGameLanguage(LATAM_EDITION, "es"), "ESM");
  assert.equal(defaultGameLanguage(["ESM", "ESN", "INT"], "es"), "ESN");
});

test("English is preselected when the game lacks the launcher's language", () => {
  assert.equal(defaultGameLanguage(WORLD_EDITION, "es"), "INT");
});

test("the first language is used when the game has neither", () => {
  assert.equal(defaultGameLanguage(["RUS", "POL"], "es"), "RUS");
});

test("no languages means no default", () => {
  assert.equal(defaultGameLanguage([], "en"), null);
  assert.equal(defaultGameLanguage(undefined, "en"), null);
});

test("language codes are named, and unknown codes are shown as they are", () => {
  assert.equal(gameLanguageName("ESN"), "Español (España)");
  assert.equal(gameLanguageName("XYZ"), "XYZ");
});
