import { describe, expect, it } from "vitest";
import {
  buildFromParts,
  cleanTail,
  extractProxiesFromText,
  proxiesFromJson,
  stripProxyCredentials,
  tryCsvLine,
} from "./parse.ts";

describe("buildFromParts", () => {
  it("plain host:port", () => {
    expect(buildFromParts("1.2.3.4", "8080")).toBe("1.2.3.4:8080");
  });
  it("rejects bad ports", () => {
    expect(buildFromParts("1.2.3.4", "0")).toBeNull();
    expect(buildFromParts("1.2.3.4", "99999")).toBeNull();
    expect(buildFromParts("1.2.3.4", "abc")).toBeNull();
    expect(buildFromParts("", "8080")).toBeNull();
  });
  it("auth + proto prefix", () => {
    expect(buildFromParts("1.2.3.4", "8080", "u", "p", "socks5")).toBe(
      "socks5://u:p@1.2.3.4:8080"
    );
    expect(buildFromParts("1.2.3.4", "8080", "u", null, null)).toBe(
      "u@1.2.3.4:8080"
    );
  });
  it("brackets bare IPv6 hosts", () => {
    expect(buildFromParts("2001:db8::1", "8080")).toBe(
      "[2001:db8::1]:8080"
    );
    expect(buildFromParts("[2001:db8::1]", "8080")).toBe(
      "[2001:db8::1]:8080"
    );
  });
});

describe("tryCsvLine", () => {
  it("host,port", () => {
    expect(tryCsvLine("1.2.3.4,8080")).toBe("1.2.3.4:8080");
  });
  it("host,port,user,pass", () => {
    expect(tryCsvLine("1.2.3.4,8080,u,p")).toBe("u:p@1.2.3.4:8080");
  });
  it("user,pass,host,port", () => {
    expect(tryCsvLine("u,p,1.2.3.4,8080")).toBe("u:p@1.2.3.4:8080");
  });
  it("proto suffix / prefix", () => {
    expect(tryCsvLine("1.2.3.4:8080:socks5")).toBeNull(); // no delimiter, not csv
    expect(tryCsvLine("1.2.3.4,8080,socks5")).toBe("socks5://1.2.3.4:8080");
    expect(tryCsvLine("socks5,1.2.3.4,8080")).toBe("socks5://1.2.3.4:8080");
  });
  it("space columns", () => {
    expect(tryCsvLine("1.2.3.4 8080 u p")).toBe("u:p@1.2.3.4:8080");
  });
  it("keeps the documented order with a numeric password", () => {
    expect(tryCsvLine("user,1234,1.2.3.4,9090")).toBe(
      "user:1234@1.2.3.4:9090"
    );
    expect(tryCsvLine("1.2.3.4,9090,user,1234")).toBe(
      "user:1234@1.2.3.4:9090"
    );
    expect(tryCsvLine("user,1234,proxy.example.com,9090")).toBe(
      "user:1234@proxy.example.com:9090"
    );
    expect(tryCsvLine("proxy.example.com,9090,user,1234")).toBe(
      "user:1234@proxy.example.com:9090"
    );
  });
  it("brackets IPv6 columns", () => {
    expect(tryCsvLine("2001:db8::1,8080")).toBe("[2001:db8::1]:8080");
  });
  it("rejects junk", () => {
    expect(tryCsvLine("hello world foo")).toBeNull();
    expect(tryCsvLine("single")).toBeNull();
  });
});

describe("cleanTail", () => {
  it("strips trailing punctuation", () => {
    expect(cleanTail("1.2.3.4:8080,")).toBe("1.2.3.4:8080");
    expect(cleanTail("(1.2.3.4:8080)")).toBe("1.2.3.4:8080");
  });
});

describe("proxiesFromJson", () => {
  it("object with keys", () => {
    const out: string[] = [];
    proxiesFromJson(
      { host: "1.2.3.4", port: 8080, username: "u", password: "p" },
      out
    );
    expect(out).toEqual(["u:p@1.2.3.4:8080"]);
  });
  it("nested lists", () => {
    const out: string[] = [];
    proxiesFromJson({ data: { proxies: ["1.2.3.4:8080", "5.6.7.8:3128"] } }, out);
    expect(out).toEqual(["1.2.3.4:8080", "5.6.7.8:3128"]);
  });
});

describe("stripProxyCredentials", () => {
  it("removes userinfo and keeps the scheme", () => {
    expect(stripProxyCredentials("http://user:pass@1.2.3.4:8080")).toBe(
      "http://1.2.3.4:8080"
    );
    expect(stripProxyCredentials("socks5://u:p@h:1080")).toBe("socks5://h:1080");
  });
  it("works without a scheme", () => {
    expect(stripProxyCredentials("user:pass@1.2.3.4:8080")).toBe("1.2.3.4:8080");
    expect(stripProxyCredentials("u:p@ss@host:80")).toBe("host:80");
  });
  it("leaves credentials-free values alone", () => {
    expect(stripProxyCredentials("1.2.3.4:8080")).toBe("1.2.3.4:8080");
    expect(stripProxyCredentials("http://1.2.3.4:8080")).toBe("http://1.2.3.4:8080");
  });
});

describe("extractProxiesFromText", () => {
  it("plain lines + comments + numbering", () => {
    expect(
      extractProxiesFromText(
        "# comment\n1) 1.2.3.4:8080\n- user:pass@5.6.7.8:3128 # home\n\n// xx"
      )
    ).toEqual(["1.2.3.4:8080", "user:pass@5.6.7.8:3128"]);
  });
  it("dedups", () => {
    expect(extractProxiesFromText("1.2.3.4:8080\n1.2.3.4:8080")).toEqual([
      "1.2.3.4:8080",
    ]);
  });
  it("finds scheme urls inside text", () => {
    const r = extractProxiesFromText("use socks5://9.9.9.9:1080 please");
    expect(r).toContain("socks5://9.9.9.9:1080");
  });
  it("json input", () => {
    expect(
      extractProxiesFromText('[{"ip":"1.2.3.4","port":8080}]')
    ).toEqual(["1.2.3.4:8080"]);
  });
  it("empty input", () => {
    expect(extractProxiesFromText("   \n # nothing\n")).toEqual([]);
  });
  it("drops too-short fragments", () => {
    expect(extractProxiesFromText("ab")).toEqual([]);
  });
  it("drops trailing comments after proxy tokens", () => {
    expect(
      extractProxiesFromText("1.2.3.4:8080, comment\n5.6.7.8:3128; note\nuser:pass@9.9.9.9:1080 here")
    ).toEqual(["1.2.3.4:8080", "5.6.7.8:3128", "user:pass@9.9.9.9:1080"]);
  });
  it("drops invalid ports and JSON metadata", () => {
    expect(extractProxiesFromText('{"version":"1.0.0"}')).toEqual([]);
    expect(
      extractProxiesFromText("1.2.3.4:0\n1.2.3.4:65536\n://host:80")
    ).toEqual([]);
    expect(extractProxiesFromText("hello world")).toEqual([]);
  });
  it("keeps documented colon formats", () => {
    expect(
      extractProxiesFromText(
        "1.2.3.4:8080:user:pass\nuser:pass:1.2.3.4:8080\n1.2.3.4:8080:socks5",
      ),
    ).toEqual([
      "1.2.3.4:8080:user:pass",
      "user:pass:1.2.3.4:8080",
      "1.2.3.4:8080:socks5",
    ]);
  });
  it("extracts an IPv4 proxy from prose", () => {
    expect(extractProxiesFromText("use 1.2.3.4:8080 here")).toEqual([
      "1.2.3.4:8080",
    ]);
  });
});
