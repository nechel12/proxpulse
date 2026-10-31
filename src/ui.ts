export function screenVisible(id: string): boolean {
  const el = document.getElementById(id);
  return !!el && !el.classList.contains("hidden");
}

export function setProgress(done: number, total: number): void {
  const p = total === 0 ? 0 : Math.round((done / total) * 100);
  const el = document.getElementById("progress");
  if (el) el.style.width = `${p}%`;
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

export function askText(title: string, value = ""): Promise<string | null> {
  const overlay = document.getElementById("text-modal");
  const input = document.getElementById("text-modal-input") as HTMLInputElement | null;
  const ok = document.getElementById("text-modal-ok") as HTMLButtonElement | null;
  const cancel = document.getElementById("text-modal-cancel") as HTMLButtonElement | null;
  const titleEl = document.getElementById("text-modal-title");
  if (!overlay || !input || !ok || !cancel || !titleEl) return Promise.resolve(null);
  titleEl.textContent = title;
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
      input.onkeydown = null;
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
      } else if (e.key === "Escape") {
        e.preventDefault();
        done(null);
      }
    };
  });
}
