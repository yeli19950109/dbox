import type { ExtensionChangedEventDto, McpChangedEventDto } from "../bindings";
import { getTransport } from "../api/transport";
import { isNewerSequence } from "./sequence";
export function resourceEvents(
  kind: "skill" | "mcp",
  reload: () => Promise<void>,
) {
  let cleanup: (() => void) | undefined;
  let connecting: Promise<void> | undefined;
  let lastSequence: string | undefined;
  let generation = 0;
  const refresh = () => void reload();
  async function connect() {
    if (cleanup || connecting) return connecting;
    const current = ++generation;
    const channel =
      kind === "skill"
        ? getTransport().events.skillsChanged
        : getTransport().events.mcpChanged;
    connecting = (async () => {
      const stop = await channel.listen((event) => {
        const payload: ExtensionChangedEventDto | McpChangedEventDto =
          event.payload;
        if (!isNewerSequence(payload.sequence, lastSequence)) return;
        lastSequence = payload.sequence;
        refresh();
      });
      if (current !== generation) {
        stop();
        return;
      }
      cleanup = stop;
      window.addEventListener("dbox:reconnected", refresh);
      window.addEventListener("focus", refresh);
    })();
    try {
      await connecting;
    } finally {
      connecting = undefined;
    }
  }
  function disconnect() {
    generation++;
    cleanup?.();
    cleanup = undefined;
    window.removeEventListener("dbox:reconnected", refresh);
    window.removeEventListener("focus", refresh);
  }
  return { connect, disconnect };
}
