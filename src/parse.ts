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

function hostForUrl(host: string): string {
  const h = host.trim();
  return h.includes(":") && !h.startsWith("[") ? `[${h}]` : h;
}

function looksLikeHost(s: string): boolean {
  const t = s.trim().replace(/^\[|\]$/g, "");
  if (!t) return false;
  if (/^(\d{1,3}\.){3}\d{1,3}$/.test(t)) return true;
  if (t.includes(":") || t.includes(".")) return true;
  return t.toLowerCase() === "localhost";
}

function isPortValue(s: string): boolean {
  return /^\d{1,5}$/.test(s) && Number(s) >= 1 && Number(s) <= 65535;
}

function isProtoValue(s: string): boolean {
  return /^(https?|socks5h?|socks4a?|socks)$/i.test(s.trim());
}

/** Port at the end of the authority (after the last @), not a numeric password. */
function hasValidPort(s: string): boolean {
  const afterScheme = s.includes("://") ? s.split("://", 2)[1] : s;
  const authority = afterScheme
    .slice(afterScheme.lastIndexOf("@") + 1)
    .split(/[/?#]/)[0];
  // Bare IPv6 is not a supported proxy form; require brackets for IPv6.
  if (!authority.includes("]") && (authority.match(/:/g)?.length ?? 0) > 1) {
    return false;
  }
  const m = authority.match(/:(\d{1,5})$/);
  if (!m) return false;
  const n = Number(m[1]);
  return n >= 1 && n <= 65535;
}

function isColonColumns(s: string): boolean {
  const parts = s.split(":");
  if (parts.length === 3) {
    return (
      (isPortValue(parts[1]) && isProtoValue(parts[2])) ||
      (isProtoValue(parts[0]) && isPortValue(parts[2]))
    );
  }
  if (parts.length === 4) {
    const hostFirst = isPortValue(parts[1]) && !!parts[0];
    const hostLast = isPortValue(parts[3]) && !!parts[2];
    return hostFirst || hostLast;
  }
  return false;
}

function looksLikeProxyValue(s: string): boolean {
  if (!s || s.startsWith("://")) return false;
  if (/\s/.test(s)) return false;
  return hasValidPort(s) || isColonColumns(s);
}

export function buildFromParts(host: string, port: string, user?: string | null, pass?: string | null, proto?: string | null): string | null {
  host = hostForUrl(host.trim().replace(/\/+$/, ""));
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

export function proxiesFromJson(
  v: unknown,
  out: string[],
  limit = Number.POSITIVE_INFINITY,
): void {
  if (v == null || out.length >= limit) return;
  if (typeof v === "string") {
    const s = v.trim();
    if (s.length > 2 && out.length < limit) out.push(s);
    return;
  }
  if (Array.isArray(v)) {
    for (const el of v) {
      if (out.length >= limit) break;
      proxiesFromJson(el, out, limit);
    }
    return;
  }
  if (typeof v === "object") {
    const o = v as Record<string, unknown>;
    const host = getKey(o, HOST_KEYS);
    const port = getKey(o, PORT_KEYS);
    if (host && port && out.length < limit) {
      const built = buildFromParts(host, port, getKey(o, USER_KEYS), getKey(o, PASS_KEYS), getKey(o, PROTO_KEYS));
      if (built) out.push(built);
    }
    for (const k of ["proxies", "proxy", "data", "list", "items", "results", "rows"]) {
      if (Array.isArray(o[k])) {
        for (const el of o[k] as unknown[]) {
          if (out.length >= limit) break;
          proxiesFromJson(el, out, limit);
        }
        return;
      }
    }
    if (!host) {
      for (const val of Object.values(o)) {
        if (out.length >= limit) break;
        if (typeof val === "string" || Array.isArray(val) || (val && typeof val === "object")) {
          proxiesFromJson(val, out, limit);
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
  const isPort = isPortValue;
  const isProto = isProtoValue;
  if (parts.length === 2 && isPort(parts[1])) {
    return `${hostForUrl(parts[0])}:${parts[1]}`;
  }
  if (parts.length === 3) {
    if (isPort(parts[1]) && isProto(parts[2])) {
      return `${parts[2]}://${hostForUrl(parts[0])}:${parts[1]}`;
    }
    if (isProto(parts[0]) && isPort(parts[2])) {
      return `${parts[0]}://${hostForUrl(parts[1])}:${parts[2]}`;
    }
    return null;
  }
  if (parts.length === 4) {
    const hostFirst = isPort(parts[1]) && !!parts[0];
    const hostLast = isPort(parts[3]) && !!parts[2];
    if (!hostFirst && !hostLast) return null;
    const first = hostFirst && !hostLast
      ? true
      : !hostFirst && hostLast
        ? false
        : !(looksLikeHost(parts[2]) && !looksLikeHost(parts[0]));
    return first
      ? `${parts[2]}:${parts[3]}@${hostForUrl(parts[0])}:${parts[1]}`
      : `${parts[0]}:${parts[1]}@${hostForUrl(parts[2])}:${parts[3]}`;
  }
  if (parts.length === 5 && isProto(parts[0]) && isPort(parts[2])) {
    return `${parts[0]}://${parts[3]}:${parts[4]}@${hostForUrl(parts[1])}:${parts[2]}`;
  }
  return null;
}

export function stripProxyCredentials(raw: string): string {
  const at = raw.lastIndexOf("@");
  if (at < 0) return raw;
  const host = raw.slice(at + 1);
  if (!host) return raw;
  const schemeSep = raw.indexOf("://");
  if (schemeSep >= 0 && schemeSep < at) {
    return raw.slice(0, schemeSep + 3) + host;
  }
  return host;
}

export const SCHEME_URL_RE = /\b(?:socks5h?|socks4a?|socks|https?):\/\/[^\s"'<>,;()[\]]+/gi;

export function cleanTail(s: string): string {
  return s.replace(/[.,;!?)\]}>]+$/, "").replace(/^[(<{]+/, "").trim();
}

export function extractProxiesFromText(
  text: string,
  limit = Number.POSITIVE_INFINITY,
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const push = (s: string) => {
    if (out.length >= limit) return;
    s = cleanTail(s.trim().replace(/^["'`]+|["'`]+$/g, ""));
    if (s.length < 3 || s.length > 512 || s.startsWith("#") || s.startsWith("//")) return;
    // Soft validation: proxies need a host:port-shaped authority, and a
    // bare string from a JSON log or prose must not become a "proxy".
    if (!looksLikeProxyValue(s)) return;
    if (seen.has(s)) return;
    seen.add(s);
    out.push(s);
  };

  try {
    const j = JSON.parse(text);
    const tmp: string[] = [];
    proxiesFromJson(j, tmp, limit);
    if (tmp.length > 0) {
      for (const p of tmp) {
        push(p);
        if (out.length >= limit) break;
      }
      return out;
    }
  } catch { /* not json */ }

  const lines = text.split(/\r?\n/);
  for (const raw of lines) {
    if (out.length >= limit) break;
    let line = raw.trim();
    if (!line || line.startsWith("#") || line.startsWith("//")) continue;
    line = line.replace(/^(\d{1,6}[).\-:]\s+|[-*•>]\s+|\[\s*[xX✓]?\s*\]\s*)/, "").trim();
    if (!line) continue;

    const csv = tryCsvLine(line);
    if (csv) { push(csv); continue; }

    const urls = line.match(SCHEME_URL_RE);
    if (urls && urls.length > 0) {
      for (const u of urls) {
        push(u);
        if (out.length >= limit) break;
      }
      continue;
    }
    const hashIdx = line.indexOf(" #");
    if (hashIdx > 0) line = line.slice(0, hashIdx).trim();
    const token = line.split(/[\s,;|]+/)[0];
    if (token && token !== line && /(?::\d{1,5}|@|:\/\/)/.test(token)) line = token;
    push(line);
  }

  if (out.length < 5 && out.length < limit) {
    const found = text.match(/\b(?:\d{1,3}\.){3}\d{1,3}:\d{2,5}\b/g);
    if (found) {
      for (const f of found) {
        if (seen.has(f)) continue;
        if (out.some((s) => s.includes(f))) continue;
        push(f);
        if (out.length >= limit) break;
      }
    }
  }
  return out;
}

export function parseInput(text: string): string[] {
  return extractProxiesFromText(text);
}
