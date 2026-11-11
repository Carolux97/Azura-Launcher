/*
 * Alboria's notices and the state of the service, as the launcher shows them
 * (alboria-notes:azura/alboria-relay.md, section 4.1): the notice for everyone and the game's own
 * notice, countdowns before scheduled actions, maintenance, a game whose online play is switched
 * off, and a launcher older than the one the game needs online. Kept apart from main.js, with no
 * DOM, so it can be tested on its own.
 *
 * A notice is HTML written in Alboria's console and shown by every launcher, so it is never inserted
 * as it is: formatNotice() rebuilds it from an allowlist. Every formatting tag works, links (https and
 * mailto only) are kept for the launcher to open in the system's browser, and nothing can run code
 * (scripts, event attributes, javascript: links) or load anything (images, frames, styles that load
 * files would tell their host each player's address). The console's preview (alboria-server,
 * console/src/notice.tsx) follows the same rules.
 */

/** Tags a notice keeps. */
const ALLOWED = new Set([
  "b", "strong", "i", "em", "u", "s", "del", "ins", "mark", "small", "sub", "sup", "code", "kbd",
  "br", "p", "div", "span", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre", "hr", "a",
]);
/** Tags with no closing tag. */
const VOID = new Set(["br", "hr", "img", "input", "link", "meta", "base", "source", "embed", "col", "area", "wbr", "track"]);
/** Tags removed together with everything inside them: what they hold is not text for the player. */
const DROPPED = new Set([
  "script", "style", "iframe", "frame", "frameset", "object", "embed", "img", "picture", "video", "audio", "source",
  "svg", "math", "canvas", "form", "input", "button", "textarea", "select", "link", "meta", "base", "template",
  "noscript", "title", "head",
]);
/** The one style a notice keeps: a text colour, with no url() or anything else in it. */
const SAFE_COLOR = /^\s*color\s*:\s*(#[0-9a-f]{3,8}|[a-z]+|rgba?\([\d\s.,%]+\))\s*;?\s*$/i;
const SAFE_LINK = /^(https:|mailto:)/i;
/** An opening or closing tag at the start of the text, with its attributes. */
const TAG = /^<(\/?)([a-zA-Z][a-zA-Z0-9]*)((?:\s+[^\s"'>/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'=<>`]+))?)*)\s*(\/?)>/;
const ATTRIBUTE = /([^\s"'>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
/** A character reference, kept as it is; any other "&" is written as "&amp;". */
const ENTITY = /^&(#\d{1,7}|#x[0-9a-f]{1,6}|[a-z][a-z0-9]{1,31});/i;

/** `text` escaped for HTML, keeping the character references it already has. */
const escapeText = (text) => {
  let out = "";
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "&") {
      const entity = ENTITY.exec(text.slice(i));
      if (entity) {
        out += entity[0];
        i += entity[0].length - 1;
      } else {
        out += "&amp;";
      }
    } else if (c === "<") out += "&lt;";
    else if (c === ">") out += "&gt;";
    else if (c === '"') out += "&quot;";
    else out += c;
  }
  return out;
};

/** The attributes `tag` keeps from its source: a safe link's address, and a text colour. */
const keptAttributes = (tag, source) => {
  let out = "";
  for (const match of source.matchAll(ATTRIBUTE)) {
    const name = match[1].toLowerCase();
    const value = (match[2] ?? match[3] ?? match[4] ?? "").trim();
    if (tag === "a" && name === "href" && SAFE_LINK.test(value)) out += ` href="${escapeText(value)}"`;
    if (name === "style" && SAFE_COLOR.test(value)) out += ` style="${escapeText(value)}"`;
  }
  return out;
};

/**
 * The notice's HTML rebuilt with only the allowed tags and attributes. Unknown tags are left out but
 * keep their text; text is escaped; tags left open are closed at the end.
 */
export const formatNotice = (html) => {
  const source = String(html ?? "");
  const open = [];
  let out = "";
  let i = 0;
  while (i < source.length) {
    const rest = source.slice(i);
    if (rest.startsWith("<!--")) {
      const end = source.indexOf("-->", i + 4);
      i = end < 0 ? source.length : end + 3;
      continue;
    }
    const tag = rest[0] === "<" ? TAG.exec(rest) : null;
    if (!tag) {
      const next = source.indexOf("<", i + 1);
      const end = next < 0 ? source.length : next;
      out += escapeText(source.slice(i, end));
      i = end;
      continue;
    }
    i += tag[0].length;
    const [, closing, rawName, attributes, selfClosing] = tag;
    const name = rawName.toLowerCase();
    if (DROPPED.has(name)) {
      // Everything up to its closing tag goes with it (a void tag has nothing inside).
      if (!closing && !selfClosing && !VOID.has(name)) {
        const end = source.toLowerCase().indexOf(`</${name}`, i);
        const after = end < 0 ? -1 : source.indexOf(">", end);
        i = after < 0 ? source.length : after + 1;
      }
      continue;
    }
    if (!ALLOWED.has(name)) continue;
    if (closing) {
      const at = open.lastIndexOf(name);
      if (at < 0) continue;
      while (open.length > at) out += `</${open.pop()}>`;
      continue;
    }
    out += `<${name}${keptAttributes(name, attributes)}>`;
    if (!VOID.has(name)) open.push(name);
  }
  while (open.length) out += `</${open.pop()}>`;
  return out;
};

/** Whether version "a.b.c" is older than "x.y.z" (the server's launcherOutdated() compares them so). */
export const olderThan = (version, min) => {
  const parse = (text) => /^(\d+)\.(\d+)\.(\d+)/.exec(String(text ?? ""))?.slice(1, 4).map(Number);
  const [a, b] = [parse(version), parse(min)];
  if (!a || !b) return false;
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i];
  return false;
};

/** The scheduled actions the launcher warns about, by action and scope (locale keys under alboriaService). */
const WARNINGS = {
  maint_on: { all: "warnMaintenanceAll", game: "warnMaintenanceGame", then: "thenMaintenance" },
  restart: { all: "warnRestart", then: "thenRestart" },
  game_off: { game: "warnGameOff", then: "thenGameOff" },
  close_rooms: { game: "warnCloseRooms", then: "thenCloseRooms" },
};

/**
 * What to show of the service's state (`status`, from alboria_status) at `now` (ms):
 * - block: what keeps the player out of new rooms, in the order the server checks it, or null:
 *   { kind: "down" } (not reachable), { kind: "outdated", min }, { kind: "gameOff" } or
 *   { kind: "maintenance", scope: "all" | "game", until } (`until` in ms, or null);
 * - notices: { scope: "all" | "game", html } for each notice, its HTML already made safe;
 * - warnings: { key, thenKey, at } for each countdown, soonest first; none for maintenance already on.
 *
 * `status` null means the service could not be reached. `launcherVersion` is this launcher's.
 */
export const serviceView = (status, launcherVersion, now = Date.now()) => {
  if (!status) return { block: { kind: "down" }, notices: [], warnings: [] };
  const game = status.game ?? null;
  const time = (iso) => (iso ? Date.parse(iso) : null);
  const ended = (until) => until !== null && until <= now;
  const serviceUntil = time(status.maintenanceUntil);
  const gameUntil = time(game?.maintenanceUntil);
  const serviceMaintenance = status.maintenance && !ended(serviceUntil);
  const gameMaintenance = Boolean(game?.maintenance) && !ended(gameUntil);

  let block = null;
  if (game?.minLauncher && olderThan(launcherVersion, game.minLauncher)) block = { kind: "outdated", min: game.minLauncher };
  else if (game && game.online === false) block = { kind: "gameOff" };
  else if (serviceMaintenance) block = { kind: "maintenance", scope: "all", until: serviceUntil };
  else if (gameMaintenance) block = { kind: "maintenance", scope: "game", until: gameUntil };

  const notices = [];
  if (status.motd) notices.push({ scope: "all", html: formatNotice(status.motd) });
  if (game?.motd) notices.push({ scope: "game", html: formatNotice(game.motd) });

  const alreadyOn = (warning) =>
    warning.action === "maint_on" && (warning.scope === "all" ? serviceMaintenance : gameMaintenance);
  const warnings = (status.warnings ?? [])
    .map((warning) => ({ ...warning, at: time(warning.at) }))
    .filter((warning) => warning.at !== null && warning.at > now && WARNINGS[warning.action]?.[warning.scope] && !alreadyOn(warning))
    .sort((a, b) => a.at - b.at)
    .map((warning) => ({
      key: WARNINGS[warning.action][warning.scope],
      thenKey: WARNINGS[warning.action].then,
      action: warning.action,
      at: warning.at,
    }));
  return { block, notices, warnings };
};

/** Whole minutes from `now` until `at`, at least 1, for "in 12 min". */
export const minutesUntil = (at, now = Date.now()) => Math.max(1, Math.ceil((at - now) / 60_000));

/** The key a closed notice is remembered by: its scope and text, so a new text shows again. */
export const noticeKey = (notice) => `${notice.scope}:${notice.html}`;
