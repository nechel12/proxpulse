import { screenVisible } from "./ui.ts";

export type Shortcut = "start" | "stop" | "close-menus" | "filter" | null;

/** What the page looks like when a key is pressed. */
export type ShortcutContext = {
  /** The Check tab is shown. */
  onCheck: boolean;
  /** The text dialog is open: it handles its own keys. */
  modalOpen: boolean;
  /** A dropdown or row menu is open. */
  menuOpen: boolean;
  /** The key was pressed in a text field or a select. */
  inField: boolean;
  /** A check runs that Stop would end. */
  running: boolean;
};

/**
 * Ctrl+Enter starts, Esc stops, Ctrl+F goes to the filter. Esc closes an
 * open menu first, and typing in a field never stops a run, which cannot be
 * resumed and whose results Start would replace.
 */
export function shortcutFor(key: { key: string; code: string; mod: boolean }, c: ShortcutContext): Shortcut {
  if (c.modalOpen) return null;
  if (key.mod && key.key === "Enter") return c.onCheck ? "start" : null;
  if (key.key === "Escape") {
    if (c.menuOpen) return "close-menus";
    return c.onCheck && c.running && !c.inField ? "stop" : null;
  }
  if (key.mod && key.code === "KeyF") return "filter";
  return null;
}

export function initShortcuts(deps: { running: () => boolean }): void {
  const click = (id: string) => {
    const b = document.getElementById(id) as HTMLButtonElement | null;
    if (b && !b.disabled) b.click();
  };
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented) return;
    const onCheck = screenVisible("screen-check");
    const menus = document.querySelectorAll<HTMLElement>(".menu:not(.hidden)");
    const target = e.target as HTMLElement | null;
    const action = shortcutFor(
      { key: e.key, code: e.code, mod: e.ctrlKey || e.metaKey },
      {
        onCheck,
        modalOpen: !document.getElementById("text-modal")?.classList.contains("hidden"),
        menuOpen: menus.length > 0,
        inField: !!target?.closest("input, textarea, select, [contenteditable='true']"),
        running: deps.running(),
      },
    );
    if (action === "start") {
      e.preventDefault();
      click("btn-start");
    } else if (action === "stop") {
      e.preventDefault();
      click("btn-stop");
    } else if (action === "close-menus") {
      e.preventDefault();
      menus.forEach((m) => m.classList.add("hidden"));
    } else if (action === "filter") {
      const field = onCheck
        ? (document.getElementById("search") as HTMLInputElement | null)
        : document.querySelector<HTMLInputElement>(".screen:not(.hidden) .pool-filter");
      if (!field) return;
      e.preventDefault();
      field.focus();
      field.select();
    }
  });
}
