// ip-api endpoint helpers (pure, covered by ipapi.test.ts).

export const IPAPI_FREE_URL = "http://ip-api.com/json";
export const IPAPI_PRO_BASE = "https://pro.ip-api.com/json/";
export const IPAPI_OLD_HTTPS_URL = "https://ip-api.com/json";

export function ipapiProUrl(key: string): string {
  return `${IPAPI_PRO_BASE}?key=${encodeURIComponent(key.trim())}`;
}

/**
 * Resolve the effective test URL. A custom URL always wins. The ip-api
 * free endpoint is HTTP-only; when Pro is enabled with a key, the same
 * selection is upgraded to the HTTPS Pro endpoint.
 */
export function resolveTestUrl(
  selected: string,
  custom: string,
  pro: boolean,
  key: string,
): string {
  const c = custom.trim();
  if (c) return c;
  const sel = selected.trim();
  const k = key.trim();
  if (pro && k && (sel === IPAPI_FREE_URL || sel === IPAPI_OLD_HTTPS_URL)) {
    return ipapiProUrl(k);
  }
  return sel;
}
