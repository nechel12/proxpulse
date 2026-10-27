import { invoke } from "@tauri-apps/api/core";

type Deps = {
  getData: () => Record<string, unknown>;
};

export function initInfoPanel({ getData }: Deps): void {
  void invoke<string>("app_version")
    .then((v) => {
      const app = document.getElementById("app-ver");
      if (app) app.textContent = v;
      const brand = document.getElementById("brand-ver");
      if (brand) brand.textContent = `proxy checker // v${v}`;
      const info = document.getElementById("brand-ver-info");
      if (info) info.textContent = `proxy checker // v${v}`;
    })
    .catch(() => { /* ignore */ });

  document.getElementById("btn-copy-diag")?.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(JSON.stringify(getData(), null, 2));
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
