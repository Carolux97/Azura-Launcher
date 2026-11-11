/*
 * The controller buttons the hint bar shows, drawn as inline SVG by the project (no third-party
 * artwork). Buttons are named by position, as src-tauri/src/gamepad.rs sends them: `south`
 * confirms and `east` goes back on every controller, `west` and `north` serve the on-screen
 * keyboard, and each family labels them its own way: Xbox (and the Steam Deck) A, B, X and Y,
 * PlayStation cross, circle, square and triangle, Nintendo B, A, Y and X (its A is the east
 * button), and a generic controller the position itself.
 */

const FACE_FILL = "#16181b";
const FONT = 'font-family="Barlow Condensed, Barlow, sans-serif" font-weight="700"';

/** A round face button with `inner` drawn on it, outlined in `color`. */
const face = (color, inner) =>
  `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10.5" fill="${FACE_FILL}" stroke="${color}" stroke-width="1.5"/>${inner}</svg>`;

/** A letter centred on a face button. */
const letter = (text, color) => face(color, `<text x="12" y="16.6" text-anchor="middle" font-size="13" fill="${color}" ${FONT}>${text}</text>`);

/** A generic face button: four dots in a diamond, the one at `position` filled. */
const diamond = (position) => {
  const dots = { north: [12, 6.5], east: [17.5, 12], south: [12, 17.5], west: [6.5, 12] };
  const circles = Object.entries(dots)
    .map(([name, [x, y]]) => `<circle cx="${x}" cy="${y}" r="2.6" ${name === position ? 'fill="#ece6da"' : 'fill="none" stroke="#6d6960" stroke-width="1.2"'}/>`)
    .join("");
  return face("#a29c90", circles);
};

/** A shoulder or menu button: a rounded bar with `text` on it. */
const bar = (text) =>
  `<svg viewBox="0 0 34 20" aria-hidden="true"><rect x="0.75" y="0.75" width="32.5" height="18.5" rx="6" fill="${FACE_FILL}" stroke="#a29c90" stroke-width="1.5"/><text x="17" y="14.6" text-anchor="middle" font-size="12" fill="#ece6da" ${FONT}>${text}</text></svg>`;

/** The menu button: three short lines, or `+` on Nintendo controllers. */
const menu = (plus) =>
  `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10.5" fill="${FACE_FILL}" stroke="#a29c90" stroke-width="1.5"/>${
    plus
      ? '<path d="M12 7.5v9M7.5 12h9" stroke="#ece6da" stroke-width="2" stroke-linecap="round"/>'
      : '<path d="M7.5 8.5h9M7.5 12h9M7.5 15.5h9" stroke="#ece6da" stroke-width="1.6" stroke-linecap="round"/>'
  }</svg>`;

/** Each family's glyph for each button. */
const GLYPHS = {
  xbox: {
    south: letter("A", "#6cbf3b"),
    east: letter("B", "#e2473b"),
    west: letter("X", "#3d8ee8"),
    north: letter("Y", "#f2c230"),
    previous: bar("LB"),
    next: bar("RB"),
    start: menu(false),
  },
  playstation: {
    south: face("#7aa7e8", '<path d="M8 8l8 8M16 8l-8 8" stroke="#7aa7e8" stroke-width="2" stroke-linecap="round"/>'),
    east: face("#e8707a", '<circle cx="12" cy="12" r="4.6" fill="none" stroke="#e8707a" stroke-width="2"/>'),
    west: face("#d08bd8", '<rect x="8" y="8" width="8" height="8" fill="none" stroke="#d08bd8" stroke-width="2"/>'),
    north: face("#4fc7a0", '<path d="M12 7.2l5 8.6H7z" fill="none" stroke="#4fc7a0" stroke-width="2" stroke-linejoin="round"/>'),
    previous: bar("L1"),
    next: bar("R1"),
    start: menu(false),
  },
  nintendo: {
    south: letter("B", "#ece6da"),
    east: letter("A", "#ece6da"),
    west: letter("Y", "#ece6da"),
    north: letter("X", "#ece6da"),
    previous: bar("L"),
    next: bar("R"),
    start: menu(true),
  },
  generic: {
    south: diamond("south"),
    east: diamond("east"),
    west: diamond("west"),
    north: diamond("north"),
    previous: bar("L1"),
    next: bar("R1"),
    start: menu(false),
  },
};

/** The SVG markup of `button` ("south", "east", "west", "north", "previous", "next", "start") for `family`. */
export const glyph = (family, button) => (GLYPHS[family] ?? GLYPHS.generic)[button] ?? "";
