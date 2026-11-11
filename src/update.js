/*
 * What the launcher's update button shows, from the update found, the download's progress and
 * the game's state. Kept apart from main.js so it can be tested without a window.
 *
 * Installing an update restarts the launcher, so it waits while the game starts or runs: the
 * button stays visible but disabled, and says why.
 */

/**
 * The update button's state.
 *
 * @param {{ version: string } | null} update  The newer launcher the updater found, or null.
 * @param {number | null} progress  null before downloading, -1 while the size is unknown, else 0 to 100.
 * @param {string} game  The game's state: "idle", or anything else while it starts or runs.
 * @returns {{ hidden: boolean, disabled: boolean, key?: string, vars?: object, hint?: string }}
 *   `key` and `vars` name the button's text in the locale files, `hint` its tooltip.
 */
export const updateButtonState = (update, progress, game) => {
  if (!update) return { hidden: true, disabled: true };
  if (progress !== null) {
    return { hidden: false, disabled: true, key: "update.downloading", vars: { percent: progress } };
  }
  const busy = game !== "idle";
  return {
    hidden: false,
    disabled: busy,
    key: "update.available",
    vars: { version: update.version },
    ...(busy ? { hint: "update.closeGame" } : {}),
  };
};

/** Whether an update can start now: there is one, nothing is downloading and the game is closed. */
export const canStartUpdate = (update, progress, game) => !updateButtonState(update, progress, game).disabled;
