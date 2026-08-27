import { describe, expect, it } from "vitest";
import { isNewerSequence } from "./sequence";

describe("decimal event sequence", () => {
  it("compares values beyond JavaScript's safe integer range without precision loss", () => {
    expect(isNewerSequence("9007199254740993", "9007199254740992")).toBe(true);
    expect(isNewerSequence("9007199254740992", "9007199254740993")).toBe(false);
  });

  it("rejects malformed and duplicate sequences", () => {
    expect(isNewerSequence("1", undefined)).toBe(true);
    expect(isNewerSequence("1", "1")).toBe(false);
    expect(isNewerSequence("unknown", "1")).toBe(false);
  });
});
