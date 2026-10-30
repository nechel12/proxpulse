import {
  BUILTIN_PROFILES,
  sanitizeProfile,
  type ProfileData,
} from "./profiles.ts";

export type ProfileEntry = { name: string; data: ProfileData };
export type Translate = (key: string, params?: Record<string, string | number>) => string;

export class ProfilePanel {
  custom: ProfileEntry[] = [];
  currentId = "";

  load(): void {
    this.custom = [];
    try {
      const raw = localStorage.getItem("pp-profiles");
      const arr = raw ? JSON.parse(raw) : null;
      if (!Array.isArray(arr)) return;
      for (const p of arr) {
        if (!p || typeof p.name !== "string" || !p.name.trim()) continue;
        const data = sanitizeProfile(p.data);
        if (!data) continue;
        this.custom.push({ name: p.name.trim().slice(0, 40), data });
        if (this.custom.length >= 20) break;
      }
    } catch {
      this.custom = [];
    }
  }

  save(): void {
    try {
      localStorage.setItem("pp-profiles", JSON.stringify(this.custom.slice(0, 20)));
    } catch { /* ignore */ }
  }

  find(id: string): ProfileData | null {
    const b = BUILTIN_PROFILES.find((x) => x.id === id);
    if (b) return b.data;
    if (id.startsWith("custom:")) {
      const c = this.custom.find((x) => "custom:" + x.name === id);
      if (c) return c.data;
    }
    return null;
  }

  upsert(name: string, data: ProfileData): boolean {
    const at = this.custom.findIndex((c) => c.name === name);
    if (at >= 0) this.custom[at] = { name, data };
    else if (this.custom.length < 20) this.custom.push({ name, data });
    else return false;
    return true;
  }

  deleteId(id: string): void {
    if (!id.startsWith("custom:")) return;
    this.custom = this.custom.filter((c) => "custom:" + c.name !== id);
    if (this.currentId === id) this.currentId = "";
  }

  render(t: Translate): void {
    const wrap = document.getElementById("preset-chips");
    if (!wrap) return;
    wrap.innerHTML = "";
    for (const b of BUILTIN_PROFILES) {
      wrap.appendChild(this.chip(b.id, t(b.nameKey), t(b.hintKey)));
    }
    for (const c of this.custom) {
      const id = "custom:" + c.name;
      const group = document.createElement("div");
      group.className = "preset-group";
      group.appendChild(this.chip(id, c.name, ""));
      const del = document.createElement("button");
      del.type = "button";
      del.className = "preset-x";
      del.textContent = "×";
      del.title = t("preset_delete");
      del.setAttribute("aria-label", `${t("preset_delete")}: ${c.name}`);
      del.dataset.presetDelete = id;
      group.appendChild(del);
      wrap.appendChild(group);
    }
  }

  private chip(id: string, label: string, hint: string): HTMLButtonElement {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "preset-chip";
    b.textContent = label;
    b.title = hint;
    b.dataset.presetId = id;
    const active = id === this.currentId;
    b.classList.toggle("active", active);
    b.setAttribute("aria-pressed", String(active));
    return b;
  }
}
