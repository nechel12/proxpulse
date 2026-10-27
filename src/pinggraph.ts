export type PingPoint = { t: number; avg: number | null; alive: number };

export const PING_HIST_MAX = 120;

export function loadPingHist(): PingPoint[] {
  try {
    const raw = localStorage.getItem("pp-pinghist");
    const arr = raw ? JSON.parse(raw) : null;
    if (Array.isArray(arr)) {
      return arr
        .filter((p) => p && typeof p.t === "number")
        .map((p) => ({
          t: p.t,
          avg: typeof p.avg === "number" ? p.avg : null,
          alive: Number(p.alive) || 0,
        }))
        .slice(-PING_HIST_MAX);
    }
  } catch { /* ignore */ }
  return [];
}

export function pushPingPointStats(
  hist: PingPoint[],
  avg: number | null,
  alive: number,
): PingPoint[] {
  hist.push({ t: Date.now(), avg, alive });
  if (hist.length > PING_HIST_MAX) hist.splice(0, hist.length - PING_HIST_MAX);
  try {
    localStorage.setItem("pp-pinghist", JSON.stringify(hist));
  } catch { /* ignore */ }
  return hist;
}

export function renderPingGraph(hist: PingPoint[], lang: "ru" | "en"): void {
  const svg = document.getElementById("ping-graph") as unknown as SVGSVGElement | null;
  if (!svg) return;
  const pts = hist.filter((p) => p.avg != null);
  if (pts.length < 2) {
    svg.innerHTML = "";
    return;
  }
  const vals = pts.map((p) => p.avg as number);
  const min = Math.min(...vals);
  const span = Math.max(1, Math.max(...vals) - min);
  const n = pts.length;
  const d = pts
    .map((p, i) => `${((i / (n - 1)) * 100).toFixed(1)},${(30 - (((p.avg as number) - min) / span) * 26).toFixed(1)}`)
    .join(" ");
  const last = pts[pts.length - 1];
  svg.innerHTML =
    `<title>${last.avg} ms · alive ${last.alive} · ${new Date(last.t).toLocaleTimeString(lang === "ru" ? "ru-RU" : "en-US")}</title>` +
    `<polyline points="${d}" />`;
}
