import { describe, expect, it } from "vitest";
import { resolveJudgeUrl } from "./judge.ts";

const base = "https://judge.example";

describe("resolveJudgeUrl", () => {
  it("appends the mode suffix to a host", () => {
    expect(resolveJudgeUrl("", true, base)).toBe(`${base}/judge`);
    expect(resolveJudgeUrl("", false, base)).toBe(`${base}/generate_204`);
  });

  it("keeps an explicit endpoint", () => {
    expect(resolveJudgeUrl(`${base}/judge`, false, base)).toBe(`${base}/judge`);
    expect(resolveJudgeUrl(`${base}/generate_204/`, true, base)).toBe(
      `${base}/generate_204`,
    );
  });

  it("keeps a custom path as-is", () => {
    expect(resolveJudgeUrl(`${base}/svc`, true, base)).toBe(`${base}/svc`);
  });

  it("preserves a query on a host and fixes the old bad concatenation", () => {
    expect(resolveJudgeUrl(`${base}/?x=1`, true, base)).toBe(
      `${base}/judge?x=1`,
    );
    expect(resolveJudgeUrl(`${base}/judge?x=1`, true, base)).toBe(
      `${base}/judge?x=1`,
    );
  });

  it("falls back for an unparsable URL", () => {
    expect(resolveJudgeUrl("not a url", true, base)).toBe("not a url/judge");
  });
});
