export function screenVisible(id: string): boolean {
  const el = document.getElementById(id);
  return !!el && !el.classList.contains("hidden");
}

export function setProgress(done: number, total: number): void {
  const p = total === 0 ? 0 : Math.round((done / total) * 100);
  const el = document.getElementById("progress");
  if (el) el.style.width = `${p}%`;
}

/** Briefly replace a button label to confirm an action (e.g. "Copied"). */
export function flashLabel(el: HTMLElement, text: string, ms = 1500): void {
  const original = el.textContent;
  el.textContent = text;
  window.setTimeout(() => {
    if (el.textContent === text) el.textContent = original;
  }, ms);
}

/** Native confirmation dialog; falls back to `window.confirm` outside Tauri. */
export async function askConfirm(message: string): Promise<boolean> {
  try {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    return await confirm(message, { title: "ProxPulse", kind: "warning" });
  } catch {
    return window.confirm(message);
  }
}

/** A two-way question with named answers; true for the first one. */
export async function askChoice(message: string, yes: string, no: string): Promise<boolean> {
  try {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    return await ask(message, { title: "ProxPulse", kind: "info", okLabel: yes, cancelLabel: no });
  } catch {
    return window.confirm(message);
  }
}

export function askText(title: string, value = "", maxLength = 40): Promise<string | null> {
  const overlay = document.getElementById("text-modal");
  const input = document.getElementById("text-modal-input") as HTMLInputElement | null;
  const ok = document.getElementById("text-modal-ok") as HTMLButtonElement | null;
  const cancel = document.getElementById("text-modal-cancel") as HTMLButtonElement | null;
  const titleEl = document.getElementById("text-modal-title");
  if (!overlay || !input || !ok || !cancel || !titleEl) return Promise.resolve(null);
  // Focus goes back where it was when the dialog closes.
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  titleEl.textContent = title;
  input.maxLength = maxLength;
  input.value = value;
  overlay.classList.remove("hidden");
  input.focus();
  input.select();
  return new Promise((resolve) => {
    const done = (v: string | null) => {
      overlay.classList.add("hidden");
      ok.onclick = null;
      cancel.onclick = null;
      overlay.onclick = null;
      overlay.onkeydown = null;
      input.onkeydown = null;
      opener?.focus({ preventScroll: true });
      resolve(v);
    };
    ok.onclick = () => done(input.value);
    cancel.onclick = () => done(null);
    overlay.onclick = (e) => {
      if (e.target === overlay) done(null);
    };
    input.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        done(input.value);
      }
    };
    // Escape closes the dialog from any of its controls, and Tab stays
    // inside it: the page behind is inert while it is open.
    overlay.onkeydown = (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        done(null);
        return;
      }
      if (e.key !== "Tab") return;
      e.preventDefault();
      const items = [input, cancel, ok];
      const at = items.indexOf(document.activeElement as HTMLInputElement);
      const next = at < 0
        ? (e.shiftKey ? items.length - 1 : 0)
        : (at + (e.shiftKey ? items.length - 1 : 1)) % items.length;
      items[next].focus();
    };
  });
}
