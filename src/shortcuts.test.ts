import { describe, expect, it } from "vitest";
import { shortcutFor, type ShortcutContext } from "./shortcuts.ts";

const page: ShortcutContext = { onCheck: true, modalOpen: false, menuOpen: false, inField: false, running: true };
const esc = { key: "Escape", code: "Escape", mod: false };
const ctrlEnter = { key: "Enter", code: "Enter", mod: true };
const ctrlF = { key: "f", code: "KeyF", mod: true };

describe("shortcutFor", () => {
  it("starts with Ctrl+Enter on the Check tab only", () => {
    expect(shortcutFor(ctrlEnter, page)).toBe("start");
    expect(shortcutFor(ctrlEnter, { ...page, onCheck: false })).toBeNull();
    expect(shortcutFor({ ...ctrlEnter, mod: false }, page)).toBeNull();
  });

  it("stops a running check with Esc, but not from a field or another tab", () => {
    expect(shortcutFor(esc, page)).toBe("stop");
    expect(shortcutFor(esc, { ...page, inField: true })).toBeNull();
    expect(shortcutFor(esc, { ...page, onCheck: false })).toBeNull();
    expect(shortcutFor(esc, { ...page, running: false })).toBeNull();
  });

  it("closes an open menu before anything else", () => {
    expect(shortcutFor(esc, { ...page, menuOpen: true })).toBe("close-menus");
    expect(shortcutFor(esc, { ...page, menuOpen: true, inField: true, running: false })).toBe("close-menus");
  });

  it("goes to the filter with Ctrl+F on any layout", () => {
    // The key code, not the letter: it works with a Russian layout too.
    expect(shortcutFor({ key: "а", code: "KeyF", mod: true }, { ...page, onCheck: false })).toBe("filter");
    expect(shortcutFor(ctrlF, page)).toBe("filter");
  });

  it("leaves every key to the open dialog", () => {
    for (const key of [esc, ctrlEnter, ctrlF]) expect(shortcutFor(key, { ...page, modalOpen: true })).toBeNull();
  });
});
