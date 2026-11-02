import { describe, expect, it } from "vitest";
import { judgeAddress, judgeUrls } from "./judge.ts";

const both = { https: true, http: true };
const httpsOnly = { https: true, http: false };
const httpOnly = { https: false, http: true };

describe("judgeUrls", () => {
  it("builds both requests from a bare host", () => {
    expect(judgeUrls("judge.example", true, both)).toEqual({
      main: "https://judge.example/judge",
      http: "http://judge.example/judge",
    });
    expect(judgeUrls("judge.example", true, httpsOnly)).toEqual({ main: "https://judge.example/judge", http: null });
    expect(judgeUrls("judge.example", true, httpOnly)).toEqual({ main: "http://judge.example/judge", http: null });
  });

  it("checks only liveness in fast mode", () => {
    expect(judgeUrls("judge.example", false, both)).toEqual({ main: "https://judge.example/generate_204", http: null });
    expect(judgeUrls("judge.example", false, httpOnly).main).toBe("http://judge.example/generate_204");
  });

  it("accepts a typed scheme, an endpoint, a custom path and a query", () => {
    expect(judgeUrls("http://judge.example/", true, both).main).toBe("https://judge.example/judge");
    expect(judgeUrls("judge.example/generate_204", true, both)).toEqual({
      main: "https://judge.example/generate_204",
      http: "http://judge.example/judge",
    });
    expect(judgeUrls("judge.example/svc", true, both)).toEqual({
      main: "https://judge.example/svc",
      http: "http://judge.example/svc",
    });
    expect(judgeUrls("judge.example:8080/?x=1", true, httpOnly).main).toBe("http://judge.example:8080/judge?x=1");
  });

  it("falls back to HTTPS when neither scheme is chosen and rejects an empty host", () => {
    expect(judgeUrls("judge.example", true, { https: false, http: false }).main).toBe("https://judge.example/judge");
    expect(judgeUrls("  ", true, both)).toEqual({ main: "", http: null });
    expect(judgeAddress(" HTTPS://Judge.example/ ")).toBe("Judge.example");
  });
});
