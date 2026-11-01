import { invoke } from "@tauri-apps/api/core";
import { flashLabel } from "./ui.ts";

type Translate = (key: string, params?: Record<string, string | number>) => string;

type Deps = {
  t: Translate;
  getData: () => Record<string, unknown>;
};

export function initInfoPanel({ t, getData }: Deps): void {
  void invoke<string>("app_version")
    .then((v) => {
      const app = document.getElementById("app-ver");
      if (app) app.textContent = v;
      const brand = document.getElementById("brand-ver");
      if (brand) brand.textContent = `proxy checker // v${v}`;
    })
    .catch(() => { /* ignore */ });

  document.getElementById("btn-copy-diag")?.addEventListener("click", async (e) => {
    const button = e.currentTarget as HTMLElement;
    // Bug reports ask for the OS and the install type; include both.
    const install = await invoke<string>("install_kind").catch(() => null);
    try {
      await navigator.clipboard.writeText(JSON.stringify({ ...getData(), install }, null, 2));
      flashLabel(button, t("copied"));
    } catch { /* ignore */ }
  });

  const openGh = (url: string) => async () => {
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch {
      window.open(url, "_blank");
    }
  };
  document.getElementById("gh-card")?.addEventListener("click", openGh("https://github.com/nechel12/proxpulse"));
  document.getElementById("gh-card-judge")?.addEventListener("click", openGh("https://github.com/nechel12/proxpulse-judge"));
}
