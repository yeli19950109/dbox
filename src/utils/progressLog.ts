import type { DisplayLogLine } from "../stores/runs";

export type CompactedLogLine = DisplayLogLine & { displayKey: string; mergedFrames?: number };

type Fragment = { source: DisplayLogLine; offset: number; message: string };
type TextLine = { fragments: Fragment[]; message: string; complete: boolean };
type Progress = { stream: string; targets: string; total: number; completed: number };
type Row = { line: CompactedLogLine; progress?: Progress; lastFragment?: Fragment };

const duration = String.raw`\d+(?:\.\d+)?(?:ms|s|m|h)`;
const headerPattern = new RegExp(
  String.raw`^mise [█░▓▒▏▎▍▌▋▊▉]+ (\d+)/(\d+) · ${duration}\r?\n$`,
);
const detailPattern = new RegExp(
  String.raw`^ {2,}(\S+@\S+) {2,}(?:downloading|verifying|extracting|installing|fetching|unpacking|compiling|configuring|building|waiting|[a-z0-9._+-]+ --version) {2,}${duration}(?: {2,}\d+(?:\.\d+)?/\d+(?:\.\d+)? [KMGT]?i?B(?: · \d+(?:\.\d+)? [KMGT]?i?B/s)?)?\r?\n$`,
);

/** Compact only recognized, adjacent mise snapshots. Stored/copied logs stay untouched. */
export function compactProgressLines(lines: readonly DisplayLogLine[]): CompactedLogLine[] {
  const rows: Row[] = [];
  const textLines = splitLines(lines);
  for (let index = 0; index < textLines.length;) {
    const current = textLines[index]!;
    const header = current.complete ? headerPattern.exec(current.message) : null;
    if (header) {
      const stream = current.fragments[0]!.source.stream;
      const targets: string[] = [];
      let end = index + 1;
      while (end < textLines.length) {
        const detail = textLines[end]!;
        if (detail.fragments[0]!.source.stream !== stream || !detail.complete) break;
        const match = detailPattern.exec(detail.message);
        if (!match) break;
        targets.push(match[1]!);
        end += 1;
      }
      const pending = textLines[end];
      // A partial detail may add another target. Keep the preceding frame until
      // the target set is known, instead of briefly replacing it with half a frame.
      const hasPendingDetail = pending && !pending.complete
        && pending.fragments[0]!.source.stream === stream && /^\s/.test(pending.message);
      const completed = Number(header[1]);
      const total = Number(header[2]);
      if (targets.length && !hasPendingDetail && new Set(targets).size === targets.length
        && Number.isSafeInteger(total) && Number.isSafeInteger(completed)
        && total > 0 && targets.length <= total && completed >= 0 && completed < total) {
        const progress: Progress = {
          stream, targets: targets.sort().join("\0"), total, completed,
        };
        const message = textLines.slice(index, end).map((line) => line.message).join("");
        const previous = rows.at(-1);
        if (previous?.progress && previous.progress.stream === stream
          && previous.progress.targets === progress.targets && previous.progress.total === total
          && previous.progress.completed <= completed) {
          // Keep the first real sequence and display key stable while the text refreshes.
          previous.line.message = message;
          previous.line.mergedFrames = (previous.line.mergedFrames ?? 1) + 1;
          previous.progress = progress;
        } else {
          const fragment = current.fragments[0]!;
          rows.push({ line: displayLine(fragment, message), progress });
        }
        index = end;
        continue;
      }
    }
    for (const fragment of current.fragments) appendPlain(rows, fragment);
    index += 1;
  }
  return rows.map((row) => row.line);
}

function displayLine(fragment: Fragment, message: string): CompactedLogLine {
  return {
    ...fragment.source,
    message,
    displayKey: `${fragment.source.sequence}:${fragment.offset}`,
  };
}

function appendPlain(rows: Row[], fragment: Fragment): void {
  const previous = rows.at(-1);
  const last = previous?.lastFragment;
  if (previous && !previous.progress && last?.source === fragment.source
    && last.offset + last.message.length === fragment.offset) {
    previous.line.message += fragment.message;
    previous.lastFragment = fragment;
  } else {
    rows.push({ line: displayLine(fragment, fragment.message), lastFragment: fragment });
  }
}

/** Reassemble logical lines across read chunks, without joining separate output streams. */
function splitLines(lines: readonly DisplayLogLine[]): TextLine[] {
  const result: TextLine[] = [];
  let fragments: Fragment[] = [];
  let stream: string | undefined;
  function flush(complete: boolean): void {
    if (!fragments.length) return;
    result.push({ fragments, message: fragments.map((part) => part.message).join(""), complete });
    fragments = [];
  }
  for (const source of lines) {
    if (source.stream !== stream) flush(false);
    stream = source.stream;
    if (!source.message.length) {
      flush(false);
      result.push({ fragments: [{ source, offset: 0, message: "" }], message: "", complete: false });
    }
    let offset = 0;
    while (offset < source.message.length) {
      const newline = source.message.indexOf("\n", offset);
      const end = newline < 0 ? source.message.length : newline + 1;
      fragments.push({ source, offset, message: source.message.slice(offset, end) });
      offset = end;
      if (newline >= 0) flush(true);
    }
  }
  flush(false);
  return result;
}
