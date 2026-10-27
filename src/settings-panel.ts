import { resolveTestUrl } from "./ipapi.ts";
import { resolveJudgeUrl } from "./judge.ts";
import { sanitizeProfile, type ProfileData } from "./profiles.ts";

export const JUDGE_DEFAULT = "https://proxycheck.lmtunnel.com";

type Translate = (key: string, params?: Record<string, string | number>) => string;

type Deps = {
  t: Translate;
  schedule: () => void;
};

const el = <T extends HTMLElement>(id: string) => document.getElementById(id) as T | null;

export class SettingsPanel {
  judgeFull = true;

  constructor(private deps: Deps) {}

  isJudgeMode(): boolean {
    return (el<HTMLSelectElement>("test-url")?.value ?? "") === "judge";
  }

  judgeBase(): string {
    const custom = el<HTMLInputElement>("test-url-custom")?.value ?? "";
    return resolveJudgeUrl(custom, this.judgeFull, JUDGE_DEFAULT);
  }

  effectiveTestUrl(): string {
    if (this.isJudgeMode()) return this.judgeBase();
    const sel = (el<HTMLSelectElement>("test-url")?.value ?? "").trim();
    const custom = (el<HTMLInputElement>("test-url-custom")?.value ?? "").trim();
    const pro = el<HTMLInputElement>("chk-ipapi-pro")?.checked ?? false;
    const key = el<HTMLInputElement>("ipapi-key")?.value ?? "";
    return resolveTestUrl(sel, custom, pro, key);
  }

  activeIpapiKey(): string | null {
    const pro = el<HTMLInputElement>("chk-ipapi-pro")?.checked ?? false;
    const key = (el<HTMLInputElement>("ipapi-key")?.value ?? "").trim();
    return pro && key ? key : null;
  }

  updateIpapiUI(): void {
    const hint = el<HTMLElement>("ipapi-hint");
    if (!hint) return;
    const pro = el<HTMLInputElement>("chk-ipapi-pro")?.checked ?? false;
    const key = (el<HTMLInputElement>("ipapi-key")?.value ?? "").trim();
    hint.textContent = this.deps.t(pro && !key ? "ipapi_key_missing" : "ipapi_hint");
  }

  loadIpapiSettings(): void {
    try {
      const pro = localStorage.getItem("pp-ipapi-pro") === "1";
      const key = localStorage.getItem("pp-ipapi-key") ?? "";
      const proEl = el<HTMLInputElement>("chk-ipapi-pro");
      const keyEl = el<HTMLInputElement>("ipapi-key");
      if (proEl) proEl.checked = pro;
      if (keyEl) keyEl.value = key;
    } catch { /* localStorage may be unavailable */ }
    this.updateIpapiUI();
  }

  saveIpapiSettings(): void {
    try {
      const pro = el<HTMLInputElement>("chk-ipapi-pro")?.checked ?? false;
      const key = el<HTMLInputElement>("ipapi-key")?.value ?? "";
      localStorage.setItem("pp-ipapi-pro", pro ? "1" : "0");
      localStorage.setItem("pp-ipapi-key", key);
    } catch { /* ignore quota/unavailable */ }
    this.updateIpapiUI();
    this.deps.schedule();
  }

  updateJudgeUI(): void {
    const j = this.isJudgeMode();
    for (const id of ["deep-checks", "deep-hint", "precheck-row", "precheck-hint"]) {
      document.getElementById(id)?.classList.toggle("hidden", j);
    }
    document.getElementById("judge-mode-row")?.classList.toggle("hidden", !j);
    let insecure = false;
    if (j) {
      try {
        const u = new URL(this.judgeBase());
        const local = ["localhost", "127.0.0.1", "::1", "[::1]"].includes(u.hostname);
        insecure = u.protocol === "http:" && !local;
      } catch { /* invalid URL is reported by the backend */ }
    }
    document.getElementById("judge-http-hint")?.classList.toggle("hidden", !insecure);
    const custom = el<HTMLInputElement>("test-url-custom");
    if (custom) custom.placeholder = this.deps.t(j ? "custom_judge_ph" : "custom_ph");
  }

  setJudgeFull(full: boolean): void {
    this.judgeFull = full;
    el<HTMLButtonElement>("judge-fast")?.classList.toggle("active", !full);
    el<HTMLButtonElement>("judge-full")?.classList.toggle("active", full);
    this.deps.schedule();
  }

  snapshotProfile(): ProfileData {
    const val = (id: string) => el<HTMLInputElement>(id)?.value ?? "";
    const chk = (id: string) => el<HTMLInputElement>(id)?.checked ?? false;
    return {
      testUrlSel: el<HTMLSelectElement>("test-url")?.value ?? "",
      customUrl: val("test-url-custom"),
      timeout: val("timeout"),
      concurrency: val("concurrency"),
      repeats: val("repeats"),
      defaultProto: el<HTMLSelectElement>("default-proto")?.value ?? "http",
      geo: chk("chk-geo"),
      anon: chk("chk-anon"),
      tamper: chk("chk-tamper"),
      tls: chk("chk-tls"),
      precheck: chk("chk-precheck"),
      precheckTimeout: val("precheck-timeout"),
      judgeFull: this.judgeFull,
    };
  }

  applyProfile(d: ProfileData): void {
    const setVal = (id: string, v: string) => {
      const input = el<HTMLInputElement>(id);
      if (input) input.value = v;
    };
    const setChk = (id: string, v: boolean) => {
      const input = el<HTMLInputElement>(id);
      if (input) input.checked = v;
    };
    const test = el<HTMLSelectElement>("test-url");
    if (test) test.value = d.testUrlSel;
    setVal("test-url-custom", d.customUrl);
    setVal("timeout", d.timeout);
    setVal("concurrency", d.concurrency);
    setVal("repeats", d.repeats);
    const proto = el<HTMLSelectElement>("default-proto");
    if (proto) proto.value = d.defaultProto;
    setChk("chk-geo", d.geo);
    setChk("chk-anon", d.anon);
    setChk("chk-tamper", d.tamper);
    setChk("chk-tls", d.tls);
    setChk("chk-precheck", d.precheck);
    setVal("precheck-timeout", d.precheckTimeout);
    this.setJudgeFull(d.judgeFull);
    this.updateJudgeUI();
    this.deps.schedule();
  }

  saveCurrentSettings(): void {
    try {
      localStorage.setItem("pp-settings", JSON.stringify(this.snapshotProfile()));
    } catch { /* ignore */ }
  }

  loadCurrentSettings(): void {
    try {
      const raw = localStorage.getItem("pp-settings");
      if (!raw) return;
      const data = sanitizeProfile(JSON.parse(raw));
      if (data) this.applyProfile(data);
    } catch { /* ignore */ }
  }
}
