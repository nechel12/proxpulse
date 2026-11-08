import { invoke } from "@tauri-apps/api/core";

/** Edge-triggered alerts: a problem is reported once until it clears. */
export class AlertLatch {
  private active = new Set<string>();

  /** True when the alert was not active yet and should be shown. */
  has(key: string): boolean {
    return this.active.has(key);
  }

  raise(key: string): boolean {
    if (this.active.has(key)) return false;
    this.active.add(key);
    return true;
  }

  clear(key: string): void {
    this.active.delete(key);
  }
}

async function windowInFront(): Promise<boolean> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const w = getCurrentWindow();
    return (await w.isVisible()) && !(await w.isMinimized()) && (await w.isFocused());
  } catch {
    return false;
  }
}

type NotifierDeps = {
  enabled: () => boolean;
  isTauriRuntime: () => boolean;
  logErr: (msg: string) => void;
};

/** System notifications, shown only while the window is not in front. */
export class Notifier {
  readonly latch = new AlertLatch();

  constructor(private deps: NotifierDeps) {}

  /**
   * One notification per problem: the latch is set only once one went out,
   * so a problem first seen while the window was in front still notifies
   * when it is not. `latch.clear(key)` ends the problem.
   */
  async alert(key: string, title: string, body: string): Promise<void> {
    if (this.latch.has(key)) return;
    if (await this.send(title, body)) this.latch.raise(key);
  }

  async send(title: string, body: string): Promise<boolean> {
    if (!this.deps.enabled() || !this.deps.isTauriRuntime()) return false;
    if (await windowInFront()) return false;
    try {
      await invoke("notify", { title, body });
      return true;
    } catch (e) {
      this.deps.logErr("notify: " + String(e).slice(0, 200));
      return false;
    }
  }
}
