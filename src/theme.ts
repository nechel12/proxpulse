// Color theme: a stored preference applied as `data-theme` on <html>.

export type ThemePref = "dark" | "light" | "system";
export type Theme = "dark" | "light";

const STORAGE_KEY = "pp-theme";
const LIGHT_QUERY = "(prefers-color-scheme: light)";

export function parseThemePref(v: unknown): ThemePref {
  return v === "light" || v === "system" ? v : "dark";
}

export function resolveTheme(pref: ThemePref, systemLight: boolean): Theme {
  if (pref === "system") return systemLight ? "light" : "dark";
  return pref;
}

export function loadThemePref(): ThemePref {
  try {
    return parseThemePref(localStorage.getItem(STORAGE_KEY));
  } catch {
    return "dark";
  }
}

export function saveThemePref(pref: ThemePref): void {
  try {
    localStorage.setItem(STORAGE_KEY, pref);
  } catch { /* ignore */ }
}

function systemLight(): boolean {
  return typeof matchMedia === "function" && matchMedia(LIGHT_QUERY).matches;
}

/** Apply the page colors and, in the desktop app, the native title bar. */
export function applyTheme(pref: ThemePref): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = resolveTheme(pref, systemLight());
  if (!("__TAURI_INTERNALS__" in window)) return;
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(pref === "system" ? null : pref))
    .catch(() => { /* older runtime */ });
}

/** Follow OS theme changes while the preference is "system". */
export function watchSystemTheme(current: () => ThemePref): void {
  if (typeof matchMedia !== "function") return;
  matchMedia(LIGHT_QUERY).addEventListener("change", () => {
    if (current() === "system") applyTheme("system");
  });
}
