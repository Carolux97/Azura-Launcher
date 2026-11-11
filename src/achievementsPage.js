/*
 * Builds the Achievements section from what platform.getAchievements returns (see achievements.js
 * for the shape of an achievement). main.js keeps the state (the data, the view and the filter) and
 * calls renderAchievementsPage again after every change and every language change.
 *
 * The section shows a banner once every achievement is unlocked, then two columns: on the left the
 * Embry Star card (the badge, the gamerscore earned out of the total with its bar, and the count),
 * and under it, in the view by acts, what comes next in the story; on the right the filters and the
 * achievements in the chosen view: a list, a grid of badges, or sections by act. Without the game
 * the page shows dimmed generic medals behind a card with a lock, a message and a button to the
 * Installation view.
 *
 * Every text from the game goes in through textContent, never as HTML.
 */

import { achievementSections, achievementState, filterCounts, gamerscorePercent, matchesFilter, nextStoryAchievements } from "./achievements.js";

const SVG_NS = "http://www.w3.org/2000/svg";

/** Creates an element with a class and children (strings become text). */
const el = (tag, className = "", ...children) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  for (const child of children) {
    if (child == null || child === false) continue;
    node.append(typeof child === "string" ? document.createTextNode(child) : child);
  }
  return node;
};

/** Creates an SVG element from markup the launcher owns (never from game data). */
const svg = (markup) => {
  const template = document.createElementNS(SVG_NS, "svg");
  template.innerHTML = markup;
  return template.firstElementChild;
};

/** Points of a star with `points` tips around (cx, cy). */
const starPoints = (cx, cy, outer, inner, points) =>
  Array.from({ length: points * 2 }, (_, k) => {
    const r = k % 2 === 0 ? outer : inner;
    const angle = (k * Math.PI) / points - Math.PI / 2;
    return `${(cx + r * Math.cos(angle)).toFixed(1)},${(cy + r * Math.sin(angle)).toFixed(1)}`;
  }).join(" ");

/** The generic medal, for an achievement without artwork and for the page without the game. */
const GENERIC_MEDAL = `<svg viewBox="0 0 56 56" aria-hidden="true"><circle cx="28" cy="28" r="20" fill="none" class="medal-line" stroke-width=".8" opacity=".5"/><polygon points="${starPoints(28, 28, 13, 6, 6)}" class="medal-fill"/><circle cx="28" cy="28" r="3.2" fill="#121416"/></svg>`;

/** The medal of a hidden achievement still locked: a question mark instead of its art. */
const HIDDEN_MEDAL = `<svg viewBox="0 0 56 56" aria-hidden="true"><text x="28" y="37" text-anchor="middle" class="medal-fill medal-question">?</text></svg>`;

/** The padlock of the navigation entry and of the locked page. */
export const LOCK_ICON = `<svg class="lock" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 10V8a5 5 0 0 1 10 0v2" fill="none" stroke="currentColor" stroke-width="2"/><rect x="5" y="10" width="14" height="10" rx="1.5" fill="currentColor"/><circle cx="12" cy="15" r="1.7" fill="#121416"/></svg>`;

/** The Embry Star badge: a cog in platinum (a placeholder design, still to be refined). */
const EMBRY_STAR = (() => {
  const teeth = Array.from({ length: 12 }, (_, k) =>
    `<rect x="56" y="6" width="8" height="14" rx="1.5" transform="rotate(${k * 30} 60 60)" fill="url(#embry-metal)"/>`,
  ).join("");
  return `<svg class="embry-star" viewBox="0 0 120 120" aria-hidden="true"><defs><linearGradient id="embry-metal" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#f3f1ea"/><stop offset=".55" stop-color="#b9bcc4"/><stop offset="1" stop-color="#7d828c"/></linearGradient></defs><g opacity=".95">${teeth}<circle cx="60" cy="60" r="40" fill="#1b1d21" stroke="url(#embry-metal)" stroke-width="6"/><circle cx="60" cy="60" r="30" fill="none" stroke="url(#embry-metal)" stroke-width="1.5" opacity=".6"/><circle cx="60" cy="60" r="14" fill="none" stroke="url(#embry-metal)" stroke-width="5"/><circle class="embry-core" cx="60" cy="60" r="5" fill="#ffb81c"/></g></svg>`;
})();

/** An achievement's state for its classes (see achievementState). */
const stateOf = achievementState;

/** The round medal of an achievement: its artwork, the generic medal, or the question mark. */
const medal = (achievement, size) => {
  const node = el("span", `medal is-${stateOf(achievement)}`);
  node.style.setProperty("--medal-size", `${size}px`);
  if (achievement.icon) {
    const image = el("img");
    image.src = achievement.icon;
    image.alt = "";
    // Artwork the webview cannot decode falls back to the generic medal.
    image.addEventListener("error", () => image.replaceWith(svg(GENERIC_MEDAL)), { once: true });
    node.append(image);
  } else {
    node.append(svg(stateOf(achievement) === "hidden" ? HIDDEN_MEDAL : GENERIC_MEDAL));
  }
  return node;
};

