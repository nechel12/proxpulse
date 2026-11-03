// Judge URL construction (pure, covered by judge.test.ts). The user enters
// a host, optionally with a path; the schemes come from the HTTPS and HTTP
// choices.

export type JudgeSchemes = { https: boolean; http: boolean };

/** Host and path as typed, without a scheme or trailing slashes. */
export function judgeAddress(input: string): string {
  return input.trim().replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/\/+(?=$|[?#])/, "");
}

/**
 * The main judge request, over HTTPS when chosen, and with both schemes in
 * full mode the plain-HTTP `/judge` request for anonymity and integrity.
 * A bare host gets `/judge` or `/generate_204`; an explicit endpoint or a
 * custom path on a self-hosted judge is used as-is.
 */
export function judgeUrls(input: string, full: boolean, schemes: JudgeSchemes): { main: string; http: string | null } {
  const address = judgeAddress(input);
  if (!address) return { main: "", http: null };
  const cut = address.search(/[?#]/);
  const hostPath = cut < 0 ? address : address.slice(0, cut);
  const query = cut < 0 ? "" : address.slice(cut);
  const slash = hostPath.indexOf("/");
  const host = slash < 0 ? hostPath : hostPath.slice(0, slash);
  const path = slash < 0 ? "" : hostPath.slice(slash).replace(/\/+$/, "");
  const endpoint = path || (full ? "/judge" : "/generate_204");
  const https = schemes.https || !schemes.http;
  const main = `${https ? "https" : "http"}://${host}${endpoint}${query}`;
  if (!full || !https || !schemes.http) return { main, http: null };
  // Anonymity needs the full answer even when /generate_204 was typed.
  const httpEndpoint = endpoint.replace(/\/generate_204$/, "/judge");
  return { main, http: `http://${host}${httpEndpoint}${query}` };
}
