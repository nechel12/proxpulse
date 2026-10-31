import { IPAPI_FREE_URL, IPAPI_OLD_HTTPS_URL, maskKey, resolveTestUrl } from "./ipapi.ts";
import { resolveJudgeUrl } from "./judge.ts";
import {
  PUBLIC_CUSTOM,
  sanitizeProfile,
  type JudgeServer,
  type ProfileData,
  type TargetMode,
} from "./profiles.ts";

export const JUDGE_DEFAULT = "https://proxycheck.lmtunnel.com";

type Translate = (key: string, params?: Record<string, string | number>) => string;

type Deps = {
  t: Translate;
  schedule: () => void;
};

const el = <T extends HTMLElement>(id: string) => document.getElementById(id) as T | null;

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export class SettingsPanel {
  judgeFull = true;

  constructor(private deps: Deps) {}

  targetMode(): TargetMode {
    const active = document.querySelector<HTMLButtonElement>("#target-mode .seg-btn.active");
    return active?.dataset.target === "judge" ? "judge" : "public";
  }

  setTargetMode(mode: TargetMode, silent = false): void {
    document.querySelectorAll<HTMLButtonElement>("#target-mode .seg-btn").forEach((b) => {
      const active = b.dataset.target === mode;
      b.classList.toggle("active", active);
      b.setAttribute("aria-pressed", String(active));
    });
    this.updateTargetUI();
    if (!silent) this.deps.schedule();
  }

  isJudgeMode(): boolean {
    return this.targetMode() === "judge";
  }

  judgeServer(): JudgeServer {
    return el<HTMLSelectElement>("judge-server")?.value === "custom" ? "custom" : "builtin";
  }

  judgeBase(): string {
    if (this.judgeServer() === "custom") {
      const custom = (el<HTMLInputElement>("judge-url")?.value ?? "").trim();
      // An empty custom server is a real error, not a silent fallback to the
      // built-in instance.
      return custom ? resolveJudgeUrl(custom, this.judgeFull, JUDGE_DEFAULT) : "";
    }
    return resolveJudgeUrl("", this.judgeFull, JUDGE_DEFAULT);
  }

  effectiveTestUrl(): string {
    if (this.isJudgeMode()) return this.judgeBase();
    const sel = (el<HTMLSelectElement>("test-url")?.value ?? "").trim();
    const custom = (el<HTMLInputElement>("test-url-custom")?.value ?? "").trim();
    const selected = sel === PUBLIC_CUSTOM ? custom : sel;
    const pro = el<HTMLInputElement>("chk-ipapi-pro")?.checked ?? false;
    const key = el<HTMLInputElement>("ipapi-key")?.value ?? "";
    return resolveTestUrl(selected, "", pro, key);
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
    // The Pro toggle and key change the effective URL too.
    this.updateTargetUI();
    this.deps.schedule();
  }

  private isIpapiSelected(): boolean {
    const sel = el<HTMLSelectElement>("test-url")?.value ?? "";
    return sel === IPAPI_FREE_URL || sel === IPAPI_OLD_HTTPS_URL;
  }

  private renderJudgeMode(): void {
    const fast = el<HTMLButtonElement>("judge-fast");
    const full = el<HTMLButtonElement>("judge-full");
    fast?.classList.toggle("active", !this.judgeFull);
    full?.classList.toggle("active", this.judgeFull);
    fast?.setAttribute("aria-pressed", String(!this.judgeFull));
    full?.setAttribute("aria-pressed", String(this.judgeFull));
  }

  private refreshEffectiveUrl(): void {
    const out = el<HTMLElement>("effective-url");
    if (!out) return;
    const url = maskKey(this.effectiveTestUrl());
    const label = this.deps.t("effective_url_t");
    out.textContent = url || "—";
    out.title = url ? `${label}\n${url}` : label;
    out.classList.toggle("bad", !/^https?:\/\//i.test(url));
  }

  updateTargetUI(): void {
    const mode = this.targetMode();
    const judge = mode === "judge";
    document.querySelectorAll<HTMLButtonElement>("#target-mode .seg-btn").forEach((b) => {
      b.setAttribute("aria-pressed", String(b.dataset.target === mode));
    });
    const server = this.judgeServer();
    const customPublic = (el<HTMLSelectElement>("test-url")?.value ?? "") === PUBLIC_CUSTOM;
    document.getElementById("target-public")?.classList.toggle("hidden", judge);
    document.getElementById("target-judge")?.classList.toggle("hidden", !judge);
    document.getElementById("test-url-custom-wrap")?.classList.toggle("hidden", judge || !customPublic);
    document.getElementById("ipapi-block")?.classList.toggle("hidden", judge || !this.isIpapiSelected());
    document.getElementById("deep-block")?.classList.toggle("hidden", judge);
    document.getElementById("judge-url-wrap")?.classList.toggle("hidden", !judge || server !== "custom");

    const builtin = document.getElementById("opt-judge-builtin");
    if (builtin) builtin.textContent = this.deps.t("judge_builtin", { host: hostOf(JUDGE_DEFAULT) });

    let insecure = false;
    let tunneled = false;
    if (judge) {
      try {
        const u = new URL(this.judgeBase());
        const local = ["localhost", "127.0.0.1", "::1", "[::1]"].includes(u.hostname);
        insecure = u.protocol === "http:" && !local;
        tunneled = u.protocol === "https:";
      } catch { /* invalid URL is reported by the backend */ }
    }
    document.getElementById("judge-http-hint")?.classList.toggle("hidden", !insecure);
    document.getElementById("judge-https-hint")?.classList.toggle("hidden", !tunneled);

    this.renderJudgeMode();
    this.updateIpapiUI();
    this.refreshEffectiveUrl();
  }

  setJudgeFull(full: boolean): void {
    this.judgeFull = full;
    this.renderJudgeMode();
    this.refreshEffectiveUrl();
    this.deps.schedule();
  }

  snapshotProfile(): ProfileData {
    const val = (id: string) => el<HTMLInputElement>(id)?.value ?? "";
    const chk = (id: string) => el<HTMLInputElement>(id)?.checked ?? false;
    return {
      target: this.targetMode(),
      testUrlSel: el<HTMLSelectElement>("test-url")?.value ?? "",
      customUrl: val("test-url-custom"),
      judgeServer: this.judgeServer(),
      judgeUrl: val("judge-url"),
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
    const server = el<HTMLSelectElement>("judge-server");
    if (server) server.value = d.judgeServer;
    setVal("judge-url", d.judgeUrl);
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
    this.judgeFull = d.judgeFull;
    this.setTargetMode(d.target, true);
    this.deps.schedule();
  }

  saveCurrentSettings(): void {
    try {
      localStorage.setItem("pp-settings", JSON.stringify(this.snapshotProfile()));
    } catch { /* ignore */ }
  }

  loadCurrentSettings(): boolean {
    try {
      const raw = localStorage.getItem("pp-settings");
      if (!raw) return false;
      const data = sanitizeProfile(JSON.parse(raw));
      if (!data) return false;
      this.applyProfile(data);
      return true;
    } catch {
      return false;
    }
  }
}
