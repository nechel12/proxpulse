import { describe, expect, it } from "vitest";
import { WebhookPanel, formatWebhookResults } from "./webhooks.ts";

describe("webhooks", () => {
  it("formats delivery results", () => {
    const t = (key: string, params?: Record<string, string | number>) =>
      key === "webhook_sent" ? `sent ${params?.r}` : `fail ${params?.e}`;
    expect(
      formatWebhookResults(
        [
          { index: 0, ok: true, message: "200" },
          { index: 1, ok: false, message: "timeout" },
        ],
        t,
      ),
    ).toBe("#1 sent 200 | #2 fail timeout");
  });

  it("trims, filters and caps URLs", () => {
    const panel = new WebhookPanel(() => {});
    panel.urls = [" https://a ", "", "https://b", ...Array(10).fill("https://c")];
    expect(panel.getUrls()).toEqual(["https://a", "https://b", ...Array(8).fill("https://c")]);
  });
});
