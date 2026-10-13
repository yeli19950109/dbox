import { describe, expect, it } from "vitest";
import type { DisplayLogLine } from "../stores/runs";
import { compactProgressLines } from "./progressLog";

function line(sequence: number, message: string, stream = "stderr"): DisplayLogLine {
  return { sequence: String(sequence), timestamp: "2026-10-13T06:00:00Z", stream, message };
}

function frame(seconds: number, targets = ["python@3.14.8"], stage = "downloading"): string {
  return `mise ███░░░░░░░░░░░░░ 0/${targets.length} · ${seconds}.0s\n`
    + targets.map((target) => `  ${target}  ${stage}  ${seconds}.0s\n`).join("");
}

describe("mise progress display compaction", () => {
  it("replaces the eight observed snapshots while retaining the first real sequence", () => {
    const messages = [
      "mise ░░░░░░░░░░░░░░░░ 0/1 · 3.0s\n  python@3.14.8  downloading  3.0s\n",
      "mise ░░░░░░░░░░░░░░░░ 0/1 · 6.0s\n  python@3.14.8  downloading  6.0s  2.1/26.7 MB · 2.3 MB/s\n",
      "mise ███░░░░░░░░░░░░░ 0/1 · 9.0s\n  python@3.14.8  downloading  9.0s  7.3/26.7 MB · 1.9 MB/s\n",
      "mise █████░░░░░░░░░░░ 0/1 · 12.0s\n  python@3.14.8  downloading  12.0s  13.4/26.7 MB · 1.9 MB/s\n",
      "mise ███████░░░░░░░░░ 0/1 · 15.0s\n  python@3.14.8  downloading  15.0s  18.7/26.7 MB · 1.9 MB/s\n",
      "mise ██████████░░░░░░ 0/1 · 18.0s\n  python@3.14.8  downloading  18.0s  24.3/26.7 MB · 1.9 MB/s\n",
      "mise ███████████░░░░░ 0/1 · 21.0s\n  python@3.14.8  verifying  21.0s  26.7/26.7 MB · 20.9 MB/s\n",
      "mise ███████████░░░░░ 0/1 · 24.0s\n  python@3.14.8  python --version  24.0s  26.7/26.7 MB · 6.2 MB/s\n",
    ];
    const input = messages.map((message, index) => Object.freeze(line(index + 8, message)));
    Object.freeze(input);
    expect(compactProgressLines(input)).toEqual([{
      ...input[0], message: messages.at(-1), displayKey: "8:0", mergedFrames: 8,
    }]);
    expect(input.map((item) => item.message)).toEqual(messages);
  });

  it("handles multiple frames and normal output in one read chunk with unique display keys", () => {
    const prefix = "mise by @jdx – installing 1 tool\n";
    const suffix = "mise ✓ python@3.14.8  24.9s\n";
    const result = compactProgressLines([line(5, prefix + frame(3) + frame(6) + suffix)]);
    expect(result.map((item) => item.message)).toEqual([prefix, frame(6), suffix]);
    expect(new Set(result.map((item) => item.displayKey)).size).toBe(result.length);
    expect(result.map((item) => item.sequence)).toEqual(["5", "5", "5"]);
  });

  it("reassembles headers and detail lines across arbitrary chunk boundaries", () => {
    const message = frame(3) + frame(6);
    const chunks = [message.slice(0, 7), message.slice(7, 29), message.slice(29, 85), message.slice(85)];
    expect(compactProgressLines(chunks.map((chunk, index) => line(index + 1, chunk))))
      .toEqual([{ ...line(1, frame(6)), displayKey: "1:0", mergedFrames: 2 }]);
  });

  it("retains the previous frame and all partial output until a detail line is complete", () => {
    const first = line(1, frame(3));
    const second = frame(6);
    const incomplete = compactProgressLines([first, line(2, second.slice(0, -1))]);
    expect(incomplete.map((item) => item.message)).toEqual([frame(3), second.slice(0, -1)]);
    const complete = compactProgressLines([first, line(2, second.slice(0, -1)), line(3, "\n")]);
    expect(complete.map((item) => item.message)).toEqual([frame(6)]);
    expect(complete[0]!.displayKey).toBe(incomplete[0]!.displayKey);
  });

  it("does not mistake a complete header without details for a replaceable frame", () => {
    const header = "mise ███░░░░░░░░░░░░░ 0/1 · 6.0s\n";
    expect(compactProgressLines([line(1, frame(3)), line(2, header)]).map((item) => item.message))
      .toEqual([frame(3), header]);
  });

  it("keeps changing target sets, version changes, and task counter resets separate", () => {
    const messages = [frame(3), frame(6, ["node@24.0.0"]), frame(9, ["node@24.1.0"]),
      frame(12, ["node@24.1.0"]).replace("0/1", "1/2"),
      frame(15, ["node@24.1.0"]).replace("0/1", "0/2")];
    expect(compactProgressLines(messages.map((message, index) => line(index, message))).map((item) => item.message))
      .toEqual(messages);
  });

  it("allows task order changes but waits for the whole multiple-tool frame", () => {
    const targets = ["python@3.14.8", "node@24.0.0"];
    const first = line(1, frame(3, targets));
    const second = frame(6, [...targets].reverse(), "verifying");
    expect(compactProgressLines([first, line(2, second.slice(0, -1))]).map((item) => item.message))
      .toEqual([frame(3, targets), second.slice(0, -1)]);
    expect(compactProgressLines([first, line(2, second)]).map((item) => item.message))
      .toEqual([second]);
  });

  it.each([
    "warning: checksum retry\n",
    "error: download failed\n",
    "  python@3.14.8  failed  9.0s\n",
    "  python@3.14.8  unexpected diagnostic\n",
    "mise ████████████████ 1/1 · installed 1 tool in 24.9s\n",
    "mise ████████████████ 1/1 · resolved 1 tool in 4ms\n",
  ])("preserves and stops merging at ordinary, error or completion output: %s", (barrier) => {
    const result = compactProgressLines([line(1, frame(3)), line(2, barrier), line(3, frame(6))]);
    expect(result.map((item) => item.message)).toEqual([frame(3), barrier, frame(6)]);
  });

  it("retains repeated ordinary messages and the original chunk grouping", () => {
    const input = [line(1, "working\nworking\n"), line(2, "working\nworking\n")];
    expect(compactProgressLines(input).map(({ displayKey: _key, ...item }) => item)).toEqual(input);
  });

  it("does not merge across another stream, including a partial interleaved frame", () => {
    const input = [line(1, frame(3)), line(2, frame(6), "stdout"), line(3, frame(9))];
    expect(compactProgressLines(input).map((item) => item.message)).toEqual(input.map((item) => item.message));
    const incomplete = [line(1, frame(3).slice(0, 8)), line(2, "notice\n", "stdout"), line(3, frame(3).slice(8))];
    expect(compactProgressLines(incomplete).map((item) => item.message)).toEqual(incomplete.map((item) => item.message));
  });

  it("keeps unknown or malformed snapshots unchanged", () => {
    const input = [frame(3).replace("downloading", "unexpected"), frame(6).replace("0/1", "1/1"),
      frame(9).replace("0/1", "0/0"), frame(12).replace("0/1", "2/1")]
      .map((message, index) => line(index, message));
    expect(compactProgressLines(input).map((item) => item.message)).toEqual(input.map((item) => item.message));
  });

  it("preserves completed multi-tool snapshots with duplicate or excessive targets", () => {
    const messages = [frame(3, ["python@3.14.8", "python@3.14.8"]),
      frame(6, ["python@3.14.8", "python@3.14.8"]),
      frame(9, ["python@3.14.8", "node@24.0.0"]).replace("0/2", "0/1")];
    expect(compactProgressLines(messages.map((message, index) => line(index, message))).map((item) => item.message))
      .toEqual(messages);
  });
});
