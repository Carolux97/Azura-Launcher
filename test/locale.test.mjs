/*
 * Tests for the launcher's locale logic and message files.
 *
 * Run with: node --test test
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  FALLBACK_LOCALE,
  SUPPORTED_LOCALES,
  flattenKeys,
  interpolate,
  lookup,
  matchLocale,
  resolveLocale,
} from "../src/locale.js";

const readLocale = (code) =>
  JSON.parse(readFileSync(new URL(`../src/locales/${code}.json`, import.meta.url), "utf8"));

test("regional variants map to their base language", () => {
  assert.equal(matchLocale("es-ES"), "es");
  assert.equal(matchLocale("es-MX"), "es");
  assert.equal(matchLocale("pt-PT"), "pt");
  assert.equal(matchLocale("pt-BR"), "pt");
  assert.equal(matchLocale("en-GB"), "en");
  assert.equal(matchLocale("de"), "de");
});

test("only Traditional Chinese matches zh-Hant", () => {
  assert.equal(matchLocale("zh-TW"), "zh-Hant");
  assert.equal(matchLocale("zh-HK"), "zh-Hant");
  assert.equal(matchLocale("zh-Hant"), "zh-Hant");
  assert.equal(matchLocale("zh-CN"), null);
  assert.equal(matchLocale("zh-Hans"), null);
});

test("unsupported and invalid tags do not match", () => {
  assert.equal(matchLocale("nl-NL"), null);
  assert.equal(matchLocale("not a tag"), null);
});

test("the first supported preferred language wins", () => {
  assert.equal(resolveLocale(["nl-NL", "fr-CA", "en-US"]), "fr");
  assert.equal(resolveLocale(["zh-CN", "ja-JP"]), "ja");
});

test("English is the fallback when nothing matches", () => {
  assert.equal(resolveLocale(["nl-NL", "zh-CN"]), FALLBACK_LOCALE);
  assert.equal(resolveLocale([]), FALLBACK_LOCALE);
  assert.equal(resolveLocale(undefined), FALLBACK_LOCALE);
});

test("interpolate fills known placeholders and keeps unknown ones visible", () => {
  assert.equal(interpolate("Launcher {version}", { version: "0.1.0" }), "Launcher 0.1.0");
  assert.equal(interpolate("Press {key}", {}), "Press {key}");
});

test("lookup reads dotted keys and returns undefined for gaps", () => {
  const messages = { nav: { play: "Play" } };
  assert.equal(lookup(messages, "nav.play"), "Play");
  assert.equal(lookup(messages, "nav.missing"), undefined);
  assert.equal(lookup(messages, "missing.deep.key"), undefined);
});

// Every message file must match English key for key, and keep the same placeholders, so no
// language shows a raw key or loses the {file} / {key} / {name} / {version} values.
const english = readLocale(FALLBACK_LOCALE);
const englishKeys = flattenKeys(english).sort();
const placeholdersOf = (text) => (text.match(/\{\w+\}/g) ?? []).sort();

for (const { code } of SUPPORTED_LOCALES.filter(({ code }) => code !== FALLBACK_LOCALE)) {
  test(`${code}.json has the same keys and placeholders as en.json`, () => {
    const messages = readLocale(code);
    assert.deepEqual(flattenKeys(messages).sort(), englishKeys);
    for (const key of englishKeys) {
      const text = lookup(messages, key);
      assert.equal(typeof text, "string", `${code}: ${key} is not a string`);
      assert.notEqual(text.trim(), "", `${code}: ${key} is empty`);
      assert.deepEqual(placeholdersOf(text), placeholdersOf(lookup(english, key)), `${code}: ${key}`);
    }
  });
}
