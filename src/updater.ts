import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Translate = (key: string, params?: Record<string, string | number>) => string;
type StatusSetter = (id: string, render: () => string) => void;

export type UpdaterDeps = {
  t: Translate;
  setStatusText: StatusSetter;
  logErr: (msg: string) => void;
  isTauriRuntime: () => boolean;
  /** Called when a newer version is found. */
  onFound?: (version: string) => void;
};

type UpdateInfo = {
  available: boolean;
  version: string | null;
  current: string;
  notes: string | null;
};

export type Updater = {
  /** Check without showing progress or errors; used at launch. */
  checkQuietly: () => Promise<void>;
};

export function initUpdater({ t, setStatusText, logErr, isTauriRuntime, onFound }: UpdaterDeps): Updater {
  let releaseUrl = "";
  let pendingVersion = "";
  let installKindPromise: Promise<string> | null = null;
  const getInstallKind = (): Promise<string> => {
    if (!installKindPromise) {
      installKindPromise = isTauriRuntime()
        ? invoke<string>("install_kind").catch(() => "auto")
        : Promise.resolve("auto");
    }
    return installKindPromise;
  };

  const check = document.getElementById("btn-check-upd") as HTMLButtonElement | null;
  const install = document.getElementById("btn-install-upd") as HTMLButtonElement | null;
  const open = document.getElementById("btn-open-rel") as HTMLButtonElement | null;
  if (!check || !install || !open) return { checkQuietly: async () => {} };

  const runCheck = async (quiet: boolean) => {
    open.classList.add("hidden");
    install.classList.add("hidden");
    pendingVersion = "";
    if (!quiet) setStatusText("upd-status", () => "...");
    try {
      const u = await invoke<UpdateInfo>("check_app_update");
      const appVer = document.getElementById("app-ver");
      if (appVer) appVer.textContent = u.current;
      if (!u.available || !u.version) {
        if (!quiet) setStatusText("upd-status", () => t("upd_latest", { v: u.current }));
        return;
      }
      const version = u.version;
      pendingVersion = version;
      releaseUrl = "https://github.com/nechel12/proxpulse/releases";
      const kind = await getInstallKind();
      if (kind === "nsis" || kind === "auto") {
        setStatusText("upd-status", () => t("upd_found", { v: version, cur: u.current }));
        install.classList.remove("hidden");
      } else {
        setStatusText("upd-status", () => t("upd_manual", { v: version }));
      }
      open.classList.remove("hidden");
      onFound?.(version);
    } catch (e) {
      logErr("update-check: " + String(e).slice(0, 200));
      if (!quiet) setStatusText("upd-status", () => t("upd_fail"));
    }
  };

  check.addEventListener("click", () => void runCheck(false));

  install.addEventListener("click", async () => {
    if (!pendingVersion || install.disabled) return;
    try {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      const ok = await confirm(t("upd_confirm", { v: pendingVersion }), { title: "ProxPulse", kind: "info" });
      if (!ok) return;
      install.disabled = true;
      setStatusText("upd-status", () => t("upd_installing"));
      const stop = await listen<{ downloaded: number; total: number | null }>("update://progress", (e) => {
        const { downloaded, total } = e.payload;
        setStatusText("upd-status", () =>
          total
            ? t("upd_progress", { p: Math.min(100, Math.round((downloaded / total) * 100)) })
            : t("upd_progress_mb", { mb: (downloaded / 1048576).toFixed(1) }),
        );
      }).catch(() => null);
      try {
        // On success the app restarts (or the Windows installer takes over),
        // so this normally does not return.
        await invoke("install_app_update", { expectedVersion: pendingVersion });
        setStatusText("upd-status", () => t("upd_restart"));
      } finally {
        stop?.();
      }
    } catch (e) {
      logErr("update-install: " + String(e).slice(0, 200));
      setStatusText("upd-status", () => t("upd_fail"));
    } finally {
      install.disabled = false;
    }
  });

  open.addEventListener("click", async () => {
    if (!releaseUrl) return;
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(releaseUrl);
    } catch {
      window.open(releaseUrl, "_blank");
    }
  });

  return { checkQuietly: () => runCheck(true) };
}
