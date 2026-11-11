/*
 * Controller navigation without the page: which control a direction moves the focus to, how a
 * select or a slider steps, and when a held direction repeats. gamepadNav.js applies them to the
 * page; the controller itself is read by the app (src-tauri/src/gamepad.rs).
 */

/** A held direction repeats after this long, then every REPEAT_INTERVAL_MS. */
export const REPEAT_DELAY_MS = 380;
export const REPEAT_INTERVAL_MS = 110;

/** The directions, as unit vectors in screen coordinates (y grows downwards). */
const VECTORS = { up: [0, -1], down: [0, 1], left: [-1, 0], right: [1, 0] };

/** Distance across the direction weighs this much more than along it. */
const CROSS_WEIGHT = 2.5;

const center = (rect) => [rect.left + rect.width / 2, rect.top + rect.height / 2];

/**
 * The index in `candidates` of the rectangle the focus moves to from `origin` in `direction`, or
 * -1 when there is none that way. A candidate counts only when it lies beyond the origin's edge in
 * that direction (so a control beside the focused one is never "below" it), and, left or right,
 * only when it shares some of the origin's height (left and right stay in the row; a control on
 * the next row is reached with down). Among those, the one nearest along the direction wins, with
 * any offset across the direction weighing CROSS_WEIGHT times more, so the focus prefers to stay
 * in its column. With `anyRow`, left and right also take controls on other rows: the fallback
 * when the row has nothing more that way, so a column beside it whose controls start lower down
 * (the friends list beside the Play page's main column) is still reached.
 */
export const nextInDirection = (origin, candidates, direction, { anyRow = false } = {}) => {
  const [dx, dy] = VECTORS[direction] ?? [0, 0];
  if (!dx && !dy) return -1;
  const [ox, oy] = center(origin);
  // How far beyond the origin's edge, in the direction, a point is.
  const edge = dx > 0 ? origin.left + origin.width : dx < 0 ? origin.left : dy > 0 ? origin.top + origin.height : origin.top;
  let best = -1;
  let bestScore = Infinity;
  candidates.forEach((rect, index) => {
    const [cx, cy] = center(rect);
    const along = dx ? (cx - edge) * dx : (cy - edge) * dy;
    if (along <= 0) return;
    if (dx && !anyRow && (rect.top >= origin.top + origin.height || rect.top + rect.height <= origin.top)) return;
    // How far the candidate's box is from the origin's centre line, zero when they overlap.
    const across = dx
      ? Math.max(0, rect.top - oy, oy - (rect.top + rect.height))
      : Math.max(0, rect.left - ox, ox - (rect.left + rect.width));
    const score = along + across * CROSS_WEIGHT;
    if (score < bestScore) {
      bestScore = score;
      best = index;
    }
  });
  return best;
};

/**
 * The option index of a select after stepping `delta` from `selected`, skipping disabled options
 * and stopping at the ends. `options` is a list of { disabled }.
 */
export const stepOption = (options, selected, delta) => {
  for (let index = selected + delta; index >= 0 && index < options.length; index += delta) {
    if (!options[index].disabled) return index;
  }
  return selected;
};

/**
 * Follows the directions held down: `press(direction)` acts at once, then again after
 * REPEAT_DELAY_MS and every REPEAT_INTERVAL_MS until `release(direction)` or `releaseAll()`.
 * `timers` are setTimeout and clearTimeout, given so tests can drive time.
 */
export const createRepeater = (act, timers = globalThis) => {
  const held = new Map();
  const repeat = (direction) => {
    act(direction);
    held.set(direction, timers.setTimeout(() => repeat(direction), REPEAT_INTERVAL_MS));
  };
  const release = (direction) => {
    timers.clearTimeout(held.get(direction));
    held.delete(direction);
  };
  return {
    press(direction) {
      release(direction);
      act(direction);
      held.set(direction, timers.setTimeout(() => repeat(direction), REPEAT_DELAY_MS));
    },
    release,
    releaseAll() {
      [...held.keys()].forEach(release);
    },
  };
};
