import type {
  ComponentDto,
  InstallationDto,
  SnapshotDto,
  ToolDto,
} from "../bindings";

export type ToolRecord = {
  tool: ToolDto;
  installations: InstallationDto[];
  providerIds: string[];
};

export function mapToolRecords(snapshot: SnapshotDto | null): ToolRecord[] {
  if (!snapshot) return [];
  const installations = new Map(
    snapshot.installations.map((installation) => [installation.id, installation]),
  );
  return snapshot.tools.map((tool) => {
    const matches = tool.installationIds.flatMap((id) => {
      const installation = installations.get(id);
      return installation ? [installation] : [];
    });
    return {
      tool,
      installations: matches,
      providerIds: [...new Set(matches.map((item) => item.providerId))],
    };
  });
}

export function updateableComponents(tool: ToolDto): ComponentDto[] {
  return tool.components.filter(
    (component) =>
      component.status.status === "update_available" &&
      component.strategies.some((strategy) => strategy.enabled),
  );
}

export function formatDate(value: string | null | undefined): string {
  if (!value) return "尚未检查";
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf())
    ? value
    : new Intl.DateTimeFormat("zh-CN", {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(parsed);
}

export function formatDuration(
  startedAt: string | null | undefined,
  finishedAt: string | null | undefined,
  now = Date.now(),
): string {
  if (!startedAt) return "—";
  const start = Date.parse(startedAt);
  const end = finishedAt ? Date.parse(finishedAt) : now;
  if (!Number.isFinite(start) || !Number.isFinite(end)) return "—";
  const seconds = Math.max(0, Math.round((end - start) / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  return `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
}
