import { invoke } from "@tauri-apps/api/core";
import { buildCsv, buildJson, buildTxt } from "./export.ts";
import type { ProxyResult } from "./types.ts";

export type ExportKind = "txt" | "json" | "csv" | "copy";
type Notify = (key: string, params?: Record<string, string | number>) => void;

type Deps = {
  isTauriRuntime: () => boolean;
  notify: Notify;
};

function browserDownload(name: string, text: string, mime = "text/plain;charset=utf-8") {
  const blob = new Blob([text], { type: mime });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

async function saveViaDialog(
  suggested: string,
  text: string,
  filters: { name: string; extensions: string[] }[],
  deps: Deps,
): Promise<void> {
  if (!deps.isTauriRuntime()) {
    browserDownload(suggested, text);
    deps.notify("st_downloaded", { name: suggested });
    return;
  }
  try {
    const path = await invoke<string | null>("export_text_file", {
      contents: text,
      defaultName: suggested,
      filterName: filters[0]?.name ?? "Text",
      extensions: filters[0]?.extensions ?? [],
    });
    if (!path) return;
    deps.notify("st_saved", { path });
  } catch (e) {
    deps.notify("st_savefail", { e: String(e).slice(0, 120) });
  }
}

export async function exportList(
  kind: ExportKind,
  list: ProxyResult[],
  deps: Deps,
): Promise<void> {
  if (kind === "copy") {
    const txt = buildTxt(list);
    if (!txt) {
      deps.notify("st_nocopy");
      return;
    }
    try {
      await navigator.clipboard.writeText(txt);
      deps.notify("st_copied", { n: txt.split("\n").length });
    } catch {
      deps.notify("st_nocopy");
    }
    return;
  }
  if (list.length === 0) {
    deps.notify("st_nothing");
    return;
  }
  if (kind === "txt") {
    await saveViaDialog("proxpulse-alive.txt", buildTxt(list), [{ name: "Text", extensions: ["txt"] }], deps);
  } else if (kind === "json") {
    await saveViaDialog("proxpulse.json", buildJson(list), [{ name: "JSON", extensions: ["json"] }], deps);
  } else {
    await saveViaDialog("proxpulse.csv", buildCsv(list), [{ name: "CSV", extensions: ["csv"] }], deps);
  }
}
