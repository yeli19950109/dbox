import type {
  ComponentDto,
  InstallationDto,
  RunDto,
  SettingsDocumentDto,
  SnapshotDto,
  ToolDto,
  UpdatePlanDto,
} from "../bindings";

export function component(
  id: string,
  status = "up_to_date",
  latestVersion: string | null = "2.0.0",
): ComponentDto {
  return {
    id,
    displayName: id === "core" ? "Core" : id,
    installationId: null,
    strategies: [
      {
        id: "provider-default",
        displayName: "Provider default",
        kind: { kind: "provider_default" },
        enabled: true,
      },
    ],
    defaultStrategy: "provider-default",
    installedVersion: "1.0.0",
    latestVersion,
    status: { status, reason: null, verification: null },
  };
}

export function installation(index: number, providerId = "npm"): InstallationDto {
  return {
    id: `${providerId}:installation-${index}`,
    providerId,
    packageKind: providerId === "brew" ? "formula" : "global",
    packageName: `package-${index}`,
    scope: "global",
    installPath: `/opt/tools/package-${index}`,
    executables: [
      { name: `package-${index}`, path: `/opt/bin/package-${index}` },
    ],
    installedVersion: "1.0.0",
    hidden: false,
  };
}

export function tool(index: number, providerId = "npm"): ToolDto {
  const installationId = `${providerId}:installation-${index}`;
  const hasUpdate = index % 3 === 0;
  const unknown = index % 5 === 0 && !hasUpdate;
  return {
    id: `${providerId}:tool-${index}`,
    displayName: index % 9 === 0 ? "Shared CLI" : `Tool ${index}`,
    installationIds: [installationId],
    components: [
      component(
        "core",
        hasUpdate ? "update_available" : unknown ? "unknown" : "up_to_date",
        unknown ? null : "2.0.0",
      ),
    ],
    categories: index % 2 ? ["developer"] : [],
    homepage: null,
    hidden: false,
    status: {
      status: hasUpdate ? "update_available" : unknown ? "unknown" : "up_to_date",
      hasUpdates: hasUpdate,
      failedComponents: 0,
      unknownComponents: unknown ? 1 : 0,
      unsupportedComponents: 0,
    },
  };
}

export function snapshotWithTools(count = 3): SnapshotDto {
  const installations = Array.from({ length: count }, (_, index) =>
    installation(index, index % 2 ? "brew" : "npm"),
  );
  const tools = Array.from({ length: count }, (_, index) =>
    tool(index, index % 2 ? "brew" : "npm"),
  );
  return {
    schemaRevision: 2,
    tools,
    installations,
    providers: [
      {
        providerId: "npm",
        enabled: true,
        status: { available: true, version: "11.0.0", detail: null },
        errors: [],
        refreshedAt: "2026-08-27T10:00:00Z",
      },
      {
        providerId: "brew",
        enabled: true,
        status: { available: true, version: "5.0.0", detail: null },
        errors: [],
        refreshedAt: "2026-08-27T10:00:00Z",
      },
    ],
    catalogDiagnostics: [],
    catalogErrors: [],
    configRevision: "config-1",
    stateRevision: "state-1",
    environmentRevision: "environment-1",
    refreshedAt: "2026-08-27T10:00:00Z",
  };
}

export function settingsDocument(): SettingsDocumentDto {
  return {
    revision: "settings-1",
    settings: {
      providerEnabled: { npm: true, brew: true },
      defaultTimeoutSeconds: "120",
      logRetention: { maxFiles: 50, maxTotalBytes: "10485760" },
      executableOverrides: {},
      componentStrategies: {},
    },
  };
}

export function updatePlan(index = 0): UpdatePlanDto {
  return {
    planId: `plan-${index}`,
    planHash: `hash-${index}`,
    toolId: "npm:tool-0",
    installationId: "npm:installation-0",
    componentId: "core",
    strategyId: "provider-default",
    command: {
      program: "/opt/bin/npm",
      args: ["update", "--global", "package-0"],
      cwd: null,
      env: {},
      timeoutSeconds: "120",
      successExitCodes: [0],
      networkRequired: true,
      postCheck: null,
      displayCommand: "npm update --global package-0",
    },
    configRevision: "config-1",
    environmentRevision: "environment-1",
    versionSnapshot: "version-1",
    createdAt: "2026-08-27T10:00:00Z",
    expiresAt: "2026-08-27T10:05:00Z",
  };
}

export function failedRun(id = "run-1"): RunDto {
  return {
    id,
    toolId: "npm:tool-0",
    subject: "tool_update",
    operation: null,
    resourceNames: [],
    componentIds: ["core"],
    status: "failed",
    createdAt: "2026-08-27T10:00:00Z",
    startedAt: "2026-08-27T10:00:01Z",
    finishedAt: "2026-08-27T10:00:03Z",
    batchId: "batch-1",
    retryOf: null,
    hasLog: true,
    summary: {
      exitCode: 1,
      outputTail: "failed safely",
      verification: { status: "verification_unknown", reason: null },
      error: "fixture failure",
    },
  };
}