/** Gamerscore as "20 G" pieces: the number, and a smaller G. Hidden ones show "?". */
const gamerscore = (achievement, className) =>
  el("span", className, achievement.gamerscore == null ? "?" : String(achievement.gamerscore), el("small", "", "G"));

/** The name, description and date of an achievement, in the reader's language. */
const texts = (achievement, ctx) => ({
  name: achievement.name ?? ctx.t("achievements.hiddenName"),
  description: achievement.description ?? ctx.t("achievements.hiddenDescription"),
});

const formatDate = (ms, ctx) => new Intl.DateTimeFormat(ctx.locale, { dateStyle: "medium" }).format(new Date(ms));

/** One row of the list (the list view and the sections of the view by acts). */
const listRow = (achievement, ctx, compact = false) => {
  const { name, description } = texts(achievement, ctx);
  const row = el(
    "li",
    `ach is-${stateOf(achievement)}`,
    medal(achievement, compact ? 40 : 56),
    el("div", "ach-text", el("div", "ach-name", name), el("div", "ach-desc", description)),
    gamerscore(achievement, "ach-g"),
  );
  if (achievement.unlocked && achievement.unlockedAt && !compact) {
    row.append(el("span", "ach-when", ctx.t("achievements.unlockedOn"), el("b", "", formatDate(achievement.unlockedAt, ctx))));
  }
  // Focusable, so the keyboard and a controller go through the list (and scroll it) row by row.
  row.tabIndex = 0;
  return row;
};

/** Delay before the card of a badge shows, so moving the mouse across the grid does not flicker. */
const TOOLTIP_DELAY_MS = 150;

/** Gap between a badge and its card, and the least room kept to the window's edges. */
const TOOLTIP_GAP = 10;
const TOOLTIP_MARGIN = 8;

/**
 * The card that shows a badge's name, description, gamerscore and date over the grid. There is one
 * for the whole page, in <body>: the tiles are clipped to their cut corner and the views animate in
 * with a transform, either of which would clip or misplace a card inside them.
 */
const tooltip = { node: null, timer: 0 };

const tooltipNode = () => {
  if (!tooltip.node) {
    tooltip.node = el("div", "ach-tooltip");
    tooltip.node.id = "achievement-tooltip";
    tooltip.node.setAttribute("role", "tooltip");
    tooltip.node.hidden = true;
    document.body.append(tooltip.node);
    // The card stays where the badge was; scrolling moves the badge away, so it goes.
    document.querySelector(".content")?.addEventListener("scroll", hideAchievementTooltip, { passive: true });
  }
  return tooltip.node;
};

/** Hides the card of a badge, and cancels one about to show. */
export const hideAchievementTooltip = () => {
  clearTimeout(tooltip.timer);
  if (!tooltip.node || tooltip.node.hidden) return;
  tooltip.node.hidden = true;
  document.querySelector('[aria-describedby="achievement-tooltip"]')?.removeAttribute("aria-describedby");
};

/**
 * Shows the card of `achievement` over its tile after TOOLTIP_DELAY_MS: above it, centred, kept
 * inside the window, or below it when there is no room above.
 */
const showTooltip = (tile, achievement, ctx) => {
  clearTimeout(tooltip.timer);
  tooltip.timer = setTimeout(() => {
    const { name, description } = texts(achievement, ctx);
    const state = stateOf(achievement);
    const status = state === "unlocked"
      ? [ctx.t("achievements.unlockedOn"), achievement.unlockedAt ? el("b", "", formatDate(achievement.unlockedAt, ctx)) : null]
      : [ctx.t(state === "hidden" ? "achievements.hiddenState" : "achievements.locked")];
    const node = tooltipNode();
    node.className = `ach-tooltip is-${state}`;
    node.replaceChildren(
      el("div", "ach-tooltip-head", el("span", "ach-tooltip-name", name), gamerscore(achievement, "ach-tooltip-g")),
      el("p", "ach-tooltip-desc", description),
      el("div", "ach-tooltip-status", ...status),
    );
    node.hidden = false;
    const box = tile.getBoundingClientRect();
    const card = node.getBoundingClientRect();
    const left = Math.min(
      Math.max(TOOLTIP_MARGIN, box.left + box.width / 2 - card.width / 2),
      window.innerWidth - card.width - TOOLTIP_MARGIN,
    );
    const above = box.top - card.height - TOOLTIP_GAP;
    const below = above < TOOLTIP_MARGIN;
    node.classList.toggle("is-below", below);
    node.style.left = `${left}px`;
    node.style.top = `${below ? box.bottom + TOOLTIP_GAP : above}px`;
    // The arrow points at the middle of the badge, wherever the card had to move.
    node.style.setProperty("--arrow-x", `${box.left + box.width / 2 - left}px`);
    tile.setAttribute("aria-describedby", node.id);
  }, TOOLTIP_DELAY_MS);
};

