/*
 * Draws a <select> as buttons: a segmented control (`data-mirror="segmented"`) or a list to pick
 * from (`data-mirror="list"`). The <select> stays the control the code reads and writes, hidden
 * from view: a button sets its value and sends the same "change" event a choice in it would, so
 * the code behind it does not know the difference.
 *
 * The buttons follow the <select>: they are drawn again when its options change (filled by code,
 * or translated) and when it sends "change". Code that sets its value without an event calls
 * syncMirrors() afterwards.
 */

/** Each mirrored <select> and the function that draws its buttons again. */
const mirrors = new Map();

/** Draws the buttons of `select` into `box`: one per option, the selected one marked. */
const draw = (select, box) => {
  const kind = select.dataset.mirror;
  box.replaceChildren(
    ...[...select.options].map((option) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = kind === "list" ? "pick" : "";
      button.textContent = option.textContent;
      button.disabled = option.disabled;
      const on = option.value === select.value;
      button.classList.toggle("is-on", on);
      button.setAttribute("aria-pressed", String(on));
      button.addEventListener("click", () => {
        if (select.value === option.value) return;
        select.value = option.value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
      });
      return button;
    }),
  );
};

/** Mirrors every `select[data-mirror]` under `root`, once each. */
export const mirrorSelects = (root = document) => {
  for (const select of root.querySelectorAll("select[data-mirror]")) {
    if (mirrors.has(select)) continue;
    const box = document.createElement("div");
    box.className = select.dataset.mirror === "list" ? "pick-list" : "segmented";
    box.setAttribute("role", "group");
    if (select.hasAttribute("aria-label")) box.setAttribute("aria-label", select.getAttribute("aria-label"));
    select.after(box);
    const redraw = () => draw(select, box);
    mirrors.set(select, redraw);
    select.addEventListener("change", redraw);
    // Options added, removed or translated.
    new MutationObserver(redraw).observe(select, { childList: true, subtree: true, characterData: true });
    redraw();
  }
};

/** Draws every mirror again, after code set a <select>'s value without a "change" event. */
export const syncMirrors = () => {
  for (const redraw of mirrors.values()) redraw();
};
