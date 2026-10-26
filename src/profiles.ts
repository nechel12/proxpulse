// Settings profiles: pure data + validation, covered by profiles.test.ts.
import { IPAPI_FREE_URL, IPAPI_OLD_HTTPS_URL } from "./ipapi.ts";

export type ProfileData = {
  testUrlSel: string;
  customUrl: string;
  timeout: string;
  concurrency: string;
  repeats: string;
  defaultProto: string;
  geo: boolean;
  anon: boolean;
  tamper: boolean;
  tls: boolean;
  precheck: boolean;
  precheckTimeout: string;
  judgeFull: boolean;
};

export const PROFILE_DEFAULTS: ProfileData = {
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

export const TEST_URL_VALUES = [
  "https://www.google.com/generate_204",
  "https://cp.cloudflare.com/generate_204",
  "https://api.ipify.org",
  "https://httpbin.org/ip",
  IPAPI_FREE_URL,
  "https://example.com",
  "judge",
];

export const DEFAULT_PROTO_VALUES = ["http", "socks5", "socks4", "http+socks5"];

export function sanitizeProfile(raw: unknown): ProfileData | null {
  if (!raw || typeof raw !== "object") return null;
  const o = raw as Partial<ProfileData>;
  const str = (v: unknown, fb: string, re?: RegExp) =>
    typeof v === "string" && (!re || re.test(v)) ? v : fb;
  const bool = (v: unknown, fb: boolean) => (typeof v === "boolean" ? v : fb);
  const rawSel = str(o.testUrlSel, PROFILE_DEFAULTS.testUrlSel);
  // Older profiles stored the free endpoint with HTTPS, which always failed.
  const urlSel = rawSel === IPAPI_OLD_HTTPS_URL ? IPAPI_FREE_URL : rawSel;
  const proto = str(o.defaultProto, PROFILE_DEFAULTS.defaultProto);
  return {
    testUrlSel: TEST_URL_VALUES.includes(urlSel) ? urlSel : PROFILE_DEFAULTS.testUrlSel,
    customUrl: str(o.customUrl, ""),
    timeout: str(o.timeout, PROFILE_DEFAULTS.timeout, /^\d+$/),
    concurrency: str(o.concurrency, PROFILE_DEFAULTS.concurrency, /^\d+$/),
    repeats: str(o.repeats, PROFILE_DEFAULTS.repeats, /^\d+$/),
    defaultProto: DEFAULT_PROTO_VALUES.includes(proto) ? proto : "http",
    geo: bool(o.geo, true),
    anon: bool(o.anon, true),
    tamper: bool(o.tamper, true),
    tls: bool(o.tls, true),
    precheck: bool(o.precheck, true),
    precheckTimeout: str(o.precheckTimeout, PROFILE_DEFAULTS.precheckTimeout, /^\d+$/),
    judgeFull: bool(o.judgeFull, true),
  };
}

export const BUILTIN_PROFILES: { id: string; nameKey: string; data: ProfileData }[] = [
  {
    id: "builtin:fast",
    nameKey: "prof_fast",
    data: {
      testUrlSel: "https://www.google.com/generate_204",
      customUrl: "",
      timeout: "5000",
      concurrency: "100",
      repeats: "1",
      defaultProto: "http",
      geo: false,
      anon: false,
      tamper: false,
      tls: false,
      precheck: true,
      precheckTimeout: "1500",
      judgeFull: false,
    },
  },
  {
    id: "builtin:deep",
    nameKey: "prof_deep",
    data: {
      testUrlSel: IPAPI_FREE_URL,
      customUrl: "",
      timeout: "8000",
      concurrency: "50",
      repeats: "3",
      defaultProto: "http",
      geo: true,
      anon: true,
      tamper: true,
      tls: true,
      precheck: true,
      precheckTimeout: "1500",
      judgeFull: false,
    },
  },
  {
    id: "builtin:judge",
    nameKey: "prof_judge",
    data: {
      testUrlSel: "judge",
      customUrl: "",
      timeout: "5000",
      concurrency: "50",
      repeats: "1",
      defaultProto: "http",
      geo: false,
      anon: false,
      tamper: false,
      tls: false,
      precheck: true,
      precheckTimeout: "1500",
      judgeFull: true,
    },
  },
];
