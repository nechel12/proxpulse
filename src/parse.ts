// ---------- proxy extraction (many formats) ----------
// Pure functions (no DOM): covered by src/parse.test.ts.

export const PROTO_KEYS = ["protocol", "proto", "type", "scheme"];
export const HOST_KEYS = ["host", "hostname", "ip", "server", "address", "addr"];
export const PORT_KEYS = ["port", "port_number", "portnumber"];
export const USER_KEYS = ["user", "username", "login", "usr", "name"];
export const PASS_KEYS = ["pass", "password", "pwd", "passw", "passwd", "secret"];

export function getKey(obj: Record<string, unknown>, keys: string[]): string | null {
  const lower: Record<string, unknown> = {};
  for (const k of Object.keys(obj)) lower[k.toLowerCase()] = obj[k];
  for (const k of keys) {
    const v = lower[k];
    if (v != null && String(v).trim() !== "") return String(v).trim();
  }
  return null;
}

export function buildFromParts(host: string, port: string, user?: string | null, pass?: string | null, proto?: string | null): string | null {
  host = host.trim().replace(/\/+$/, "");
  port = port.trim();
  if (!host || !/^\d{1,5}$/.test(port)) return null;
  const p = Number(port);
  if (p < 1 || p > 65535) return null;
  let prefix = "";
  if (proto && proto.trim()) {
    const s = proto.trim().toLowerCase();
    prefix = s.includes("://") ? s : `${s}://`;
  }
  if (user && pass) return `${prefix}${user}:${pass}@${host}:${port}`;
  if (user) return `${prefix}${user}@${host}:${port}`;
  return `${prefix}${host}:${port}`;
}

export function proxiesFromJson(v: unknown, out: string[]): void {
  if (v == null) return;
  if (typeof v === "string") {
    const s = v.trim();
    if (s.length > 2) out.push(s);
    return;
  }
  if (Array.isArray(v)) {
    for (const el of v) proxiesFromJson(el, out);
    return;
  }
  if (typeof v === "object") {
    const o = v as Record<string, unknown>;
    const host = getKey(o, HOST_KEYS);
    const port = getKey(o, PORT_KEYS);
    if (host && port) {
      const built = buildFromParts(host, port, getKey(o, USER_KEYS), getKey(o, PASS_KEYS), getKey(o, PROTO_KEYS));
      if (built) out.push(built);
    }
    for (const k of ["proxies", "proxy", "data", "list", "items", "results", "rows"]) {
      if (Array.isArray(o[k])) {
        for (const el of o[k] as unknown[]) proxiesFromJson(el, out);
        return;
      }
    }
    if (!host) {
      for (const val of Object.values(o)) {
        if (typeof val === "string" || Array.isArray(val) || (val && typeof val === "object")) {
          proxiesFromJson(val, out);
        }
      }
    }
  }
}

export function tryCsvLine(line: string): string | null {
  let parts: string[] | null = null;
  if (/[,;|\t]/.test(line)) {
    parts = line.split(/[,;|\t]/).map((s) => s.trim().replace(/^["'`]+|["'`]+$/g, "")).filter(Boolean);
  } else if (!line.includes("://") && !line.includes("@")) {
    const ws = line.split(/\s+/).filter(Boolean);
    if (ws.length >= 2 && ws.length <= 5) parts = ws;
  }
  if (!parts || parts.length < 2 || parts.length > 5) return null;
  const isPort = (s: string) => /^\d{2,5}$/.test(s) && Number(s) >= 1 && Number(s) <= 65535;
  const isProto = (s: string) => /^(https?|socks5h?|socks4a?|socks)$/i.test(s.trim());
  if (parts.length === 2 && isPort(parts[1])) return `${parts[0]}:${parts[1]}`;
  if (parts.length === 3) {
    if (isPort(parts[1]) && isProto(parts[2])) return `${parts[2]}://${parts[0]}:${parts[1]}`;
    if (isProto(parts[0]) && isPort(parts[2])) return `${parts[0]}://${parts[1]}:${parts[2]}`;
    return null;
  }
  if (parts.length === 4) {
    if (isPort(parts[1])) return `${parts[2]}:${parts[3]}@${parts[0]}:${parts[1]}`;
    if (isPort(parts[3])) return `${parts[0]}:${parts[1]}@${parts[2]}:${parts[3]}`;
    return null;
  }
  if (parts.length === 5 && isProto(parts[0]) && isPort(parts[2])) {
    return `${parts[0]}://${parts[3]}:${parts[4]}@${parts[1]}:${parts[2]}`;
  }
  return null;
}

export const SCHEME_URL_RE = /\b(?:socks5h?|socks4a?|socks|https?)\:\/\/[^\s"'<>,;()\[\]]+/gi;

export function cleanTail(s: string): string {
  return s.replace(/[.,;!?)\]}>]+$/, "").replace(/^[(<{]+/, "").trim();
}

export function extractProxiesFromText(text: string): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const push = (s: string) => {
    s = cleanTail(s.trim().replace(/^["'`]+|["'`]+$/g, ""));
    if (s.length < 3 || s.length > 512 || s.startsWith("#") || s.startsWith("//")) return;
    if (seen.has(s)) return;
    seen.add(s);
    out.push(s);
  };

  try {
    const j = JSON.parse(text);
    const tmp: string[] = [];
    proxiesFromJson(j, tmp);
    if (tmp.length > 0) {
      tmp.forEach(push);
      return out;
    }
  } catch { /* not json */ }

  const lines = text.split(/\r?\n/);
  for (let raw of lines) {
    let line = raw.trim();
    if (!line || line.startsWith("#") || line.startsWith("//")) continue;
    line = line.replace(/^(\d{1,6}[).\-:]\s+|[-*•>]\s+|\[\s*[xX✓]?\s*\]\s*)/, "").trim();
    if (!line) continue;

    const csv = tryCsvLine(line);
    if (csv) { push(csv); continue; }

    const urls = line.match(SCHEME_URL_RE);
    if (urls && urls.length > 0) {
      urls.forEach(push);
      continue;
    }
    const hashIdx = line.indexOf(" #");
    if (hashIdx > 0) line = line.slice(0, hashIdx).trim();
    push(line);
  }

  if (out.length < 5) {
    const found = text.match(/\b(?:\d{1,3}\.){3}\d{1,3}:\d{2,5}\b/g);
    if (found) {
      for (const f of found) {
        if (seen.has(f)) continue;
        if (out.some((s) => s.includes(f))) continue;
        push(f);
      }
    }
  }
  return out;
}

export function parseInput(text: string): string[] {
  return extractProxiesFromText(text);
}
