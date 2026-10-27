import { clip } from "./clip.ts";

export type ErrItem = { t: string; msg: string };

const STORAGE_KEY = "pp-errors";
const MAX_ERRORS = 50;

export class ErrorLog {
  private items: ErrItem[] = [];

  load(): void {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const arr = JSON.parse(raw) as ErrItem[];
      if (Array.isArray(arr)) {
        this.items = arr.filter((e) => e && typeof e.msg === "string").slice(-MAX_ERRORS);
      }
    } catch { /* ignore */ }
  }

  log(msg: string): void {
    this.items.push({ t: new Date().toISOString(), msg: clip(msg, 300) });
    if (this.items.length > MAX_ERRORS) this.items.shift();
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.items.slice(-MAX_ERRORS)));
    } catch { /* ignore */ }
    this.render();
  }

  recent(n: number): ErrItem[] {
    return this.items.slice(-n);
  }

  render(): void {
    if (typeof document === "undefined") return;
    const el = document.getElementById("err-list");
    if (!el) return;
    el.textContent = this.items.length === 0
      ? "-"
      : this.items.slice(-10).reverse().map((e) => `${e.t} ${e.msg}`).join("\n");
  }
}

export const errorLog = new ErrorLog();
export const logErr = (msg: string): void => errorLog.log(msg);
