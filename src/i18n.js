/*
 * Launcher translations: loads the message files, applies them to the page and lets the user
 * override the language detected from the system.
 *
 * Static text in index.html is marked with attributes:
 *   data-i18n="key"                    sets the element's text;
 *   data-i18n-attr="attr:key,attr:key" sets attributes (placeholder, aria-label, ...).
 * Text built from JavaScript asks for its strings with t() and re-renders through onLocaleChange.
 *
 * This module does not store the user's choice; main.js saves it with the other settings.
 */

import {
  FALLBACK_LOCALE,
  interpolate,
  isSupportedLocale,
  lookup,
  resolveLocale,
} from "./locale.js";

/** Preference value that follows the system language. */
const AUTO = "auto";

/**
 * Tags that wrap a placeholder in an element instead of plain text. Keyed by message key, then by
 * placeholder name: { file: ["code", "default.xex"] } renders {file} as <code>default.xex</code>.
 * The content is a fixed text, or a function that returns it (a translated word, in the title's
 * own word order).
 */
const RICH_PLACEHOLDERS = {
  "install.gameFolderHelp": { file: ["code", "default.xex"] },
  "welcome.title": { player: ["em", () => t("welcome.titlePlayer")] },
};

let fallbackMessages = {};
let messages = {};
let currentLocale = FALLBACK_LOCALE;
let currentPreference = AUTO;
const listeners = new Set();

/** Fetches one message file from locales/. */
const loadMessages = async (code) => {
  const response = await fetch(`locales/${code}.json`);
  if (!response.ok) throw new Error(`Cannot load locale "${code}": HTTP ${response.status}`);
  return response.json();
};

/** The user's choice: "auto" or a locale code. */
export const getPreference = () => currentPreference;

/** Keeps a preference only when it is "auto" or a supported locale; anything else means "auto". */
const normalizePreference = (preference) =>
  preference === AUTO || isSupportedLocale(preference) ? preference : AUTO;

/** The locale the system asks for, ignoring any saved override. */
export const systemLocale = () => resolveLocale(navigator.languages ?? [navigator.language]);

/** The active locale code, also used for Intl date and number formatting. */
export const getLocale = () => currentLocale;

/**
 * Returns the message for `key` in the active language, falling back to English when the active
 * file lacks it, and to the key itself when English lacks it too (so a gap is visible, not blank).
 */
export const t = (key, vars) => {
  const text = lookup(messages, key) ?? lookup(fallbackMessages, key);
  if (typeof text !== "string") {
    console.warn(`Missing translation key: ${key}`);
    return key;
  }
  return interpolate(text, vars);
};

/** Fills `el` with `text`, turning rich placeholders into their elements. */
const fillRich = (el, text, placeholders) => {
  el.replaceChildren();
  for (const part of text.split(/(\{\w+\})/)) {
    const name = part.match(/^\{(\w+)\}$/)?.[1];
    if (name && placeholders[name]) {
      const [tag, content] = placeholders[name];
      const node = document.createElement(tag);
      node.textContent = typeof content === "function" ? content() : content;
      el.appendChild(node);
    } else if (part) {
      el.appendChild(document.createTextNode(part));
    }
  }
};

/** Applies the active language to every marked element under `root`. */
export const translatePage = (root = document) => {
  root.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.dataset.i18n;
    const placeholders = RICH_PLACEHOLDERS[key];
    if (placeholders) {
      fillRich(el, t(key), placeholders);
    } else {
      el.textContent = t(key);
    }
  });
  root.querySelectorAll("[data-i18n-attr]").forEach((el) => {
    for (const pair of el.dataset.i18nAttr.split(",")) {
      const [attr, key] = pair.split(":").map((s) => s.trim());
      el.setAttribute(attr, t(key));
    }
  });
  document.documentElement.lang = currentLocale;
  document.title = t("meta.title");
};

/** Registers a callback that runs after every language change (and returns an unsubscribe). */
export const onLocaleChange = (listener) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

/**
 * Applies the user's choice ("auto" follows the system): re-translates the page and notifies the
 * listeners. A message file that fails to load leaves the launcher in English instead of half
 * translated.
 */
export const setPreference = async (preference) => {
  currentPreference = normalizePreference(preference);
  let locale = currentPreference === AUTO ? systemLocale() : currentPreference;
  try {
    messages = locale === FALLBACK_LOCALE ? fallbackMessages : await loadMessages(locale);
  } catch (error) {
    console.error(error);
    locale = FALLBACK_LOCALE;
    messages = fallbackMessages;
  }
  currentLocale = locale;
  translatePage();
  listeners.forEach((listener) => listener(locale));
};

/** Loads English (always needed as the fallback) and then the saved preference. */
export const initI18n = async (preference = AUTO) => {
  fallbackMessages = await loadMessages(FALLBACK_LOCALE);
  await setPreference(preference);
};
