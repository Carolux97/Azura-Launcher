/**
 * The "Report a problem" panel in Installation: the diagnostic file (src-tauri/src/diagnostics.rs).
 *
 * "Create diagnostic file" lists what the file would hold, each file with its size and a box the
 * player can untick (the system's crash reports start unticked). "Save" asks where, writes it and
 * says where it is and how big, with a button that shows it in the file manager. What the file
 * leaves out is said under the list.
 */

import { createDiagnostic, diagnosticPlan, showDiagnostic } from "./platform.js";

/**
 * Wires the panel. `formatSize` words a byte count; `gameOutput` gives the game output the
 * launcher showed this session, as lines.
 */
export const setupDiagnostics = ({ t, formatSize, gameOutput }) => {
  const panel = document.getElementById("diagnostic-panel");
  const open = document.getElementById("diagnostic-open");
  const plan = document.getElementById("diagnostic-plan");
  const list = document.getElementById("diagnostic-list");
  const save = document.getElementById("diagnostic-save");
  const cancel = document.getElementById("diagnostic-cancel");
  const result = document.getElementById("diagnostic-result");
  const show = document.getElementById("diagnostic-show");
  if (!panel) return { render: () => {} };

  // The entries listed ([{ name, kind, bytes, included }]), null while the list is closed; the last
  // result ({ key, vars, state }), and whether a file is being written.
  let entries = null;
  let last = null;
  let busy = false;

  const render = () => {
    plan.hidden = !entries;
    open.hidden = Boolean(entries);
    save.disabled = busy || !entries?.some((entry) => entry.included);
    cancel.disabled = busy;
    save.textContent = t(busy ? "diagnostic.saving" : "diagnostic.save");
    list.replaceChildren(
      ...(entries ?? []).map((entry) => {
        const row = document.createElement("li");
        const label = document.createElement("label");
        label.className = "diagnostic-entry";
        const box = document.createElement("input");
        box.type = "checkbox";
        box.checked = entry.included;
        box.disabled = busy;
        box.addEventListener("change", () => {
          entry.included = box.checked;
          render();
        });
        const name = document.createElement("span");
        name.className = "diagnostic-name";
        name.textContent = entry.name;
        const detail = document.createElement("small");
        detail.textContent = [t(`diagnostic.kind.${entry.kind}`), entry.bytes ? formatSize(entry.bytes) : null].filter(Boolean).join(" · ");
        label.append(box, name, detail);
        row.append(label);
        return row;
      }),
    );
    result.hidden = !last;
    if (last) {
      result.textContent = t(last.key, last.vars);
      result.dataset.state = last.state;
    }
    show.hidden = last?.state !== "ok";
  };

  open.addEventListener("click", async () => {
    last = null;
    try {
      entries = await diagnosticPlan();
    } catch (error) {
      console.error("The diagnostic plan failed:", error);
      last = { key: "diagnostic.error.io", vars: { detail: String(error) }, state: "error" };
    }
    render();
    list.querySelector("input")?.focus({ preventScroll: true });
  });

  cancel.addEventListener("click", () => {
    entries = null;
    render();
    open.focus({ preventScroll: true });
  });

  save.addEventListener("click", async () => {
    busy = true;
    render();
    try {
      const leftOut = entries.filter((entry) => !entry.included).map((entry) => entry.name);
      const created = await createDiagnostic(leftOut, gameOutput());
      if (created) {
        entries = null;
        last = { key: "diagnostic.created", vars: { path: created.path, size: formatSize(created.bytes) }, state: "ok" };
      }
    } catch (error) {
      const kind = ["nothingChosen", "diskFull", "unwritable"].includes(error?.kind) ? error.kind : "io";
      last = { key: `diagnostic.error.${kind}`, vars: { detail: error?.detail ?? String(error) }, state: "error" };
    } finally {
      busy = false;
      render();
      (show.hidden ? open.hidden ? save : open : show).focus({ preventScroll: true });
    }
  });

  show.addEventListener("click", () => showDiagnostic().catch((error) => console.error("Showing the diagnostic file failed:", error)));

  render();
  return { render };
};
