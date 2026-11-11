/*
 * Tests for the Achievements section's views, filters and sections.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  ACHIEVEMENT_SECTIONS,
  achievementSections,
  achievementView,
  filterCounts,
  gamerscorePercent,
  matchesFilter,
  nextStoryAchievements,
} from "../src/achievements.js";

/** The game's 50 achievements, with `unlocked` ones unlocked and 19 hidden, as the runtime sends. */
const achievements = (unlocked = []) =>
  Array.from({ length: 50 }, (_, index) => {
    const id = index + 1;
    return { id, hidden: id === 19, unlocked: unlocked.includes(id), gamerscore: 10 };
  });

test("the list is the view when none, or an unknown one, is stored", () => {
  assert.equal(achievementView(undefined), "list");
  assert.equal(achievementView("tiles"), "list");
  assert.equal(achievementView("acts"), "acts");
});

test("the filters never overlap, so their counts add up to the total", () => {
  assert.deepEqual(filterCounts(achievements([1, 2])), { all: 50, unlocked: 2, locked: 47, hidden: 1 });
  // Once unlocked, the hidden achievement is revealed: it counts as unlocked only.
  const revealed = achievements([1, 2, 19]);
  assert.deepEqual(filterCounts(revealed), { all: 50, unlocked: 3, locked: 47, hidden: 0 });
  assert.ok(matchesFilter(revealed[18], "unlocked") && !matchesFilter(revealed[18], "hidden"));
});

test("every achievement of the game and of RAAM's Shadow is in exactly one section", () => {
  const ids = ACHIEVEMENT_SECTIONS.flatMap((section) => section.ids).sort((a, b) => a - b);
  const raamsShadow = Array.from({ length: 10 }, (_, index) => index + 56);
  assert.deepEqual(ids, [...Array.from({ length: 50 }, (_, index) => index + 1), ...raamsShadow]);
});

test("RAAM's Shadow's section shows only with its achievements", () => {
  assert.ok(!achievementSections(achievements()).some(({ key }) => key === "raamsShadow"));
  const withDlc = [...achievements(), ...[56, 61].map((id) => ({ id, hidden: false, unlocked: id === 56, gamerscore: 25 }))];
  const section = achievementSections(withDlc).find(({ key }) => key === "raamsShadow");
  assert.deepEqual([section.unlocked, section.total, section.chapters], [1, 2, 5]);
});

test("sections count their unlocks and drop the ones the filter empties", () => {
  const sections = achievementSections(achievements([2, 3]), "unlocked");
  assert.deepEqual(sections.map(({ key, unlocked, total }) => [key, unlocked, total]), [["act1", 2, 4]]);
  assert.equal(achievementSections(achievements(), "all")[1].chapters, 6);
});

test("an achievement the table does not know goes to the last section", () => {
  const sections = achievementSections([...achievements(), { id: 99, hidden: false, unlocked: false }]);
  assert.equal(sections.at(-1).achievements.at(-1).id, 99);
});

test("the next story achievements follow the campaign and keep the secret one out", () => {
  assert.deepEqual(nextStoryAchievements(achievements([1, 2, 4])).map(({ id }) => id), [3, 5]);
  const beforeAct4 = achievements(Array.from({ length: 12 }, (_, i) => i + 1));
  assert.deepEqual(nextStoryAchievements(beforeAct4).map(({ id }) => id), [13, 14]);
  assert.deepEqual(nextStoryAchievements(achievements(Array.from({ length: 18 }, (_, i) => i + 1))), []);
});

test("the gamerscore share reaches 100 only when all of it is earned", () => {
  assert.equal(gamerscorePercent(30, 1000), 3);
  assert.equal(gamerscorePercent(999, 1000), 99);
  assert.equal(gamerscorePercent(1000, 1000), 100);
  assert.equal(gamerscorePercent(0, 0), 0);
});
