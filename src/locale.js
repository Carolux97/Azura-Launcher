/*
 * Locale logic for the launcher, kept free of DOM access so it can be unit tested with Node.
 *
 * The launcher supports the base languages Gears of War 3 shipped with, across its disc editions
 * and its store listing. Regional variants map to their base language (es-MX -> es, pt-PT -> pt).
 * Chinese is the exception: the game only had Traditional Chinese, so only Traditional locales
 * (zh-TW, zh-HK, zh-Hant) match it.
 */

/** Language used when none of the user's preferred languages is supported. */
export const FALLBACK_LOCALE = "en";

/** Supported launcher languages, with their names written in that same language. */
export const SUPPORTED_LOCALES = [
  { code: "en", name: "English" },
  { code: "cs", name: "Čeština" },
  { code: "de", name: "Deutsch" },
  { code: "es", name: "Español" },
  { code: "fr", name: "Français" },
  { code: "hu", name: "Magyar" },
  { code: "it", name: "Italiano" },
  { code: "ja", name: "日本語" },
  { code: "ko", name: "한국어" },
  { code: "pl", name: "Polski" },
  { code: "pt", name: "Português" },
  { code: "ru", name: "Русский" },
  { code: "zh-Hant", name: "繁體中文" },
];

const SUPPORTED_CODES = new Set(SUPPORTED_LOCALES.map(({ code }) => code));

/**
 * Maps one BCP 47 language tag (such as "es-MX" or "zh-TW") to a supported launcher locale, or
 * returns null when the tag is invalid or its language is not supported.
 *
 * Intl.Locale#maximize() fills in the likely script, which is how "zh-TW" and "zh-HK" are
 * recognised as Traditional Chinese ("zh-Hant-TW") and "zh-CN" as Simplified ("zh-Hans-CN").
 */
export const matchLocale = (tag) => {
  let locale;
  try {
    locale = new Intl.Locale(tag).maximize();
  } catch {
    return null;
  }
  if (locale.language === "zh") {
    return locale.script === "Hant" ? "zh-Hant" : null;
  }
  return SUPPORTED_CODES.has(locale.language) ? locale.language : null;
};

/**
 * Picks the launcher locale from the user's preferred languages, in order of preference (the
 * shape of navigator.languages). The first supported one wins; otherwise the fallback is used.
 */
export const resolveLocale = (preferredTags) => {
  for (const tag of preferredTags ?? []) {
    const match = matchLocale(tag);
    if (match) return match;
  }
  return FALLBACK_LOCALE;
};

/** True when `code` is one of the supported launcher locales. */
export const isSupportedLocale = (code) => SUPPORTED_CODES.has(code);

/** Reads a dotted key ("nav.play") from a nested message object; undefined when absent. */
export const lookup = (messages, key) =>
  key.split(".").reduce((node, part) => (node == null ? undefined : node[part]), messages);

/** Replaces {name} placeholders in `text` with the matching value from `vars`. */
export const interpolate = (text, vars = {}) =>
  text.replace(/\{(\w+)\}/g, (placeholder, name) => (name in vars ? String(vars[name]) : placeholder));

/** Lists every dotted key that holds a string in a nested message object. */
export const flattenKeys = (messages, prefix = "") =>
  Object.entries(messages).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof value === "object" && value !== null ? flattenKeys(value, path) : [path];
  });
