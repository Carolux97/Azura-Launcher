/*
 * Navigating the launcher with a controller: applies the actions the app sends
 * (src-tauri/src/gamepad.rs) to the page, and shows the hint bar with the buttons of the
 * controller in use.
 *
 * The focus moves between the controls of the navigation rail and of the section on screen. The
 * page is in "controller mode" (body.pad-mode: a strong focus ring and the hint bar) from the
 * first controller input until the mouse or the keyboard is used again.
 */

import { createRepeater, nextInDirection, stepOption } from "./gamepad.js";
import { glyph } from "./gamepadGlyphs.js";
import { ruleOf } from "./textRules.js";
import { isKeyboardOpen, openKeyboard, pressKeyboardKey, turnKeyboardPage } from "./virtualKeyboard.js";

const FOCUSABLE = 'button, input:not([type="hidden"]), select, a[href], [tabindex]:not([tabindex="-1"])';

const DIRECTIONS = new Set(["up", "down", "left", "right"]);

/** True when `element` can take the focus now: shown, enabled and not inside a hidden block. */
const isUsable = (element) =>
  !element.disabled &&
  !element.closest("[hidden]") &&
  element.getClientRects().length > 0 &&
  getComputedStyle(element).visibility !== "hidden";

/** A dialog over the launcher (data-modal, such as the welcome screen) while it is open, or null. */
const openModal = () => [...document.querySelectorAll("[data-modal]")].find((modal) => !modal.hidden) ?? null;

/**
 * The controls the focus can move between: the on-screen keyboard's keys while it is open, those
 * of an open dialog, or else the rail's and those of the section on screen.
 */
const controls = () => {
  const modal = openModal();
  if (modal && !isKeyboardOpen()) return [...modal.querySelectorAll(FOCUSABLE)].filter(isUsable);
  const scope = isKeyboardOpen() ? `.vkbd :is(${FOCUSABLE})` : `.rail :is(${FOCUSABLE}), .view.is-active :is(${FOCUSABLE})`;
  return [...document.querySelectorAll(scope)].filter(isUsable);
};

/** True for a field typed into, which the on-screen keyboard edits. */
const isTextField = (element) =>
  element instanceof HTMLInputElement && ["text", "search"].includes(element.type) && !element.readOnly;

/**
 * Where a control is on screen. A switch's checkbox is hidden behind its drawn track, so the
 * label that holds both stands for it.
 */
const rectOf = (element) => (element.closest(".toggle") ?? element).getBoundingClientRect();

const focus = (element) => {
  element.focus({ preventScroll: true });
  (element.closest(".toggle") ?? element).scrollIntoView({ block: "nearest", inline: "nearest" });
};

/** The navigation rail, the on-screen keyboard or the section on screen: where a control is. */
const regionOf = (element) => (element.closest(".rail") ? "rail" : element.closest(".vkbd") ? "keyboard" : "view");

/** The last control focused in a section, to come back to it from the rail. */
let lastInView = null;
document.addEventListener("focusin", (event) => {
  if (event.target instanceof Element && regionOf(event.target) === "view") lastInView = event.target;
});

/** Focuses the first control of the section on screen (or the keyboard, while open), or its rail item. */
const focusFirst = () => {
  if (isKeyboardOpen()) {
    const key = [...document.querySelectorAll(`.vkbd :is(${FOCUSABLE})`)].find(isUsable);
    if (key) focus(key);
    return;
  }
  const modal = openModal();
  const scope = modal ? modal.querySelectorAll(FOCUSABLE) : document.querySelectorAll(`.view.is-active :is(${FOCUSABLE})`);
  const first = [...scope].find(isUsable);
  const target = first ?? (modal ? null : document.querySelector(".nav-item.is-active"));
  if (target) focus(target);
};

/** Fires the events a change made by the user fires, so the page's own listeners run. */
const notify = (element, ...types) => types.forEach((type) => element.dispatchEvent(new Event(type, { bubbles: true })));

