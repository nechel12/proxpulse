// Judge URL construction (pure, covered by judge.test.ts).

export const JUDGE_SUFFIX_RE = /\/(judge|generate_204)$/;

export function resolveJudgeUrl(
  custom: string,
  full: boolean,
  defaultBase: string,
): string {
  const raw = (custom.trim() || defaultBase).replace(/\/+$/, "");
  try {
    const u = new URL(raw);
    if (JUDGE_SUFFIX_RE.test(u.pathname)) return u.toString();
    // A custom path on a self-hosted judge is used as-is.
    if (u.pathname && u.pathname !== "/") return raw;
    u.pathname = full ? "/judge" : "/generate_204";
    return u.toString();
  } catch {
    // Invalid URL: let the backend report it.
    return raw + (full ? "/judge" : "/generate_204");
  }
}
