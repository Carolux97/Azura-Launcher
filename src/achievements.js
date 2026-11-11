/*
 * What the Achievements section needs beyond the data platform.getAchievements returns: the ways to
 * see them, the filters, and the sections of the view by acts.
 *
 * An achievement, as src-tauri/src/achievements.rs sends it:
 *   { id, name, description, gamerscore, hidden, unlocked, unlockedAt, icon }
 * `name`, `description`, `gamerscore` and `icon` are null for a hidden achievement that is still
 * locked, and `icon` is null too when the game has no artwork for it (the generic medal shows).
 */

/** The ways to see the achievements; the first is the default. */
export const ACHIEVEMENT_VIEWS = ["list", "badges", "acts"];

/** The filters, the same in every view. */
export const ACHIEVEMENT_FILTERS = ["all", "unlocked", "locked", "hidden"];

/**
 * Sections of the view by acts, in order. The story achievements say in the game's own text which
 * act and chapter unlock them ("Story progress in Act 1, Chapter 2"). The secret one (19, "Respect
 * for the Dead") is earned in Act 4, Chapter 1, where Griffin's weapon stash is, so it opens that
 * act. The others belong to no act: difficulty, collectibles and co-op go under the campaign, and
 * levels, Versus, Horde and Beast under the other modes. RAAM's Shadow's achievements (56 to 65,
 * sent only when the add-on is installed) have a section of their own, which shows only then.
 * `chapters` is how many chapters the act (or the add-on) has.
 */
export const ACHIEVEMENT_SECTIONS = [
  { key: "prologue", ids: [1] },
  { key: "act1", chapters: 6, ids: [2, 3, 4, 5] },
  { key: "act2", chapters: 7, ids: [6, 7, 8] },
  { key: "act3", chapters: 5, ids: [9, 10, 11, 12] },
  { key: "act4", chapters: 6, ids: [19, 13, 14, 15] },
  { key: "act5", chapters: 6, ids: [16, 17, 18] },
  { key: "campaign", ids: [20, 21, 22, 23, 24, 25, 26, 27, 28] },
  { key: "raamsShadow", chapters: 5, ids: [56, 57, 58, 59, 60, 61, 62, 63, 64, 65] },
  { key: "modes", ids: [29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50] },
];

/** Story achievements, in the order the campaign unlocks them. */
const STORY_IDS = ACHIEVEMENT_SECTIONS.filter(({ key }) => key === "prologue" || key.startsWith("act")).flatMap(({ ids }) => ids);

/** The view to show for a stored choice: the stored one when it exists, else the list. */
export const achievementView = (stored) => (ACHIEVEMENT_VIEWS.includes(stored) ? stored : ACHIEVEMENT_VIEWS[0]);

/**
 * What an achievement is for the filters: "unlocked", "hidden" (hidden by the game and still
 * locked) or "locked" (any other locked one). The three never overlap, so their counts add up to
 * the total, and a hidden achievement moves to "unlocked" once it is revealed.
 */
export const achievementState = (achievement) => (achievement.unlocked ? "unlocked" : achievement.hidden ? "hidden" : "locked");

/** Whether an achievement shows under a filter ("all" or one of the states). */
export const matchesFilter = (achievement, filter) => filter === "all" || achievementState(achievement) === filter;

/** How many achievements show under each filter: { all, unlocked, locked, hidden }. */
export const filterCounts = (achievements) =>
  Object.fromEntries(ACHIEVEMENT_FILTERS.map((filter) => [filter, achievements.filter((a) => matchesFilter(a, filter)).length]));

/**
 * The sections of the view by acts that have something under `filter`, each with its achievements
 * (in the section's order) and how many of them are unlocked. An achievement the table does not
 * name (another build of the game) goes to the last section, so none is ever left out.
 */
export const achievementSections = (achievements, filter = "all") => {
  const byId = new Map(achievements.map((achievement) => [achievement.id, achievement]));
  const named = new Set(ACHIEVEMENT_SECTIONS.flatMap(({ ids }) => ids));
  const unnamed = achievements.filter(({ id }) => !named.has(id));
  return ACHIEVEMENT_SECTIONS.map((section, index) => {
    const all = section.ids.map((id) => byId.get(id)).filter(Boolean);
    if (index === ACHIEVEMENT_SECTIONS.length - 1) all.push(...unnamed);
    return {
      key: section.key,
      chapters: section.chapters ?? null,
      unlocked: all.filter((achievement) => achievement.unlocked).length,
      total: all.length,
      achievements: all.filter((achievement) => matchesFilter(achievement, filter)),
    };
  }).filter((section) => section.achievements.length > 0);
};

/**
 * The next story achievements still locked, in campaign order (the view by acts suggests them). A
 * hidden one is left out, so the suggestion does not give away that a secret is near.
 */
export const nextStoryAchievements = (achievements, count = 2) => {
  const byId = new Map(achievements.map((achievement) => [achievement.id, achievement]));
  return STORY_IDS.map((id) => byId.get(id))
    .filter((achievement) => achievement && !achievement.unlocked && !achievement.hidden)
    .slice(0, count);
};

/** Share of the gamerscore earned, from 0 to 100 (whole numbers, never 100 before it is all). */
export const gamerscorePercent = (earned, total) => {
  if (!total) return 0;
  return earned >= total ? 100 : Math.min(99, Math.floor((earned / total) * 100));
};
