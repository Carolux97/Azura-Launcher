/*
 * The player's account: the welcome screen and the Account section.
 *
 * The player plays with an Alboria account (our online ecosystem: alboria-notes:azura/alboria-relay.md, sections
 * 3 and 4.6) or with a local account (no Alboria: Local and System Link). Until they choose, also
 * after updating from a launcher without accounts, the welcome screen asks: create an account,
 * sign in to one (which links this installation to it) or play locally. An account has no
 * password: a 6-digit code sent to its email proves it is the player's.
 *
 * The choice is saved with the settings (`account`, see settings.rs) and sets the game's
 * network_mode. With an Alboria account the launcher signs in on start-up to show the identity
 * (Gamertag#tag) and the state of the service, and the game later signs in by itself with the
 * same key (identity.toml). The gamertag lives in the account: changing it here signs in again so
 * Alboria gives the tag for the new name.
 *
 * The same dialog box holds the account's other steps: moving it to another email and deleting it,
 * each confirmed with a code.
 *
 * While the service cannot send codes (alboriaStatus: emailCodes false), an account is made with
 * no code and its email stays unverified; signing in on another device waits until codes are back.
 * Once they are, the Account section asks to verify the email by the deadline the service gives
 * (verify_by), after which an unverified account is deleted.
 */

import {
  alboriaAccount,
  alboriaChangeEmail,
  alboriaDelete,
  alboriaDeleteCode,
  alboriaEmailCode,
  alboriaExport,
  alboriaRemoveDevice,
  alboriaRequestCode,
  alboriaSignIn,
  alboriaStatus,
  alboriaVerify,
  alboriaVerifyEmail,
  alboriaVerifyEmailCode,
  openAlboriaSite,
  alboriaWebLink,
  alboriaWebLinkAnswer,
  openAlboriaBlog,
  openAlboriaDiscord,
} from "./platform.js";
import { TEXT_RULES } from "./textRules.js";

/** The account choices saved with the settings (settings.rs). */
export const ACCOUNT_ALBORIA = "alboria";
export const ACCOUNT_LOCAL = "local";

/** Gears of War 3's title ID, as devices report their game. */
const GAME_NAMES = { "4D5308AB": "Gears of War 3" };

/** The error codes the service answers that have their own message (accountErrors in the locales). */
const KNOWN_ERRORS = new Set([
  "network",
  "email_taken",
  "no_account",
  "bad_code",
  "code_expired",
  "too_many_requests",
  "disposable_email",
  "email_suppressed",
  "banned",
  "gamertag_taken",
  "mail_failed",
  "bad_request",
  "client_too_old",
  "codes_unavailable",
  "already_verified",
  "no_link",
]);

/**
 * Wires the welcome screen and the Account section. `state` is main.js's (it reads and sets
 * `account`, `userName` and `alboria`); `persist` saves the settings; `renderPlay` redraws the
 * Play section's player line; `toast` shows a short message; `t` and `getLocale` translate.
 */
