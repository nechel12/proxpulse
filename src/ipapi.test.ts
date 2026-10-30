import { describe, expect, it } from "vitest";
import {
  IPAPI_FREE_URL,
  IPAPI_OLD_HTTPS_URL,
  ipapiProUrl,
  maskKey,
  resolveTestUrl,
} from "./ipapi.ts";

describe("ipapi", () => {
  it("builds the Pro URL with an encoded key", () => {
    expect(ipapiProUrl("abc 123")).toBe(
      "https://pro.ip-api.com/json/?key=abc%20123",
    );
  });

  it("masks the key in the effective URL shown by the UI", () => {
    expect(maskKey("https://pro.ip-api.com/json/?key=abc&x=1")).toBe(
      "https://pro.ip-api.com/json/?key=***&x=1",
    );
    expect(maskKey("https://example.com/path")).toBe("https://example.com/path");
  });

  it("keeps the free ip-api URL without a Pro key", () => {
    expect(resolveTestUrl(IPAPI_FREE_URL, "", true, "  ")).toBe(IPAPI_FREE_URL);
    expect(resolveTestUrl(IPAPI_FREE_URL, "", false, "key")).toBe(
      IPAPI_FREE_URL,
    );
  });

  it("upgrades ip-api to Pro when enabled", () => {
    expect(resolveTestUrl(IPAPI_FREE_URL, "", true, "my-key")).toBe(
      "https://pro.ip-api.com/json/?key=my-key",
    );
    expect(resolveTestUrl(IPAPI_OLD_HTTPS_URL, "", true, "my-key")).toBe(
      "https://pro.ip-api.com/json/?key=my-key",
    );
  });

  it("does not touch non-ip-api or custom URLs", () => {
    expect(
      resolveTestUrl("https://example.com", "", true, "my-key"),
    ).toBe("https://example.com");
    expect(
      resolveTestUrl(IPAPI_FREE_URL, "https://example.org", true, "my-key"),
    ).toBe("https://example.org");
  });
});
