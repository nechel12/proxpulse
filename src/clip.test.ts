import { describe, expect, it } from "vitest";
import { clip } from "./clip.ts";

describe("clip", () => {
  it("keeps short strings unchanged", () => {
    expect(clip("ok", 2)).toBe("ok");
  });

  it("does not split a surrogate pair", () => {
    const emoji = "\u{1F600}";
    expect(clip(`a${emoji}b`, 2)).toBe(`a${emoji}`);
  });
});
