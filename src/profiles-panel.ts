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

  deleteCurrent(): void {
    if (!this.currentId.startsWith("custom:")) return;
    this.custom = this.custom.filter((c) => "custom:" + c.name !== this.currentId);
    this.currentId = "";
  }

  render(t: Translate): void {
    const sel = document.getElementById("profile-sel") as HTMLSelectElement | null;
    if (!sel) return;
    sel.innerHTML = "";
    const ph = document.createElement("option");
    ph.value = "";
    ph.textContent = t("profile_none");
    sel.appendChild(ph);
    for (const b of BUILTIN_PROFILES) {
      const o = document.createElement("option");
      o.value = b.id;
      o.textContent = t(b.nameKey);
      sel.appendChild(o);
    }
    for (const c of this.custom) {
      const o = document.createElement("option");
      o.value = "custom:" + c.name;
      o.textContent = c.name;
      sel.appendChild(o);
    }
    sel.value = this.currentId;
    const del = document.getElementById("btn-profile-del") as HTMLButtonElement | null;
    if (del) del.disabled = !this.currentId.startsWith("custom:");
  }
}