/**
 * One tile of the grid of badges. Hovering it, or reaching it with the keyboard, shows its card
 * (see showTooltip).
 */
const badgeTile = (achievement, ctx) => {
  const { name } = texts(achievement, ctx);
  const tile = el(
    "li",
    `tile is-${stateOf(achievement)}`,
    medal(achievement, 64),
    // The name is read by screen readers; the card over the badge shows it to everyone else.
    el("div", "tile-name visually-hidden", name),
    el("div", "tile-g", achievement.gamerscore == null ? "? G" : `${achievement.gamerscore} G`),
  );
  tile.tabIndex = 0;
  tile.addEventListener("mouseenter", () => showTooltip(tile, achievement, ctx));
  tile.addEventListener("focus", () => showTooltip(tile, achievement, ctx));
  tile.addEventListener("mouseleave", hideAchievementTooltip);
  tile.addEventListener("blur", hideAchievementTooltip);
  return tile;
};

/**
 * The Embry Star card, beside every view: the badge (dimmed until every achievement is unlocked,
 * with rays and a shine once it is earned), what earns it, "250 / 1000 G" with its bar, and the
 * count.
 */
const embryCard = (data, ctx) => {
  const earned = Boolean(data.completedAt) || (data.achievements.length > 0 && data.unlockedCount === data.achievements.length);
  const bar = el("div", "embry-bar", el("span"));
  bar.firstElementChild.style.width = `${earned ? 100 : gamerscorePercent(data.earnedGamerscore, data.totalGamerscore)}%`;
  const badge = el("div", `embry-badge${earned ? "" : " is-dim"}`);
  badge.append(svg(EMBRY_STAR));
  return el(
    "div",
    `panel embry-card${earned ? " is-earned" : ""}`,
    badge,
    el("h2", "embry-title", ctx.t(earned ? "achievements.embryEarned" : "achievements.embryStar")),
    el("p", "help embry-hint", ctx.t(earned ? "achievements.embryEarnedHint" : "achievements.embryHint")),
    el("div", "embry-g", el("b", "", String(data.earnedGamerscore)), ` / ${data.totalGamerscore} `, el("small", "", "G")),
    bar,
    el("div", "embry-count", ctx.t("achievements.count", { unlocked: data.unlockedCount, total: data.achievements.length })),
  );
};

/** The filter buttons, each with how many achievements it shows; `onFilter(name)` runs when one is chosen. */
const filterBar = (data, filter, ctx) => {
  const counts = filterCounts(data.achievements);
  const bar = el("div", "segmented ach-filters");
  bar.setAttribute("role", "group");
  for (const [name, key] of [
    ["all", "achievements.filterAll"],
    ["unlocked", "achievements.filterUnlocked"],
    ["locked", "achievements.filterLocked"],
    ["hidden", "achievements.filterHidden"],
  ]) {
    const chip = el("button", filter === name ? "is-on" : "", ctx.t(key), el("small", "", String(counts[name])));
    chip.type = "button";
    chip.setAttribute("aria-pressed", String(filter === name));
    chip.addEventListener("click", () => ctx.onFilter(name));
    bar.append(chip);
  }
  return bar;
};

/** Text shown when a filter leaves nothing to show. */
const emptyNote = (ctx) => el("p", "help ach-empty", ctx.t("achievements.noneHere"));

/**
 * Every view's layout: the Embry Star card (and, in the view by acts, what comes next) in a column
 * on the left, and the filters with the view's own content in a card on the right.
 */
const layout = (data, filter, ctx, content, sideExtra = null) =>
  el(
    "div",
    "ach-columns",
    el("div", "ach-side", embryCard(data, ctx), sideExtra),
    el("div", "panel ach-main", filterBar(data, filter, ctx), content),
  );

/** The list view. */
const listView = (data, filter, ctx) => {
  const items = data.achievements.filter((achievement) => matchesFilter(achievement, filter));
  const list = el("ul", "ach-list");
  items.forEach((achievement) => list.append(listRow(achievement, ctx)));
  return [layout(data, filter, ctx, items.length ? list : emptyNote(ctx))];
};

/** The grid of badges. */
const badgesView = (data, filter, ctx) => {
  const items = data.achievements.filter((achievement) => matchesFilter(achievement, filter));
  const grid = el("ul", "ach-grid");
  items.forEach((achievement) => grid.append(badgeTile(achievement, ctx)));
  return [layout(data, filter, ctx, items.length ? grid : emptyNote(ctx))];
};

