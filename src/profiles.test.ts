import { describe, expect, it } from "vitest";
import {
  BUILTIN_PROFILES,
  DEFAULT_PROTO_VALUES,
  PROFILE_DEFAULTS,
  sanitizeProfile,
} from "./profiles.ts";

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
      timeout: "8000",
      concurrency: "100",
      repeats: "3",
      precheck: false,
      judgeFull: false,
      customUrl: "judge",
    });
    expect(data).toMatchObject({
      timeout: "8000",
      concurrency: "100",
      repeats: "3",
      precheck: false,
      judgeFull: false,
      customUrl: "judge",
    });
  });

  it("builtin profiles are valid", () => {
    for (const profile of BUILTIN_PROFILES) {
      expect(sanitizeProfile(profile.data)).toEqual(profile.data);
    }
  });
});