export const setupAccount = ({ state, persist, renderPlay, toast, t, getLocale }) => {
  const $ = (id) => document.getElementById(id);
  const welcome = $("welcome");
  const box = welcome.querySelector(".welcome-box");

  // The service as the Account section shows it: "connecting", "connected", "offline" or
  // "error", the identity once known and the account (email, devices) once read.
  state.alboria = { status: "connecting", identity: null, account: null, error: null };

  // The gamertag typed in the Account section and not saved yet (null when there is none): kept
  // apart so that a redraw, or the on-screen keyboard writing it back, never loses it.
  let gamertagDraft = null;

  // The dialog's flow: the step on screen and what the code step is for.
  const flow = { step: null, purpose: null, email: "", gamertag: "", resendAt: 0, from: "choose", dialog: false };
  let resendTimer = null;

  /** The message of a failed request, in the interface's language. */
  const errorMessage = (error) => {
    const code = KNOWN_ERRORS.has(error?.error) ? error.error : "generic";
    return t(`accountErrors.${code}`, {
      attempts: error?.attemptsLeft ?? 0,
      seconds: error?.retryIn ?? 60,
      code: error?.error ?? "?",
    });
  };

  /** The step's error line: `message`, or nothing. */
  const showError = (message) => {
    const line = welcome.querySelector(`[data-step="${flow.step}"] [data-step-error]`);
    if (!line) return;
    line.textContent = message ?? "";
    line.hidden = !message;
    $("code-input").closest(".code-field").classList.toggle("is-error", Boolean(message) && flow.step === "code");
  };

  /** Disables the dialog's buttons while a request runs. */
  const busy = async (work) => {
    box.classList.add("is-busy");
    const buttons = [...welcome.querySelectorAll("button")];
    buttons.forEach((button) => (button.disabled = true));
    try {
      return await work();
    } finally {
      box.classList.remove("is-busy");
      buttons.forEach((button) => (button.disabled = false));
      renderResend();
    }
  };

  // --- The dialog --------------------------------------------------------------------------

  /** Shows `step` in the dialog (opening it), and focuses its first field or button. */
  const showStep = (step) => {
    flow.step = step;
    // The page's header and its Alborian follow the step (styles.css).
    welcome.dataset.atStep = step;
    welcome.hidden = false;
    for (const section of welcome.querySelectorAll("[data-step]")) section.hidden = section.dataset.step !== step;
    showError(null);
    if (step === "code") renderCode();
    if (step === "create" || step === "signin") refreshCodes();
    renderResend();
    const first = welcome.querySelector(`[data-step="${step}"] :is(input, button.choice, button[type="submit"])`);
    first?.focus({ preventScroll: true });
  };

  /** Closes the dialog. The welcome screen closes only once the player has chosen. */
  const closeDialog = () => {
    if (!state.account) {
      showStep("choose");
      return;
    }
    welcome.hidden = true;
    flow.step = null;
    clearInterval(resendTimer);
  };

  /** Opens the welcome screen at its first step, with the gamertag the launcher has. */
  const openWelcome = (message = null) => {
    flow.dialog = false;
    $("create-consent").checked = false;
    $("create-gamertag").value = state.userName;
    $("welcome-local-gamertag").value = state.userName;
    showStep("choose");
    showError(message);
  };

  /** Opens one of the account's dialogs (create or sign in from a local account, email, delete). */
  const openDialog = (step) => {
    flow.dialog = true;
    $("create-consent").checked = false;
    $("create-gamertag").value = state.userName;
    showStep(step);
  };

  // --- Codes off -----------------------------------------------------------------------------

  /**
   * Asks the service whether it sends codes now and shows what that changes in the create and
   * sign-in steps: a note, and Create account instead of Send code. Unknown (no answer) counts
   * as on: the request itself then tells.
   */
  const refreshCodes = async () => {
    let codesOff = false;
    try {
      codesOff = (await alboriaStatus()).emailCodes === false;
    } catch (error) {
      console.warn("Reading the Alboria status failed:", error?.error ?? error);
    }
    $("create-codes-off").hidden = !codesOff;
    $("signin-codes-off").hidden = !codesOff;
    $("create-submit").textContent = t(codesOff ? "welcome.createNow" : "welcome.sendCode");
  };

  // --- The code step -----------------------------------------------------------------------

  const renderCode = () => {
    $("code-text").textContent = t("welcome.codeText", { email: flow.email });
    const input = $("code-input");
    input.value = "";
    renderCodeBoxes();
  };

  /** Draws the six boxes from the field's digits; the next one to fill is marked. */
  const renderCodeBoxes = () => {
    const digits = [...$("code-input").value];
    [...welcome.querySelectorAll(".code-boxes span")].forEach((cell, i) => {
      cell.textContent = digits[i] ?? "";
      cell.classList.toggle("is-next", i === Math.min(digits.length, 5));
    });
  };

  /** "Resend code", disabled with the seconds left until another one can be asked for. */
  const renderResend = () => {
    const button = $("code-resend");
    const left = Math.ceil((flow.resendAt - Date.now()) / 1000);
    button.disabled = left > 0 || box.classList.contains("is-busy");
    button.textContent = left > 0 ? t("welcome.resendIn", { seconds: left }) : t("welcome.resend");
  };

  /**
   * Asks for a code for the flow's purpose and email, and shows the code step; or, when the
   * service needs no code now (codes off), goes on without one.
   */
  const sendCode = async () => {
    const language = getLocale();
    const sent = await busy(async () => {
      if (flow.purpose === "email") return alboriaEmailCode(flow.email, language);
      if (flow.purpose === "delete") return alboriaDeleteCode(language);
      if (flow.purpose === "verify") return alboriaVerifyEmailCode(language);
      return alboriaRequestCode({ purpose: flow.purpose, email: flow.email, gamertag: flow.gamertag || null, language });
    });
    if (sent?.codeNeeded === false) {
      await finish(null);
      return;
    }
    flow.resendAt = Date.now() + (sent?.resendIn ?? 60) * 1000;
    clearInterval(resendTimer);
    resendTimer = setInterval(() => {
      renderResend();
      if (Date.now() >= flow.resendAt) clearInterval(resendTimer);
    }, 1000);
    showStep("code");
    // A development server (the local Alboria) sends the code itself: filled in and used at once.
    if (/^\d{6}$/.test(sent?.devCode ?? "")) {
      $("code-input").value = sent.devCode;
      renderCodeBoxes();
      await attempt(confirmCode);
    }
  };

  /** Runs a step's request, showing its error in the step when it fails. */
  const attempt = async (work) => {
    try {
      showError(null);
      await work();
    } catch (error) {
      console.warn("Alboria request failed:", error?.error ?? error);
      showError(errorMessage(error));
      if (flow.step === "code") {
        $("code-input").value = "";
        renderCodeBoxes();
      }
    }
  };

  /** The player signed in (or created the account): the identity, the choice saved. */
  const signedIn = (identity) => {
    state.alboria = { status: "connected", identity, account: null, error: null };
    state.account = ACCOUNT_ALBORIA;
    state.userName = identity.gamertag;
    persist();
    renderAll();
    loadAccount();
  };

  /** Uses the code typed for the flow's purpose. */
  const confirmCode = async () => {
    const code = $("code-input").value;
    if (code.length !== 6) {
      showError(t("accountErrors.codeLength"));
      return;
    }
    await finish(code);
  };

  /** Does what the flow is for, with `code`; null when the service needs none now. */
  const finish = async (code) => {
    if (flow.purpose === "create" || flow.purpose === "signin") {
      const identity = await busy(() =>
        alboriaVerify({
          purpose: flow.purpose,
          email: flow.email,
          code,
          gamertag: flow.gamertag || state.userName,
          language: getLocale(),
        }),
      );
      signedIn(identity);
      $("done-title").textContent = t(flow.purpose === "create" ? "welcome.createdTitle" : "welcome.linkedTitle");
      $("done-text").textContent = identity.emailVerified === false
        ? `${t("welcome.createdText")} ${t("welcome.createdUnverified")}`
        : t(flow.purpose === "create" ? "welcome.createdText" : "welcome.linkedText");
      $("done-name").textContent = identity.gamertag;
      $("done-tag").textContent = `#${identity.tag}`;
      showStep("done");
      return;
    }
    if (flow.purpose === "email") {
      await busy(() => alboriaChangeEmail(flow.email, code));
      closeDialog();
      toast(t("account.emailChanged"));
      loadAccount();
      return;
    }
    if (flow.purpose === "delete") {
      await busy(() => alboriaDelete(code));
      leaveAccount();
      toast(t("account.deleted"));
      return;
    }
    if (flow.purpose === "verify") {
      await busy(() => alboriaVerifyEmail(code));
      closeDialog();
      toast(t("account.emailVerified"));
      loadAccount();
    }
  };

  /**
   * The player is no longer signed in here (signed out, deleted the account, or the account no
   * longer has this device): the welcome screen asks again, with `message` when there is one.
   */
  const leaveAccount = (message = null) => {
    gamertagDraft = null;
    state.account = null;
    state.alboria = { status: "connecting", identity: null, account: null, error: null };
    persist();
    renderAll();
    openWelcome(message);
  };

  // --- The Account section ------------------------------------------------------------------

  /** Signs in with the launcher's gamertag, to show the identity and the service's state. */
  const connect = async () => {
    if (state.account !== ACCOUNT_ALBORIA) return;
    state.alboria = { ...state.alboria, status: "connecting", error: null };
    renderAll();
    try {
      const identity = await alboriaSignIn(state.userName.trim() || t("install.defaultUserName"));
      state.alboria = { ...state.alboria, status: "connected", identity };
      renderAll();
      loadAccount();
    } catch (error) {
      // The account no longer has this device (removed from another one, or deleted): ask again.
      if (error?.error === "no_account") {
        leaveAccount(t("accountErrors.unlinked"));
        return;
      }
      state.alboria = { ...state.alboria, status: error?.error === "network" ? "offline" : "error", error };
      renderAll();
    }
  };

  /** Reads the account's email and devices. */
  const loadAccount = async () => {
    try {
      state.alboria = { ...state.alboria, account: await alboriaAccount() };
    } catch (error) {
      console.warn("Reading the Alboria account failed:", error?.error ?? error);
    }
    renderAccount();
  };

  const formatDate = (iso) => new Intl.DateTimeFormat(getLocale(), { dateStyle: "long" }).format(new Date(iso));
  const formatDateTime = (iso) =>
    new Intl.DateTimeFormat(getLocale(), { dateStyle: "long", timeStyle: "short" }).format(new Date(iso));

  /** One device of the account, with Remove for the others. */
  const deviceRow = (device) => {
    const row = document.createElement("div");
    row.className = "device";
    const info = document.createElement("div");
    info.className = "device-info";
    const name = document.createElement("strong");
    name.textContent = t("account.device", { platform: device.platform, game: GAME_NAMES[device.title_id] ?? device.title_id });
    const when = document.createElement("span");
    when.textContent = device.current
      ? t("account.since", { date: formatDate(device.created_at) })
      : t("account.lastUsed", { date: formatDate(device.last_seen_at) });
    info.append(name, when);
    row.append(info);
    if (device.current) {
      const current = document.createElement("span");
      current.className = "device-current";
      current.textContent = t("account.thisDevice");
      row.append(current);
    } else {
      const remove = document.createElement("button");
      remove.className = "btn btn-small";
      remove.textContent = t("account.remove");
      remove.addEventListener("click", async () => {
        try {
          await alboriaRemoveDevice(device.public_key);
          loadAccount();
        } catch (error) {
          toast(errorMessage(error));
        }
      });
      row.append(remove);
    }
    return row;
  };

  const renderAccount = () => {
    const alboria = state.account === ACCOUNT_ALBORIA;
    // In Play too, once the email must be verified by a deadline: gone when it is verified, or
    // when the account is no longer this launcher's (signed out, or deleted at the deadline).
    const deadline = alboria && state.alboria.account?.email_verified === false ? state.alboria.account.verify_by : null;
    $("play-verify").hidden = !deadline;
    if (deadline) $("play-verify-text").textContent = t("account.playVerifyBy", { date: formatDateTime(deadline) });
    $("account-alboria").hidden = !alboria;
    $("account-local").hidden = state.account !== ACCOUNT_LOCAL;
    const local = $("local-gamertag");
    if (document.activeElement !== local) local.value = state.userName;
    if (!alboria) return;

    const { status, identity, account, error } = state.alboria;
    const badge = $("account-badge");
    badge.dataset.state = { connected: "ok", connecting: "warn" }[status] ?? "error";
    $("account-badge-text").textContent = t(
      { connected: "account.connected", connecting: "account.connecting", offline: "account.offline" }[status] ?? "account.error",
    );
    const field = $("account-gamertag");
    if (field.value !== (gamertagDraft ?? state.userName)) field.value = gamertagDraft ?? state.userName;
    const editing = field.value.trim() !== (identity?.gamertag ?? state.userName.trim());
    $("account-name").textContent = field.value.trim() || state.userName;
    const tag = $("account-tag");
    tag.textContent = identity && !editing ? `#${identity.tag}` : status === "connecting" ? "#…" : "#—";
    tag.classList.toggle("is-unknown", !identity || editing);
    let detail = "";
    if (status === "offline") detail = t("account.offlineDetail");
    else if (status === "error") detail = errorMessage(error);
    else if (editing && identity) detail = t("account.gamertagHint", { name: field.value.trim(), tag: identity.tag });
    $("account-detail").textContent = detail;
    $("account-save").hidden = !editing || !field.value.trim();
    $("account-retry").hidden = status !== "offline" && status !== "error";
    $("account-email").value = account?.email ?? "";
    // An account made while codes were off: verify by the deadline, or, before there is one, later.
    const unverified = account?.email_verified === false;
    $("account-verify").hidden = !unverified;
    if (unverified) {
      $("account-verify-text").textContent = account.verify_by
        ? t("account.verifyBy", { date: formatDateTime(account.verify_by) })
        : t("account.verifyLater");
      $("account-verify-email").hidden = !account.verify_by;
    }
    $("account-change-email").disabled = !account;
    $("account-export").disabled = status !== "connected";
    $("account-sign-out").disabled = status !== "connected";
    $("account-delete").disabled = !account;
    $("account-devices-panel").hidden = !account;
    $("account-devices").replaceChildren(...(account?.devices ?? []).map(deviceRow));
  };

  const renderAll = () => {
    renderAccount();
    renderPlay();
  };

  // --- Events --------------------------------------------------------------------------------

  /** Keeps only what a field's rule accepts, however it was typed or pasted. */
  const cleanField = (field) => {
    const rule = TEXT_RULES[field.dataset.textRule];
    if (!rule) return;
    const clean = rule.clean(field.value);
    if (clean !== field.value) field.value = clean;
  };
  for (const field of welcome.querySelectorAll("input[data-text-rule]")) {
    field.addEventListener("input", () => cleanField(field));
  }
  $("code-input").addEventListener("input", () => {
    renderCodeBoxes();
    showError(null);
    // The sixth digit confirms by itself.
    if ($("code-input").value.length === 6) attempt(confirmCode);
  });

  welcome.addEventListener("click", (event) => {
    const go = event.target.closest("[data-welcome-go]");
    if (go) {
      flow.from = flow.step;
      showStep(go.dataset.welcomeGo);
      return;
    }
    if (event.target.closest("[data-welcome-back]")) {
      // From the code, back to the step that asked for it; from the others, to the first one, or
      // closed when it is one of the account's dialogs.
      if (flow.step === "code" && flow.purpose !== "verify") showStep(flow.purpose);
      else if (flow.dialog) closeDialog();
      else showStep("choose");
      return;
    }
    if (event.target.closest("[data-welcome-close]")) closeDialog();
  });

  // Each form step submits with Enter, its main button or, with a controller, A on that button.
  welcome.addEventListener("submit", (event) => {
    event.preventDefault();
    const step = event.target.dataset.step;
    if (step === "create") {
      const gamertag = $("create-gamertag").value.trim();
      const email = $("create-email").value.trim();
      if (!gamertag || !email) return showError(t("accountErrors.bad_request"));
      if (!$("create-consent").checked) return showError(t("accountErrors.consent"));
      Object.assign(flow, { purpose: "create", email, gamertag });
      attempt(sendCode);
    } else if (step === "signin") {
      const email = $("signin-email").value.trim();
      if (!email) return showError(t("accountErrors.bad_request"));
      Object.assign(flow, { purpose: "signin", email, gamertag: "" });
      attempt(sendCode);
    } else if (step === "email") {
      const email = $("new-email").value.trim();
      if (!email) return showError(t("accountErrors.bad_request"));
      Object.assign(flow, { purpose: "email", email, gamertag: "" });
      attempt(sendCode);
    } else if (step === "delete") {
      Object.assign(flow, { purpose: "delete", email: state.alboria.account?.email ?? "", gamertag: "" });
      attempt(sendCode);
    } else if (step === "approve") {
      attempt(checkBrowser);
    } else if (step === "approve-confirm") {
      attempt(() => answerBrowser(true));
    } else if (step === "code") {
      attempt(confirmCode);
    } else if (step === "local") {
      const gamertag = TEXT_RULES.gamertag.clean($("welcome-local-gamertag").value).trim();
      state.userName = gamertag || state.userName;
      state.account = ACCOUNT_LOCAL;
      persist();
      renderAll();
      closeDialog();
    }
  });

  $("code-resend").addEventListener("click", () => attempt(sendCode));

  // Verify email (an account made while codes were off): a code to the account's email, from the
  // Account section or from Play.
  const verifyEmail = async () => {
    flow.dialog = true;
    Object.assign(flow, { purpose: "verify", email: state.alboria.account?.email ?? "", gamertag: "" });
    try {
      await sendCode();
    } catch (error) {
      // The dialog is not open yet: a toast says why.
      toast(errorMessage(error));
    }
  };
  $("account-verify-email").addEventListener("click", verifyEmail);
  $("play-verify-email").addEventListener("click", verifyEmail);

  // The Account section.
  const gamertagField = $("account-gamertag");
  gamertagField.addEventListener("input", () => {
    cleanField(gamertagField);
    gamertagDraft = gamertagField.value;
    renderAccount();
  });
  const saveGamertag = async () => {
    const gamertag = gamertagField.value.trim();
    if (!gamertag) return;
    gamertagDraft = null;
    state.userName = gamertag;
    persist();
    try {
      state.alboria = { ...state.alboria, identity: await alboriaSignIn(gamertag), status: "connected", error: null };
      state.userName = state.alboria.identity.gamertag;
    } catch (error) {
      // Saved here anyway: the account takes the name the next time the device signs in.
      state.alboria = { ...state.alboria, status: error?.error === "network" ? "offline" : "error", error };
    }
    renderAll();
  };
  $("account-save").addEventListener("click", saveGamertag);
  gamertagField.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !$("account-save").hidden) saveGamertag();
  });
  $("account-retry").addEventListener("click", connect);
  $("account-change-email").addEventListener("click", () => {
    $("new-email").value = "";
    openDialog("email");
  });
  $("account-delete").addEventListener("click", () => {
    $("delete-text").textContent = t("account.deleteText", { email: state.alboria.account?.email ?? "" });
    openDialog("delete");
  });
  // Approving a browser on Alboria's website: the website shows a code, the player types it here,
  // sees which browser asks and roughly from where, and allows it or not. The approval always
  // happens here, on a device already signed in, so a code read out to someone else is worth nothing.
  let approving = "";
  const checkBrowser = async () => {
    const code = $("approve-code").value.trim().toUpperCase();
    if (code.replace(/[^A-Z0-9]/g, "").length !== 8) return showError(t("accountErrors.no_link"));
    const link = await alboriaWebLink(code, getLocale());
    approving = code;
    $("approve-who").textContent = t("account.approveWho", {
      browser: link.browser,
      place: link.place ?? t("account.approveSomewhere"),
      code,
    });
    showStep("approve-confirm");
  };
  const answerBrowser = async (allow) => {
    await alboriaWebLinkAnswer(approving, allow);
    approving = "";
    closeDialog();
    toast(t(allow ? "account.approveAllowed" : "account.approveRefused"));
  };
  $("approve-code").addEventListener("input", () => {
    // Letters in capitals, and the hyphen after the first four, as the website shows the code.
    const raw = $("approve-code").value.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 8);
    $("approve-code").value = raw.length > 4 ? `${raw.slice(0, 4)}-${raw.slice(4)}` : raw;
    showError(null);
  });
  $("approve-refuse").addEventListener("click", () => attempt(() => answerBrowser(false)));
  $("account-approve-browser").addEventListener("click", () => {
    $("approve-code").value = "";
    openDialog("approve");
  });
  $("account-export").addEventListener("click", async () => {
    try {
      if (await alboriaExport()) toast(t("account.exported"));
    } catch (error) {
      toast(errorMessage(error));
    }
  });
  $("account-sign-out").addEventListener("click", async () => {
    const current = state.alboria.account?.devices?.find((device) => device.current);
    try {
      if (current) await alboriaRemoveDevice(current.public_key);
      leaveAccount();
      toast(t("account.signedOut"));
    } catch (error) {
      toast(errorMessage(error));
    }
  });

  // A local account keeps its gamertag as it is typed, as the launcher always did.
  const localField = $("local-gamertag");
  localField.addEventListener("input", () => {
    cleanField(localField);
    state.userName = localField.value;
    persist();
    renderPlay();
  });
  // The links to alboria.org: under the brand and in the Account section.
  for (const button of document.querySelectorAll("[data-alboria-site]")) {
    button.addEventListener("click", () => openAlboriaSite().catch((error) => console.warn("Opening alboria.org failed:", error)));
  }
  for (const button of document.querySelectorAll("[data-alboria-blog]")) {
    button.addEventListener("click", () => openAlboriaBlog(getLocale()).catch((error) => console.warn("Opening the blog failed:", error)));
  }
  for (const button of document.querySelectorAll("[data-alboria-discord]")) {
    button.addEventListener("click", () => openAlboriaDiscord().catch((error) => console.warn("Opening the Discord server failed:", error)));
  }
  for (const button of document.querySelectorAll("[data-account-open]")) {
    button.addEventListener("click", () => openDialog(button.dataset.accountOpen));
  }

  return {
    /** Shows the welcome screen when the player has not chosen yet, or signs in when they have an account. */
    start: () => {
      renderAll();
      if (!state.account) openWelcome();
      else connect();
    },
    render: () => {
      renderAccount();
      if (flow.step === "code") $("code-text").textContent = t("welcome.codeText", { email: flow.email });
      renderResend();
    },
    /** The Play section's player line: the identity, the gamertag of a local account, or nothing yet. */
    playerLine: () => {
      const name = state.userName.trim() || t("install.defaultUserName");
      if (state.account === ACCOUNT_LOCAL) return t("status.playerLocal", { name });
      const identity = state.alboria.identity;
      if (state.account === ACCOUNT_ALBORIA && identity) return t("status.playerOnline", { name: `${identity.gamertag}#${identity.tag}` });
      return name;
    },
  };
};
