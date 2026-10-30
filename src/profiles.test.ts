import { describe, expect, it } from "vitest";
import { IPAPI_FREE_URL, IPAPI_OLD_HTTPS_URL } from "./ipapi.ts";
import {
  BUILTIN_PROFILES,
  DEFAULT_PROTO_VALUES,
  PROFILE_DEFAULTS,
  PUBLIC_CUSTOM,
  sanitizeProfile,
} from "./profiles.ts";

const LEGACY = {
  testUrlSel: "https://www.google.com/generate_204",
  customUrl: "",
  timeout: "5000",
  concurrency: "50",
  repeats: "1",
  defaultProto: "http",
  geo: true,
  anon: true,
  tamper: true,
  tls: true,
  precheck: true,
  precheckTimeout: "1500",
  judgeFull: true,
};

describe("sanitizeProfile", () => {
  it("round-trips every supported default protocol", () => {
    for (const proto of DEFAULT_PROTO_VALUES) {
      const data = sanitizeProfile({ ...PROFILE_DEFAULTS, defaultProto: proto });
      expect(data?.defaultProto).toBe(proto);
    }
  });

  it("falls back for unknown values", () => {
    const data = sanitizeProfile({
      testUrlSel: "ftp://example.com",
      defaultProto: "gopher",
      timeout: "abc",
      concurrency: "-1",
      repeats: "1.5",
      precheckTimeout: "x",
      geo: "yes",
    });
    expect(data).not.toBeNull();
    expect(data?.testUrlSel).toBe(PROFILE_DEFAULTS.testUrlSel);
    expect(data?.defaultProto).toBe("http");
    expect(data?.timeout).toBe(PROFILE_DEFAULTS.timeout);
    expect(data?.concurrency).toBe(PROFILE_DEFAULTS.concurrency);
    expect(data?.repeats).toBe(PROFILE_DEFAULTS.repeats);
    expect(data?.precheckTimeout).toBe(PROFILE_DEFAULTS.precheckTimeout);
    expect(data?.geo).toBe(true);
  });

  it("rejects non-objects", () => {
    expect(sanitizeProfile(null)).toBeNull();
    expect(sanitizeProfile("x")).toBeNull();
    expect(sanitizeProfile(42)).toBeNull();
  });

  it("keeps valid numeric strings and booleans", () => {
    const data = sanitizeProfile({
      ...PROFILE_DEFAULTS,
      testUrlSel: PUBLIC_CUSTOM,
      customUrl: "https://custom.example/path",
      timeout: "8000",
      concurrency: "100",
      repeats: "3",
      precheck: false,
      judgeFull: false,
    });
    expect(data).toMatchObject({
      target: "public",
      testUrlSel: PUBLIC_CUSTOM,
      customUrl: "https://custom.example/path",
      timeout: "8000",
      concurrency: "100",
      repeats: "3",
      precheck: false,
      judgeFull: false,
    });
  });

  it("does not turn a hidden custom URL into the active one", () => {
    const data = sanitizeProfile({
      ...PROFILE_DEFAULTS,
      customUrl: "https://left-over.example",
    });
    expect(data?.testUrlSel).toBe(PROFILE_DEFAULTS.testUrlSel);
  });

  it("builtin profiles are valid", () => {
    for (const profile of BUILTIN_PROFILES) {
      expect(sanitizeProfile(profile.data)).toEqual(profile.data);
    }
  });

  it("keeps the standard builtin in sync with the defaults", () => {
    const standard = BUILTIN_PROFILES.find((p) => p.id === "builtin:standard");
    expect(standard?.data).toEqual(PROFILE_DEFAULTS);
  });

  it("migrates the obsolete ip-api HTTPS URL", () => {
    const data = sanitizeProfile({
      ...PROFILE_DEFAULTS,
      testUrlSel: IPAPI_OLD_HTTPS_URL,
    });
    expect(data?.testUrlSel).toBe(IPAPI_FREE_URL);
  });

  it("migrates a legacy judge preset with a custom server", () => {
    const data = sanitizeProfile({
      ...LEGACY,
      testUrlSel: "judge",
      customUrl: "https://judge.example",
    });
    expect(data).toMatchObject({
      target: "judge",
      judgeServer: "custom",
      judgeUrl: "https://judge.example",
      customUrl: "",
      testUrlSel: PROFILE_DEFAULTS.testUrlSel,
    });
  });

  it("migrates a legacy judge preset without a custom server", () => {
    const data = sanitizeProfile({ ...LEGACY, testUrlSel: "judge" });
    expect(data).toMatchObject({
      target: "judge",
      judgeServer: "builtin",
      judgeUrl: "",
    });
  });

  it("migrates a legacy custom public URL", () => {
    const data = sanitizeProfile({
      ...LEGACY,
      testUrlSel: "https://example.com",
      customUrl: "https://my.example/list",
    });
    expect(data).toMatchObject({
      target: "public",
      testUrlSel: PUBLIC_CUSTOM,
      customUrl: "https://my.example/list",
    });
  });
});
