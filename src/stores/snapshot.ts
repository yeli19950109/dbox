import { computed, ref } from "vue";
import { defineStore } from "pinia";
import type {
  RefreshProgressEventDto,
  RefreshScopeDto,
  SnapshotDto,
} from "../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";
import { isNewerSequence } from "../utils/sequence";
import { mapToolRecords } from "../utils/presentation";
import { useNotificationsStore } from "./notifications";

export const useSnapshotStore = defineStore("snapshot", () => {
  const snapshot = ref<SnapshotDto | null>(null);
  const loading = ref(false);
  const refreshing = ref(false);
  const error = ref<string | null>(null);
  const refreshProgress = ref<RefreshProgressEventDto[]>([]);
  const currentRefreshProgress = ref<RefreshProgressEventDto | null>(null);
  const lastRefreshSequence = new Map<string, string>();
  let lastToolSequence: string | undefined;
  let initializePromise: Promise<SnapshotDto> | null = null;
  const refreshes = new Map<string, Promise<SnapshotDto>>();
  let eventCleanup: Array<() => void> = [];
  let eventsConnected = false;

  const toolRecords = computed(() => mapToolRecords(snapshot.value));
  const providerIds = computed(() => [
    ...new Set([
      ...(snapshot.value?.providers.map((provider) => provider.providerId) ?? []),
      ...toolRecords.value.flatMap((record) => record.providerIds),
    ]),
  ]);

  function acceptSnapshot(next: SnapshotDto): void {
    snapshot.value = next;
    error.value = null;
  }

  function initialize(): Promise<SnapshotDto> {
    if (initializePromise) return initializePromise;
    loading.value = true;
    error.value = null;
    initializePromise = unwrapCommand(getTransport().commands.snapshot())
      .then((next) => {
        acceptSnapshot(next);
        return next;
      })
      .catch((reason: unknown) => {
        error.value = errorMessage(
          reason,
          "无法连接 dbox 后端。请从桌面应用启动后重试。",
        );
        throw reason;
      })
      .finally(() => {
        initializePromise = null;
        loading.value = refreshing.value;
      });
    return initializePromise;
  }

  function refresh(scope: RefreshScopeDto): Promise<SnapshotDto> {
    const key = JSON.stringify(scope);
    const active = refreshes.get(key);
    if (active) return active;
    loading.value = true;
    refreshing.value = true;
    error.value = null;
    if (refreshes.size === 0) currentRefreshProgress.value = null;
    const request = unwrapCommand(
      getTransport().commands.refresh({ scope, force: true }),
    )
      .then((next) => {
        acceptSnapshot(next);
        useNotificationsStore().push(
          "success",
          "刷新完成",
          scope.scope === "all" ? "所有 Provider 已完成检查。" : "所选范围已完成检查。",
        );
        return next;
      })
      .catch((reason: unknown) => {
        error.value = errorMessage(reason, "后端暂时不可用，无法刷新工具。");
        useNotificationsStore().push("error", "刷新失败", error.value);
        throw reason;
      })
      .finally(() => {
        refreshes.delete(key);
        refreshing.value = refreshes.size > 0;
        loading.value = refreshing.value || initializePromise !== null;
        if (!refreshing.value) currentRefreshProgress.value = null;
      });
    refreshes.set(key, request);
    return request;
  }

  function applyRefreshProgress(event: RefreshProgressEventDto): void {
    const previous = lastRefreshSequence.get(event.requestId);
    if (!isNewerSequence(event.sequence, previous)) return;
    lastRefreshSequence.set(event.requestId, event.sequence);
    currentRefreshProgress.value = event;
    refreshProgress.value = [
      event,
      ...refreshProgress.value.filter(
        (item) => item.requestId !== event.requestId,
      ),
    ].slice(0, 8);
  }

  async function connectEvents(): Promise<void> {
    if (eventsConnected) return;
    eventsConnected = true;
    const transport = getTransport();
    const [stopProgress, stopTools] = await Promise.all([
      transport.events.refreshProgress.listen(({ payload }) => {
        applyRefreshProgress(payload);
      }),
      transport.events.toolState.listen(({ payload }) => {
        if (!isNewerSequence(payload.sequence, lastToolSequence)) return;
        lastToolSequence = payload.sequence;
        void initialize().catch(() => undefined);
      }),
    ]);
    eventCleanup = [stopProgress, stopTools];
  }

  function disconnectEvents(): void {
    eventCleanup.forEach((cleanup) => cleanup());
    eventCleanup = [];
    eventsConnected = false;
  }

  return {
    snapshot,
    loading,
    refreshing,
    error,
    refreshProgress,
    currentRefreshProgress,
    toolRecords,
    providerIds,
    acceptSnapshot,
    initialize,
    refresh,
    applyRefreshProgress,
    connectEvents,
    disconnectEvents,
  };
});
