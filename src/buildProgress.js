/*
 * The progress of a runtime build from the kit, as a single bar: how far along the whole build is
 * and how long is left, from the steps platform.onRuntimeBuildProgress reports
 * ({ phase, done, total }).
 *
 * Each step gets the share of the bar it takes of a first build (measured on a base M4: codegen
 * about 25 s, compiling about 75 s, the rest a second or two). Compiling advances with its files;
 * translating reports no progress of its own, so it advances with the time it has taken against
 * the time it usually takes, and stops short of its end.
 */

/** Where each step starts and ends on the bar, in percent. */
export const PHASE_RANGES = {
  preparing: [0, 2],
  translating: [2, 20],
  compiling: [20, 95],
  linking: [95, 98],
  installing: [98, 100],
};

/** How long translating usually takes, in milliseconds, for its share of the bar. */
export const TRANSLATING_MS = 30_000;

/** Linking and installing after the last file, in milliseconds, for the time left. */
const AFTER_COMPILING_MS = 2_000;

/** Files compiled before the time left is estimated from their pace. */
const FILES_BEFORE_ESTIMATE = 5;

/**
 * Follows one build: `update` takes each progress report with the time it arrived, `percent` and
 * `remainingMs` read the bar and the time left (null while it cannot be told).
 */
export const createBuildTracker = () => {
  let phase = "preparing";
  let done = 0;
  let total = 0;
  let phaseStarted = 0;
  let compileStarted = null;
  let compileDoneAtStart = 0;

  return {
    update({ phase: next, done: nextDone = 0, total: nextTotal = 0 }, now) {
      if (next !== phase) {
        phase = next;
        phaseStarted = now;
      }
      if (next === "compiling" && compileStarted === null) {
        compileStarted = now;
        // Files already up to date are reported at once; the pace counts only the ones compiled.
        compileDoneAtStart = nextDone;
      }
      done = nextDone;
      total = nextTotal;
    },

    get phase() {
      return phase;
    },

    percent(now) {
      const [start, end] = PHASE_RANGES[phase] ?? PHASE_RANGES.preparing;
      let share = 0;
      if (phase === "compiling" && total > 0) share = done / total;
      else if (phase === "translating") share = Math.min(0.95, (now - phaseStarted) / TRANSLATING_MS);
      return Math.round(start + (end - start) * share);
    },

    remainingMs(now) {
      if (phase === "linking" || phase === "installing") return AFTER_COMPILING_MS;
      if (phase !== "compiling" || compileStarted === null) return null;
      const compiled = done - compileDoneAtStart;
      if (compiled < FILES_BEFORE_ESTIMATE || now <= compileStarted) return null;
      const perFile = (now - compileStarted) / compiled;
      return Math.round((total - done) * perFile + AFTER_COMPILING_MS);
    },
  };
};
