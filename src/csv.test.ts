import { describe, expect, it } from "vitest";
import { csvCell } from "./csv.ts";

describe("csvCell", () => {
  it("passes plain values through", () => {
    expect(csvCell("1.2.3.4:8080")).toBe("1.2.3.4:8080");
    expect(csvCell(123)).toBe("123");
    expect(csvCell(null)).toBe("");
  });

  it("quotes commas, quotes, newlines and carriage returns", () => {
    expect(csvCell("user,pass")).toBe('"user,pass"');
    expect(csvCell('a"b')).toBe('"a""b"');
    expect(csvCell("line\nbreak")).toBe('"line\nbreak"');
    expect(csvCell("line\rbreak")).toBe('"line\rbreak"');
  });

  it("guards spreadsheet formula characters", () => {
    expect(csvCell("=cmd|' /C calc'!A0")).toBe("'=cmd|' /C calc'!A0");
    expect(csvCell("+123")).toBe("'+123");
    expect(csvCell("-123")).toBe("'-123");
    expect(csvCell("@SUM(A1)")).toBe("'@SUM(A1)");
    expect(csvCell("\t=1")).toBe("'\t=1");
  });
});