/** The view by acts: one card per section, and what comes next in the story under the Embry Star. */
const actsView = (data, filter, ctx) => {
  const sections = achievementSections(data.achievements, filter);
  const main = el("div", "ach-sections");
  for (const section of sections) {
    const bar = el("div", "embry-bar is-thin", el("span"));
    bar.firstElementChild.style.width = `${section.total ? (section.unlocked / section.total) * 100 : 0}%`;
    const list = el("ul", "ach-list");
    section.achievements.forEach((achievement) => list.append(listRow(achievement, ctx)));
    main.append(
      el(
        "div",
        "ach-section",
        el(
          "div",
          "ach-section-head",
          el(
            "div",
            "",
            el("h2", "panel-title", ctx.t(`achievements.section.${section.key}`)),
            section.chapters ? el("p", "help", ctx.t("achievements.chapters", { count: section.chapters })) : null,
          ),
          el("span", "ach-section-count", `${section.unlocked} / ${section.total}`),
        ),
        bar,
        list,
      ),
    );
  }
  if (!sections.length) main.append(emptyNote(ctx));

  const next = nextStoryAchievements(data.achievements);
  let nextPanel = null;
  if (next.length) {
    const list = el("ul", "ach-list is-compact");
    next.forEach((achievement) => list.append(listRow(achievement, ctx, true)));
    nextPanel = el("div", "panel", el("h2", "panel-title", ctx.t("achievements.next")), el("p", "help", ctx.t("achievements.nextHelp")), list);
  }
  return [layout(data, filter, ctx, main, nextPanel)];
};

/**
 * The page without the game, or while its achievements cannot be read: dimmed generic medals
 * behind a card with a lock, a message and, when there is something to do, a button to the
 * Installation view.
 */
const lockedView = (data, ctx) => {
  const ghost = el("ul", "ach-list");
  for (let index = 0; index < 6; index++) {
    ghost.append(
      el(
        "li",
        "ach is-locked",
        medal({ unlocked: false, hidden: false }, 56),
        el("div", "ach-text", el("div", "skeleton"), el("div", "skeleton is-small")),
        el("div", "skeleton is-g"),
      ),
    );
    // Lines of different lengths, like real names and descriptions.
    const [nameLine, descriptionLine] = ghost.lastElementChild.querySelectorAll(".skeleton");
    nameLine.style.width = `${60 - index * 5}%`;
    descriptionLine.style.width = `${30 + ((index * 7) % 20)}%`;
  }
  const key = { noRuntime: "achievements.lockedNoRuntime", unavailable: "achievements.lockedUnavailable", loading: "achievements.loading" }[data?.status] ?? "achievements.lockedNoGame";
  const gate = el("div", "panel ach-gate");
  gate.append(svg(LOCK_ICON), el("h2", "panel-title", ctx.t(data?.status === "loading" ? "achievements.loadingTitle" : "achievements.lockedTitle")), el("p", "help", ctx.t(key)));
  if (data?.status === "unavailable" && data.detail) gate.append(el("p", "ach-detail", data.detail));
  if (data?.status === "noGame" || data?.status === "noRuntime" || !data) {
    const button = el("button", "btn btn-primary", ctx.t("achievements.goToInstall"));
    button.type = "button";
    button.addEventListener("click", ctx.onGoToInstall);
    gate.append(button);
  }
  return [el("div", "ach-stage", el("div", "panel ach-ghost", ghost), gate)];
};

/**
 * Fills `root` with the section for `data` (or the locked page when it is not "ok"), in `view`
 * ("list", "badges" or "acts") under `filter`. `ctx` holds `t`, `locale`, and the handlers
 * `onFilter(name)` and `onGoToInstall()`. Returns whether the achievements are shown (false for the
 * locked page), which main.js uses for the lock mark of the navigation entry.
 */
export const renderAchievementsPage = (root, data, view, filter, ctx) => {
  // The tile a card belongs to is about to be replaced.
  hideAchievementTooltip();
  const ok = data?.status === "ok";
  let parts;
  if (!ok) parts = lockedView(data, ctx);
  else if (view === "badges") parts = badgesView(data, filter, ctx);
  else if (view === "acts") parts = actsView(data, filter, ctx);
  else parts = listView(data, filter, ctx);
  root.replaceChildren(...parts);
  return ok;
};

/** The banner shown once every achievement is unlocked, or null. */
export const completedBanner = (data, ctx) => {
  if (data?.status !== "ok" || !data.completedAt) return null;
  const details = [
    ctx.t("achievements.completedOn", { date: formatDate(data.completedAt, ctx) }),
    ctx.t("achievements.count", { unlocked: data.unlockedCount, total: data.achievements.length }),
    `${data.totalGamerscore} G`,
  ].join(" · ");
  return el("div", "embry-banner", ctx.t("achievements.embryBanner"), el("small", "", details));
};
