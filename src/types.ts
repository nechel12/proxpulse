export type ProxyResult = {
  proxy: string;
  url: string;
  proto: string;
  alive: boolean;
  latency_ms: number | null;
  jitter_ms: number | null;
  success_rate: number;
  attempts: number;
  ip: string | null;
  country: string | null;
  country_code: string | null;
  city: string | null;
  isp: string | null;
  org: string | null;
  asn: string | null;
  ip_type: string | null;
  anonymity: string | null;
  tamper: string | null;
  tls: string | null;
  tls_info: string | null;
  error: string | null;
  /** Deep checks skipped at their concurrency limit: "geo", "anon", "tamper", "tls". */
  skipped?: string[];
};

/** Messages on the check channel (`CheckEvent` in check.rs). */
export type CheckEvent = (ProxyResult & { kind: "result" }) | { kind: "done" };
