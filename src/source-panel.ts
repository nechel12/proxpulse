import type { ParseResult } from "./parse-runner.ts";

type ParseFn = (text: string, limit: number, wantList: boolean) => Promise<ParseResult>;
type SetStatus = (key: string, params?: Record<string, string | number>) => void;

type Deps = {
  parseTextAsync: ParseFn;
  fetchUrl: (url: string) => Promise<string>;
  setStatus: SetStatus;
  maxProxies: number;
  maxInputChars: number;
  maxFileBytes: number;
  maxFileMb: number;
};

export function initSourcePanel(deps: Deps): void {
  const input = document.getElementById("proxy-input") as HTMLTextAreaElement | null;
  if (!input) return;
  let updGen = 0;
  let updT: number | undefined;

  const update = () => {
    const text = input.value;
    const countEl = document.getElementById("src-count");
    if (!countEl) return;
    if (text.length > deps.maxInputChars) {
      countEl.textContent = "!";
      return;
    }
    const gen = ++updGen;
    void deps.parseTextAsync(text, deps.maxProxies + 1, false)
      .then((r) => {
        if (gen !== updGen) return;
        countEl.textContent = String(Math.min(r.count, deps.maxProxies)) + (r.count > deps.maxProxies ? "+" : "");
      })
      .catch(() => {
        if (gen === updGen) countEl.textContent = "!";
      });
  };

  input.addEventListener("input", () => {
    window.clearTimeout(updT);
    updT = window.setTimeout(update, 250);
  });
  update();

  const append = (found: string[]) => {
    input.value = (input.value ? input.value.replace(/\s+$/, "") + "\n" : "") + found.join("\n");
    update();
  };

  document.getElementById("btn-clear-in")?.addEventListener("click", () => {
    input.value = "";
    update();
  });

  document.getElementById("btn-import-url")?.addEventListener("click", async () => {
    const field = document.getElementById("import-url") as HTMLInputElement | null;
    const u = (field?.value || "").trim();
    if (!u) return;
    try {
      const txt = await deps.fetchUrl(u);
      const found = (await deps.parseTextAsync(txt, deps.maxProxies, true)).list ?? [];
      if (found.length === 0) {
        deps.setStatus("file_none", { name: u.slice(0, 60) });
        return;
      }
      append(found);
      deps.setStatus("url_loaded", { n: found.length });
    } catch {
      deps.setStatus("url_fail");
    }
  });

  document.getElementById("file-input")?.addEventListener("change", async (e) => {
    const files = (e.target as HTMLInputElement).files;
    if (!files || files.length === 0) return;
    const parts: string[] = [];
    for (const f of Array.from(files)) {
      if (f.size > deps.maxFileBytes) {
        deps.setStatus("file_too_big", { name: f.name, mb: deps.maxFileMb });
        continue;
      }
      try {
        const txt = await f.text();
        const found = (await deps.parseTextAsync(txt, deps.maxProxies, true)).list ?? [];
        if (found.length > 0) parts.push(...found);
        else deps.setStatus("file_none", { name: f.name });
      } catch {
        deps.setStatus("file_fail", { name: f.name });
      }
    }
    if (parts.length > 0) {
      append(parts);
      deps.setStatus("file_loaded", { n: parts.length, f: files.length });
    }
    (e.target as HTMLInputElement).value = "";
  });
}
