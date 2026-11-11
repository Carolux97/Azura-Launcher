/*
 * What each text field of the launcher accepts, for typing with the physical keyboard and with the
 * on-screen one (virtualKeyboard.js). A field names its rule with data-text-rule.
 *
 * gamertag: ASCII letters, digits and single spaces, starting with a letter, up to 15 characters,
 * as Xbox 360 gamertags are. The runtime makes it a folder name (the profile's saves,
 * content_manager.cpp in the SDK), where "/" or ":" would break the path, and the game reads it
 * through a 16-byte buffer (XamUserGetName), where a character of more than one UTF-8 byte could
 * be cut in half.
 *
 * email and code: the Alboria account's email and the 6-digit codes sent to it (account.js).
 */

/** A rule: the characters a field may ever hold, what may be added to a text, and a cleaner. */
const rule = ({ maxLength, character, canStart }) => ({
  maxLength,
  /** True when `ch` may ever be part of the field. */
  accepts: (ch) => character.test(ch),
  /** True when `ch` may be added at the end of `text` now. */
  canAppend: (text, ch) =>
    character.test(ch) &&
    [...text].length < maxLength &&
    (text.length > 0 || canStart.test(ch)) &&
    !(ch === " " && text.endsWith(" ")),
  /** `text` with what the field does not accept left out, character by character. */
  clean(text) {
    let out = "";
    for (const ch of text) {
      if (this.canAppend(out, ch)) out += ch;
    }
    return out;
  },
});

export const TEXT_RULES = {
  gamertag: rule({ maxLength: 15, character: /^[A-Za-z0-9 ]$/, canStart: /^[A-Za-z]$/ }),
  // An email address: printable ASCII with no spaces; the service checks the rest (and a code
  // proves it is the player's).
  email: rule({ maxLength: 254, character: /^[!-~]$/, canStart: /^[A-Za-z0-9]$/ }),
  // A code sent by email: six digits.
  code: rule({ maxLength: 6, character: /^[0-9]$/, canStart: /^[0-9]$/ }),
};

/** The rule of a field (its data-text-rule), or null when it accepts any text. */
export const ruleOf = (field) => TEXT_RULES[field?.dataset?.textRule] ?? null;
