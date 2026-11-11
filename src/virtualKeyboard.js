/*
 * The on-screen keyboard a controller types into the launcher's text fields with: three pages of
 * keys (letters, symbols, accented letters), space, delete, a shift lock, Done and Cancel. It edits
 * a copy of the field's text and writes it back only on Done, firing the field's "input" event so
 * the page saves it as if typed. A field with a rule (textRules.js) gets only the keys it accepts:
 * the others are disabled, and a key that does not fit where the text is now does nothing. gamepadNav.js opens it and moves the focus over its keys; a mouse
 * can press them too.
 */

/** The character keys of each page, row by row; letters follow the shift state's case. */
export const PAGES = {
  letters: ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"],
  symbols: ["1234567890", "-_.,:;!?@#", "/\\()[]{}<>", "&%$+=*'\"~|"],
  accents: ["áéíóúàèìòù", "âêîôûäëïöü", "ñçãõåøæœß", "¿¡€£¥°·…"],
};

/** The order the shoulder buttons go through the pages in. */
export const PAGE_ORDER = ["letters", "symbols", "accents"];

/**
 * A character in the shift state's case. A letter whose capital is more than one character (ß)
 * stays as it is, so one key always types one character.
 */
export const shifted = (character, shift) => {
  if (!shift) return character;
  const upper = character.toUpperCase();
  return [...upper].length === 1 ? upper : character;
};

/**
 * The text after pressing `key` on `text`: a character (in the shift state's case), "space", or
 * "delete" (the last character). A character is added only within `maxLength` and, with a `rule`
 * (textRules.js), only where the rule lets it.
 */
export const applyKey = (text, key, { shift, maxLength, rule = null }) => {
  if (key === "delete") return [...text].slice(0, -1).join("");
  const character = key === "space" ? " " : shifted(key, shift);
  if ([...text].length >= maxLength) return text;
  if (rule && !rule.canAppend(text, character)) return text;
  return text + character;
};

let current = null;

/** True while a keyboard is open. */
export const isKeyboardOpen = () => current !== null;

/**
 * Presses one of the keyboard's own keys ("delete", "space", "done", "cancel", "symbols",
 * "accents"), when it is open.
 */
export const pressKeyboardKey = (key) => current?.root.querySelector(`[data-vk="${key}"]`)?.click();

/** Moves the open keyboard to the page `delta` steps away (the shoulder buttons). */
export const turnKeyboardPage = (delta) => current?.turn(delta);

/**
 * Opens the keyboard for `input`. `t` translates its labels; `onChange()` runs when it opens and
 * closes, so the hint bar can follow; `focus(element)` moves the focus to a key.
 */
export const openKeyboard = (input, { t, onChange, focus, rule = null }) => {
  if (current) return;
  const maxLength = rule?.maxLength ?? (input.maxLength > 0 ? input.maxLength : 64);
  /** True when the field accepts `character` in some case. */
  const accepted = (character) => !rule || rule.accepts(character) || rule.accepts(character.toUpperCase());
  let text = input.value;
  let shift = text.length === 0;
  let page = "letters";

  const element = (tag, className, content = "") => {
    const node = document.createElement(tag);
    node.className = className;
    node.textContent = content;
    return node;
  };
  const key = (label, vk, wide = false) => {
    const button = element("button", wide ? "vkbd-key is-wide" : "vkbd-key", label);
    button.type = "button";
    button.dataset.vk = vk;
    return button;
  };

  const root = element("div", "vkbd");
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-modal", "true");
  const panel = element("div", "vkbd-panel");
  const title = element("div", "vkbd-title", input.closest(".field")?.querySelector("span")?.textContent ?? "");
  const display = element("div", "vkbd-display");
  const characters = element("div", "vkbd-keys");
  const shiftKey = key("⇧", "shift");
  const symbolsKey = key("#+=", "symbols");
  const accentsKey = key("áé", "accents");
  const cancel = key(t("gamepad.cancel"), "cancel", true);
  // East (B, Circle) cancels the keyboard, as it cancels any prompt (gamepadNav.js).
  cancel.dataset.padBack = "";
  const bottom = element("div", "vkbd-row");
  bottom.append(shiftKey, symbolsKey, accentsKey, key(t("gamepad.space"), "space", true), key("⌫", "delete"), cancel, key(t("gamepad.done"), "done", true));

  /** Draws the current page's keys, keeping the focus on the same row and column. */
  const renderPage = () => {
    const focused = characters.contains(document.activeElement) ? document.activeElement : null;
    const at = focused ? [...characters.querySelectorAll("button")].indexOf(focused) : -1;
    characters.replaceChildren(
      ...PAGES[page].map((row) => {
        const line = element("div", "vkbd-row");
        line.append(
          ...[...row].map((character) => {
            const button = key(shifted(character, shift), character);
            button.disabled = !accepted(character);
            return button;
          }),
        );
        return line;
      }),
    );
    symbolsKey.textContent = page === "symbols" ? "ABC" : "#+=";
    accentsKey.textContent = page === "accents" ? "ABC" : "áé";
    shiftKey.classList.toggle("is-on", shift);
    if (at >= 0) {
      // The key in the same place, or the nearest one the field accepts.
      const buttons = [...characters.querySelectorAll("button")];
      const place = Math.min(at, buttons.length - 1);
      const usable = buttons
        .map((button, index) => [button, Math.abs(index - place)])
        .filter(([button]) => !button.disabled)
        .sort((a, b) => a[1] - b[1]);
      focus(usable[0]?.[0] ?? shiftKey);
    }
  };
  const render = () => {
    display.textContent = text;
    display.dataset.count = `${[...text].length}/${maxLength}`;
  };
  const close = (save) => {
    if (save && text !== input.value) {
      input.value = text;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
    root.remove();
    current = null;
    focus(input);
    onChange();
  };
  /** Moves `delta` pages on, skipping the pages the field accepts nothing of. */
  const turn = (delta) => {
    const open = PAGE_ORDER.filter((name) => name === "letters" || !root.querySelector(`[data-vk="${name}"]`).disabled);
    const index = open.indexOf(page);
    page = open[(index + delta + open.length) % open.length];
    renderPage();
  };

  panel.addEventListener("click", (event) => {
    const vk = event.target.closest("[data-vk]")?.dataset.vk;
    if (!vk) return;
    if (vk === "done") return close(true);
    if (vk === "cancel") return close(false);
    if (vk === "shift") {
      shift = !shift;
      renderPage();
    } else if (vk === "symbols" || vk === "accents") {
      page = page === vk ? "letters" : vk;
      renderPage();
    } else {
      text = applyKey(text, vk, { shift, maxLength, rule });
      render();
    }
  });
  root.addEventListener("keydown", (event) => {
    if (event.key === "Escape") close(false);
  });

  // A page with no key the field accepts cannot be opened.
  symbolsKey.disabled = ![...PAGES.symbols.slice(1).join("")].some(accepted);
  accentsKey.disabled = ![...PAGES.accents.join("")].some(accepted);
  bottom.querySelector('[data-vk="space"]').disabled = !accepted(" ");
  const keys = element("div", "vkbd-keys");
  keys.append(characters, bottom);
  panel.append(title, display, keys);
  root.append(panel);
  document.body.append(root);
  current = { root, turn };
  renderPage();
  render();
  focus([...characters.querySelectorAll("button")].slice(10).find((button) => !button.disabled) ?? shiftKey);
  onChange();
};
