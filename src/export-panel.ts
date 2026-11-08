import { invoke } from "@tauri-apps/api/core";
import { buildCsv, buildJson, buildTxt } from "./export.ts";
import { buildClash, buildPac, buildProxifier, buildSingBox, buildTemplate } from "./export-configs.ts";
import type { ProxyResult } from "./types.ts";

export type ExportKind = "txt" | "json" | "csv" | "copy" | "clash" | "singbox" | "proxifier" | "pac" | "template";
type Notify = (key: string, params?: Record<string, string | number>) => void;

type Deps = {
  isTauriRuntime: () => boolean;
  notify: Notify;
  /** Dead proxies are part of the list: the file must not claim "alive". */
  includeDead: boolean;
  /** Asks for the line template; null cancels. */
  askTemplate?: () => Promise<string | null>;
  /** Set when the table's filters hide part of the results. */
  filtered?: { shown: number; total: number };
};

/** The status key, with "N of M" when filters left part of the list out. */
function done(deps: Deps, key: string, params: Record<string, string | number>): void {
  if (deps.filtered) deps.notify(`${key}_part`, { ...params, shown: deps.filtered.shown, total: deps.filtered.total });
  else deps.notify(key, params);
}

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
    done(deps, "st_downloaded", { name: suggested });
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
    done(deps, "st_saved", { path });
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
      done(deps, "st_copied", { n: txt.split("\n").length });
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
    const name = deps.includeDead ? "proxpulse-all.txt" : "proxpulse-alive.txt";
    await saveViaDialog(name, buildTxt(list), [{ name: "Text", extensions: ["txt"] }], deps);
  } else if (kind === "json") {
    await saveViaDialog("proxpulse.json", buildJson(list), [{ name: "JSON", extensions: ["json"] }], deps);
  } else if (kind === "clash") {
    await saveViaDialog("proxpulse-clash.yaml", buildClash(list), [{ name: "YAML", extensions: ["yaml", "yml"] }], deps);
  } else if (kind === "singbox") {
    await saveViaDialog("proxpulse-sing-box.json", buildSingBox(list), [{ name: "JSON", extensions: ["json"] }], deps);
  } else if (kind === "proxifier") {
    await saveViaDialog("proxpulse.ppx", buildProxifier(list), [{ name: "Proxifier", extensions: ["ppx"] }], deps);
  } else if (kind === "pac") {
    await saveViaDialog("proxpulse.pac", buildPac(list), [{ name: "PAC", extensions: ["pac"] }], deps);
  } else if (kind === "template") {
    const template = await deps.askTemplate?.();
    if (!template) return;
    await saveViaDialog("proxpulse.txt", buildTemplate(list, template), [{ name: "Text", extensions: ["txt"] }], deps);
  } else {
    await saveViaDialog("proxpulse.csv", buildCsv(list), [{ name: "CSV", extensions: ["csv"] }], deps);
  }
}