/**
 * Changes a select or a slider by one step (`delta` 1 or -1). True when `element` is one, so left
 * and right change it instead of moving the focus away.
 */
const adjust = (element, delta, wrap = false) => {
  if (element instanceof HTMLSelectElement) {
    const options = [...element.options];
    let index = stepOption(options, element.selectedIndex, delta);
    if (index === element.selectedIndex && wrap) index = stepOption(options, delta > 0 ? -1 : options.length, delta);
    if (index !== element.selectedIndex) {
      element.selectedIndex = index;
      notify(element, "input", "change");
    }
    return true;
  }
  if (element instanceof HTMLInputElement && element.type === "range") {
    const before = element.value;
    if (delta > 0) element.stepUp();
    else element.stepDown();
    if (element.value !== before) notify(element, "input", "change");
    return true;
  }
  return false;
};

/**
 * Wires controller navigation. `showView(name)` switches sections; `blocked()` is true while the
 * controller belongs to the game (it is running); `t` translates the hint labels and
 * `onLocaleChange` re-renders them; `onInput(listener)` subscribes to the app's controller events.
 */
export const setupGamepad = ({ showView, blocked, t, onLocaleChange, onInput }) => {
  const hints = document.getElementById("pad-hints");
  let family = "generic";

  const renderHints = () => {
    hints.classList.toggle("is-keyboard", isKeyboardOpen());
    const item = (buttons, key) => {
      const node = document.createElement("span");
      node.className = "pad-hint";
      // The glyphs are the project's own static SVG; the label goes in as text.
      const icons = document.createElement("span");
      icons.className = "pad-glyphs";
      icons.innerHTML = buttons.map((button) => glyph(family, button)).join("");
      const label = document.createElement("span");
      label.textContent = t(key);
      node.append(icons, label);
      return node;
    };
    hints.replaceChildren(
      ...(isKeyboardOpen()
        ? [
            item(["south"], "gamepad.type"),
            item(["west"], "gamepad.delete"),
            item(["north"], "gamepad.space"),
            item(["previous", "next"], "gamepad.page"),
            item(["east"], "gamepad.cancel"),
            item(["start"], "gamepad.done"),
          ]
        : [
            item(["south"], "gamepad.select"),
            item(["east"], "gamepad.back"),
            item(["previous", "next"], "gamepad.section"),
            item(["start"], "gamepad.play"),
          ]),
    );
  };

  const enter = (newFamily) => {
    if (document.body.classList.contains("pad-mode") && newFamily === family) return;
    family = newFamily;
    document.body.classList.add("pad-mode");
    document.body.dataset.padFamily = family;
    renderHints();
    hints.hidden = false;
  };
  const leave = () => {
    if (!document.body.classList.contains("pad-mode")) return;
    document.body.classList.remove("pad-mode");
    hints.hidden = true;
  };
  // Pointer movement of a few pixels only: a trackpad reports tiny motions on its own.
  let lastPointer = null;
  window.addEventListener("mousemove", (event) => {
    const moved = lastPointer && Math.hypot(event.screenX - lastPointer[0], event.screenY - lastPointer[1]) > 4;
    lastPointer = [event.screenX, event.screenY];
    if (moved) leave();
  });
  window.addEventListener("mousedown", leave);
  window.addEventListener("keydown", leave);
  onLocaleChange(() => {
    if (!hints.hidden) renderHints();
  });

  const move = (direction) => {
    const list = controls();
    const current = document.activeElement;
    if (!list.includes(current)) {
      focusFirst();
      return;
    }
    if ((direction === "left" || direction === "right") && adjust(current, direction === "right" ? 1 : -1)) return;
    // The focus moves within the rail or within the section; left from a section's edge goes to
    // the rail's current item, and right from the rail back to where it was in the section.
    const region = regionOf(current);
    const others = list.filter((element) => element !== current && regionOf(element) === region);
    let index = nextInDirection(rectOf(current), others.map(rectOf), direction);
    // Nothing more in the row: left or right reach the nearest control that way on another row (a
    // column beside this one whose controls start lower down), before leaving for the rail.
    if (index < 0 && (direction === "left" || direction === "right")) {
      index = nextInDirection(rectOf(current), others.map(rectOf), direction, { anyRow: true });
    }
    if (index >= 0) {
      focus(others[index]);
    } else if (direction === "left" && region === "view") {
      const item = document.querySelector(".nav-item.is-active");
      if (item) focus(item);
    } else if (direction === "right" && region === "rail") {
      if (lastInView && list.includes(lastInView)) focus(lastInView);
      else focusFirst();
    } else if ((direction === "up" || direction === "down") && region === "view") {
      // Nothing more to focus that way: the section scrolls, so none of it stays out of sight.
      const content = document.querySelector(".content");
      content?.scrollBy({ top: (direction === "down" ? 1 : -1) * content.clientHeight * 0.4, behavior: "smooth" });
    }
  };
  const repeater = createRepeater(move);

  const confirm = () => {
    const current = document.activeElement;
    if (!controls().includes(current)) {
      focusFirst();
      return;
    }
    // A select has no popup a controller can drive: confirm steps to its next option.
    if (adjust(current, 1, true)) return;
    // A text field is typed into with the on-screen keyboard.
    if (isTextField(current)) {
      openKeyboard(current, { t, focus, rule: ruleOf(current), onChange: () => !hints.hidden && renderHints() });
      return;
    }
    if (current instanceof HTMLInputElement && !["checkbox", "radio", "button", "submit"].includes(current.type)) return;
    current.click();
  };

  const back = () => {
    if (isKeyboardOpen()) {
      pressKeyboardKey("cancel");
      return;
    }
    // A prompt waiting for an answer is cancelled first (its cancel button has data-pad-back).
    const cancel = [...(openModal() ?? document).querySelectorAll("[data-pad-back]")].find(isUsable);
    if (cancel) {
      cancel.click();
      return;
    }
    // A dialog with nothing to cancel (the welcome screen's first step) stays until answered.
    if (openModal()) return;
    if (!document.querySelector('.view.is-active[data-view="play"]')) showView("play");
    const launch = document.getElementById("launch");
    if (launch && isUsable(launch)) focus(launch);
  };

  const switchSection = (delta) => {
    const items = [...document.querySelectorAll(".nav-item")].filter(isUsable);
    const index = items.findIndex((item) => item.classList.contains("is-active"));
    const target = items[Math.min(items.length - 1, Math.max(0, index + delta))];
    if (!target || target.classList.contains("is-active")) return;
    target.click();
    focusFirst();
  };

  onInput(({ action, pressed, family: inputFamily }) => {
    if (!pressed) {
      if (DIRECTIONS.has(action)) repeater.release(action);
      return;
    }
    if (blocked()) {
      repeater.releaseAll();
      return;
    }
    enter(inputFamily);
    if (DIRECTIONS.has(action)) repeater.press(action);
    else if (action === "confirm") confirm();
    else if (action === "back") back();
    else if (isKeyboardOpen()) {
      // The keyboard's own buttons: West deletes, North types a space, Start is Done.
      if (action === "west") pressKeyboardKey("delete");
      else if (action === "north") pressKeyboardKey("space");
      else if (action === "start") pressKeyboardKey("done");
      else if (action === "previous") turnKeyboardPage(-1);
      else if (action === "next") turnKeyboardPage(1);
    }
    // While a dialog is open, the sections and Play wait for it.
    else if (openModal()) return;
    else if (action === "previous") switchSection(-1);
    else if (action === "next") switchSection(1);
    else if (action === "start") {
      showView("play");
      document.getElementById("launch")?.click();
    }
  });
};
