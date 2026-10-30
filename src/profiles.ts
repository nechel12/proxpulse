// Settings presets: pure data + validation, covered by profiles.test.ts.
import { IPAPI_FREE_URL, IPAPI_OLD_HTTPS_URL } from "./ipapi.ts";

export type TargetMode = "public" | "judge";
export type JudgeServer = "builtin" | "custom";

export type ProfileData = {
  target: TargetMode;
  testUrlSel: string;
  customUrl: string;
  judgeServer: JudgeServer;
  judgeUrl: string;
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

export const PUBLIC_CUSTOM = "custom";

export const PUBLIC_URL_PRESETS = [
  "https://www.google.com/generate_204",
  "https://cp.cloudflare.com/generate_204",
  "https://api.ipify.org",
  "https://httpbin.org/ip",
  IPAPI_FREE_URL,
  "https://example.com",
];

export const DEFAULT_PROTO_VALUES = ["http", "socks5", "socks4", "http+socks5"];

export const PROFILE_DEFAULTS: ProfileData = {
  target: "public",
  testUrlSel: PUBLIC_URL_PRESETS[0],
  customUrl: "",
  judgeServer: "builtin",
  judgeUrl: "",
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

export function sanitizeProfile(raw: unknown): ProfileData | null {
  if (!raw || typeof raw !== "object") return null;
  const o = raw as Partial<ProfileData>;
  const str = (v: unknown, fb: string, re?: RegExp) =>
    typeof v === "string" && (!re || re.test(v)) ? v : fb;
  const bool = (v: unknown, fb: boolean) => (typeof v === "boolean" ? v : fb);
  const rawSel = str(o.testUrlSel, PROFILE_DEFAULTS.testUrlSel);
  // Older presets stored the free ip-api endpoint with HTTPS, which always failed.
  let testUrlSel = rawSel === IPAPI_OLD_HTTPS_URL ? IPAPI_FREE_URL : rawSel;
  let customUrl = str(o.customUrl, "");
  let target: TargetMode = o.target === "judge" ? "judge" : "public";
  let judgeServer: JudgeServer = o.judgeServer === "custom" ? "custom" : "builtin";
  let judgeUrl = str(o.judgeUrl, "");

  if (rawSel === "judge") {
    // Legacy shape: the judge option lived in testUrlSel and its custom base
    // was stored in customUrl.
    target = "judge";
    judgeServer = customUrl.trim() ? "custom" : "builtin";
    judgeUrl = customUrl;
    customUrl = "";
    testUrlSel = PROFILE_DEFAULTS.testUrlSel;
  } else if (o.target !== "public" && o.target !== "judge" && customUrl.trim()) {
    // Legacy shape without a target: a non-empty custom URL always won.
    target = "public";
    testUrlSel = PUBLIC_CUSTOM;
  }

  if (testUrlSel !== PUBLIC_CUSTOM && !PUBLIC_URL_PRESETS.includes(testUrlSel)) {
    testUrlSel = PROFILE_DEFAULTS.testUrlSel;
  }

  const proto = str(o.defaultProto, PROFILE_DEFAULTS.defaultProto);
  return {
    target,
    testUrlSel,
    customUrl,
    judgeServer,
    judgeUrl,
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

export const BUILTIN_PROFILES: {
  id: string;
  nameKey: string;
  hintKey: string;
  data: ProfileData;
}[] = [
  {
    id: "builtin:fast",
    nameKey: "prof_fast",
    hintKey: "prof_fast_hint",
    data: {
      ...PROFILE_DEFAULTS,
      concurrency: "100",
      geo: false,
      anon: false,
      tamper: false,
      tls: false,
      judgeFull: false,
    },
  },
  {
    id: "builtin:standard",
    nameKey: "prof_standard",
    hintKey: "prof_standard_hint",
    data: { ...PROFILE_DEFAULTS },
  },
  {
    id: "builtin:deep",
    nameKey: "prof_deep",
    hintKey: "prof_deep_hint",
    data: {
      ...PROFILE_DEFAULTS,
      testUrlSel: IPAPI_FREE_URL,
      timeout: "8000",
      repeats: "3",
    },
  },
  {
    id: "builtin:judge",
    nameKey: "prof_judge",
    hintKey: "prof_judge_hint",
    data: {
      ...PROFILE_DEFAULTS,
      target: "judge",
      geo: false,
      anon: false,
      tamper: false,
      tls: false,
    },
  },
];
