<template>
  <div class="view tools-view">
    <section
      v-if="providerErrors.length"
      class="provider-warning"
      role="status"
      aria-label="Provider 部分失败"
    >
      <strong>部分 Provider 未完成</strong>
      <span v-for="report in providerErrors" :key="report.providerId">
        {{ report.providerId }}：{{ report.errors.map((item) => item.summary).join("；") }}
      </span>
    </section>

    <div class="tools-layout">
      <aside v-if="!compactFilters" class="filter-sidebar" aria-label="工具筛选">
        <header class="filter-sidebar__header">
          <h2>筛选</h2>
          <button type="button" class="text-button" @click="resetSideFilters">重置</button>
        </header>
        <ToolFilters
          v-model:status="statusFilter"
          v-model:provider="providerFilter"
          v-model:category="categoryFilter"
          v-model:include-hidden="includeHidden"
          :providers="providers"
          :provider-counts="providerCounts"
          :categories="categories"
        />
      </aside>
      <dialog v-else ref="filterDialog" class="filter-drawer" aria-label="工具筛选" @click="closeFilterBackdrop">
        <header class="filter-sidebar__header">
          <h2>筛选</h2>
          <button type="button" class="text-button" @click="resetSideFilters">重置</button>
          <button type="button" class="icon-button" aria-label="关闭筛选" @click="filterDialog?.close()">×</button>
        </header>
        <ToolFilters
          v-model:status="statusFilter"
          v-model:provider="providerFilter"
          v-model:category="categoryFilter"
          v-model:include-hidden="includeHidden"
          :providers="providers"
          :provider-counts="providerCounts"
          :categories="categories"
        />
        <button type="button" class="button primary filter-drawer__done" @click="filterDialog?.close()">查看 {{ filteredRecords.length }} 项结果</button>
      </dialog>
      <div class="tools-content">
        <section class="tool-search" aria-label="搜索工具">
          <label class="search-field">
            <span class="sr-only">搜索工具、包或可执行文件</span>
            <span aria-hidden="true">⌕</span>
            <input
              v-model="searchInput"
              type="search"
              placeholder="搜索工具、包或可执行文件"
            />
          </label>
          <button v-if="compactFilters" class="button secondary" type="button" @click="filterDialog?.showModal()">
            筛选<span v-if="activeFilterCount"> · {{ activeFilterCount }}</span>
          </button>
          <button
            class="button primary"
            type="button"
            :disabled="updateBusy || !visibleUpdates.length"
            title="直接更新当前筛选结果中的所有可更新项"
            @click="updateNow(visibleUpdates)"
          >
            一键更新<span v-if="visibleUpdates.length"> · {{ visibleUpdates.length }} 项</span>
          </button>
        </section>

        <section
          v-if="store.refreshing"
          class="refresh-progress"
          role="status"
          aria-live="polite"
        >
          <div class="refresh-progress__summary">
            <span class="spinner refresh-progress__spinner" aria-hidden="true" />
            <div>
              <strong>{{ refreshProgressTitle }}</strong>
              <span>{{ refreshProgressDetail }}</span>
            </div>
            <span v-if="refreshProgressTotal" class="refresh-progress__count">
              {{ refreshProgressCompleted }} / {{ refreshProgressTotal }}
            </span>
          </div>
          <div
            class="refresh-progress__track"
            :data-indeterminate="!refreshProgressTotal"
            role="progressbar"
            :aria-label="refreshItemProgress ? '当前来源的软件进度' : '刷新来源进度'"
            aria-valuemin="0"
            :aria-valuemax="refreshProgressTotal || undefined"
            :aria-valuenow="refreshProgressTotal ? refreshProgressCompleted : undefined"
            :aria-valuetext="refreshProgressDetail"
          >
            <span :style="refreshProgressBarStyle" />
          </div>
        </section>

        <div class="results-bar">
          <span>
            <strong>{{ filteredRecords.length }}</strong> / {{ store.toolRecords.length }} 项
          </span>
          <span v-if="lastChecked">上次检查：{{ lastChecked }}</span>
        </div>
        <p v-if="providerFilter === 'pi-extensions'" class="provider-scope-note">
          Pi 用户级托管 npm 扩展包；按配置的版本范围检查，固定版本不自动更新。
        </p>

        <div v-if="store.error" class="inline-error" role="alert">
          <span>{{ store.error }}</span>
          <button type="button" class="text-button" @click="initialize">重试</button>
        </div>
        <div v-if="runs.error && !store.error" class="inline-error" role="alert">
          {{ runs.error }}
        </div>
        <AppLoading v-if="store.loading && !store.snapshot" label="正在读取工具快照…" />
        <AppEmptyState
          v-else-if="!filteredRecords.length"
          title="没有符合条件的工具"
          description="调整筛选条件，或刷新 Provider 重新扫描全局安装项。"
          mark="⌁"
        >
          <button class="button secondary" type="button" @click="clearFilters">
            清除筛选
          </button>
        </AppEmptyState>
        <div
          v-else
          ref="scrollElement"
          class="virtual-list tool-virtual-list"
          data-testid="tool-virtual-list"
        >
          <div class="virtual-list__sizer" :style="{ height: `${virtualizer.getTotalSize()}px` }">
            <div
              v-for="virtualRow in virtualRows"
              :key="filteredRecords[virtualRow.index]?.tool.id"
              :data-index="virtualRow.index"
              class="virtual-list__row"
              :style="{ transform: `translateY(${virtualRow.start}px)` }"
            >
              <ToolCard
                v-if="filteredRecords[virtualRow.index]"
                :record="filteredRecords[virtualRow.index]!"
                :selected="selected"
                :update-disabled="updateBusy"
                @details="openDetails(filteredRecords[virtualRow.index]!, $event)"
                @select-updates="selectToolUpdates(filteredRecords[virtualRow.index]!)"
                @update="updateNow(selectionsFor([filteredRecords[virtualRow.index]!]))"
              />
            </div>
          </div>
        </div>

        <div v-if="selected.size" class="selection-bar" role="region" aria-label="批量更新选择">
          <span>已选择 <strong>{{ selected.size }}</strong> 个 Component</span>
          <button type="button" class="text-button" @click="selected = new Set()">
            清除
          </button>
          <button type="button" class="button secondary" :disabled="updateBusy || !selectedUpdates.length" @click="previewSelected">
            预览更新
          </button>
          <button type="button" class="button primary" :disabled="updateBusy || !selectedUpdates.length" @click="updateNow(selectedUpdates)">
            更新所选
          </button>
        </div>

      </div>
    </div>

    <aside
      v-if="detailRecord"
      class="detail-panel"
      role="dialog"
      aria-modal="true"
      :aria-label="`${detailRecord.tool.displayName} 详情`"
      @keydown.esc="closeDetails"
    >
      <header>
        <div>
          <p class="eyebrow">Tool detail</p>
          <h2>{{ detailRecord.tool.displayName }}</h2>
        </div>
        <button
          ref="detailClose"
          type="button"
          class="icon-button"
          aria-label="关闭工具详情"
          @click="closeDetails"
        >
          ×
        </button>
      </header>
      <section>
        <h3>安装来源</h3>
        <article
          v-for="installation in detailRecord.installations"
          :key="installation.id"
          class="detail-block"
        >
          <strong>{{ providerLabel(installation.providerId) }} · {{ installation.packageKind }}</strong>
          <code>{{ installation.packageName }}</code>
          <p v-if="installation.providerId === 'mise'">{{ miseInstallationLabel(installation) }}</p>
          <p>{{ installation.installPath ?? "Provider 未返回安装路径" }}</p>
          <ul>
            <li v-for="executable in installation.executables" :key="executable.path">
              <span>{{ executable.name }}</span>
              <code>{{ executable.path }}</code>
            </li>
          </ul>
        </article>
      </section>
      <section>
        <h3>Component 与策略</h3>
        <article
          v-for="component in detailRecord.tool.components"
          :key="component.id"
          class="detail-block"
        >
          <strong>{{ component.displayName }}</strong>
          <p v-if="detailRecord.providerIds.includes('pi-extensions')">
            已安装 {{ component.installedVersion ?? "未知" }} · 可用版本 {{ component.latestVersion ?? "尚未确定" }}
          </p>
          <p v-if="component.status.reason" class="reason">
            {{ component.status.reason.summary }}
          </p>
          <label>
            <span>更新策略</span>
            <select
              :value="strategyFor(detailRecord, component.id)"
              :disabled="!component.strategies.some((item) => item.enabled)"
              @change="setStrategy(detailRecord, component.id, ($event.target as HTMLSelectElement).value)"
            >
              <option
                v-for="strategy in component.strategies"
                :key="strategy.id"
                :value="strategy.id"
                :disabled="!strategy.enabled"
              >
                {{ strategy.displayName }}{{ strategy.enabled ? "" : "（不可用）" }}
              </option>
            </select>
          </label>
        </article>
      </section>
      <section>
        <h3>Provider 诊断</h3>
        <div
          v-for="providerId in detailRecord.providerIds"
          :key="providerId"
          class="provider-diagnostic"
        >
          <strong>{{ providerId }}</strong>
          <span>{{ providerDiagnostic(providerId) }}</span>
        </div>
      </section>
    </aside>
    <button
      v-if="detailRecord"
      class="detail-backdrop"
      type="button"
      aria-label="关闭工具详情"
      @click="closeDetails"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { useDebounceFn, useMediaQuery } from "@vueuse/core";
import { useVirtualizer } from "@tanstack/vue-virtual";
import { useRouter } from "vue-router";
import type { UpdateSelectionDto } from "../bindings";
import AppEmptyState from "../components/AppEmptyState.vue";
import AppLoading from "../components/AppLoading.vue";
import ToolCard from "../components/ToolCard.vue";
import ToolFilters from "../components/ToolFilters.vue";
import { useRunsStore } from "../stores/runs";
import { useSnapshotStore } from "../stores/snapshot";
import type { ToolRecord } from "../utils/presentation";
import {
  compareToolUpdatePriority,
  formatDate,
  miseInstallationLabel,
  providerLabel,
  toolHasUpdates,
  updateableComponents,
} from "../utils/presentation";

const store = useSnapshotStore();
const runs = useRunsStore();
const router = useRouter();
const searchInput = ref("");
const search = ref("");
const statusFilter = ref("all");
const providerFilter = ref("all");
const categoryFilter = ref("all");
const includeHidden = ref(false);
const compactFilters = useMediaQuery("(max-width: 760px)");
const filterDialog = ref<HTMLDialogElement | null>(null);
const activeFilterCount = computed(() =>
  Number(statusFilter.value !== "all") + Number(providerFilter.value !== "all") +
  Number(categoryFilter.value !== "all") + Number(includeHidden.value),
);
const selected = ref<Set<string>>(new Set());
const previewing = ref(false);
const updateBusy = computed(() => store.loading || runs.startingUpdate || runs.executing || previewing.value);
const strategies = ref<Record<string, string>>({});
const detailRecord = ref<ToolRecord | null>(null);
const detailClose = ref<HTMLButtonElement | null>(null);
const detailTrigger = ref<HTMLElement | null>(null);
const scrollElement = ref<HTMLElement | null>(null);

const updateSearch = useDebounceFn((value: string) => {
  search.value = value.trim().toLocaleLowerCase();
}, 120);

watch(searchInput, (value) => void updateSearch(value));

const providers = computed(() => store.providerIds);
const providerCounts = computed(() => Object.fromEntries(
  providers.value.map((id) => [id, store.toolRecords.filter((record) => record.providerIds.includes(id)).length]),
));
const categories = computed(() => [
  ...new Set(store.toolRecords.flatMap((record) => record.tool.categories)),
]);
const providerErrors = computed(
  () => store.snapshot?.providers.filter((provider) => provider.errors.length) ?? [],
);
const lastChecked = computed(() => formatDate(store.snapshot?.refreshedAt));
const refreshItemProgress = computed(() => (store.currentRefreshProgress?.itemTotal ?? 0) > 0);
const refreshProgressTotal = computed(() => refreshItemProgress.value
  ? store.currentRefreshProgress?.itemTotal ?? 0
  : store.currentRefreshProgress?.total ?? 0,
);
const refreshProgressCompleted = computed(() => Math.min(refreshProgressTotal.value, Math.max(0,
  (refreshItemProgress.value
    ? store.currentRefreshProgress?.itemCompleted
    : store.currentRefreshProgress?.completed) ?? 0,
)));
const refreshProgressTitle = computed(() => {
  const progress = store.currentRefreshProgress;
  if (progress?.phase === "completed") return "刷新完成";
  if (progress?.phase === "failed") return "刷新未完成";
  if (!progress?.providerId) return "正在准备刷新";
  const name = progress.packageName ?? providerLabel(progress.providerId);
  if (progress.stage === "scan") return `扫描软件 · ${name}`;
  if (progress.stage === "check_updates") return `检查更新 · ${name}`;
  return `正在检查 ${name}`;
});
const refreshProgressDetail = computed(() => {
  const progress = store.currentRefreshProgress;
  if (!progress?.providerId) {
    return progress?.phase === "completed" ? "所有来源已完成检查" : "正在连接后端并准备检查…";
  }
  const source = `${providerLabel(progress.providerId)}${progress.total
    ? ` · 来源 ${Math.min((progress.completed ?? 0) + 1, progress.total)} / ${progress.total}` : ""}`;
  if (refreshItemProgress.value) {
    const action = progress.stage === "scan" ? "已扫描" : "已检查";
    const batch = progress.stage === "check_updates" && !progress.packageName
      && refreshProgressCompleted.value < refreshProgressTotal.value ? " · 正在批量查询版本" : "";
    return `${source} · ${action} ${refreshProgressCompleted.value} / ${refreshProgressTotal.value} 项${batch}`;
  }
  if (progress.stage === "scan") return `${source} · 正在读取软件清单…`;
  if (progress.stage === "check_updates") return `${source} · 正在查询版本…`;
  return `${source} · 正在检测可用性…`;
});
const refreshProgressBarStyle = computed(() => {
  if (!refreshProgressTotal.value) return undefined;
  const percent = Math.round(
    (refreshProgressCompleted.value / refreshProgressTotal.value) * 100,
  );
  return { width: `${percent}%` };
});

const filteredRecords = computed(() =>
  store.toolRecords.filter((record) => {
    if (!includeHidden.value && (record.tool.hidden || record.installations.some((item) => item.hidden))) {
      return false;
    }
    if (
      providerFilter.value !== "all" &&
      !record.providerIds.includes(providerFilter.value)
    ) {
      return false;
    }
    if (
      categoryFilter.value !== "all" &&
      !record.tool.categories.includes(categoryFilter.value)
    ) {
      return false;
    }
    if (statusFilter.value === "updates" && !toolHasUpdates(record.tool)) return false;
    if (
      statusFilter.value === "unknown" &&
      record.tool.status.status !== "unknown" &&
      !record.tool.status.unknownComponents &&
      !record.tool.status.unsupportedComponents
    ) return false;
    if (
      statusFilter.value === "failed" &&
      !["check_failed", "update_failed"].includes(record.tool.status.status) &&
      !record.tool.status.failedComponents
    ) return false;
    if (!search.value) return true;
    const haystack = [
      record.tool.displayName,
      record.tool.id,
      ...record.installations.flatMap((installation) => [
        installation.providerId,
        installation.packageName,
        ...installation.executables.flatMap((executable) => [executable.name, executable.path]),
      ]),
    ]
      .join(" ")
      .toLocaleLowerCase();
    return haystack.includes(search.value);
  }).sort(compareToolUpdatePriority),
);

const visibleUpdates = computed(() => selectionsFor(filteredRecords.value));
const selectedUpdates = computed(() => selectionsFor(store.toolRecords).filter((selection) =>
  selected.value.has(`${selection.toolId}\u0000${selection.componentId}`),
));

const virtualizer = useVirtualizer(
  computed(() => ({
    count: filteredRecords.value.length,
    getScrollElement: () => scrollElement.value,
    estimateSize: () => 58,
    overscan: 5,
    initialRect: { width: 900, height: 720 },
  })),
);
const virtualRows = computed(() => virtualizer.value.getVirtualItems());

function key(record: ToolRecord, componentId: string): string {
  return `${record.tool.id}\u0000${componentId}`;
}

function selectToolUpdates(record: ToolRecord): void {
  const next = new Set(selected.value);
  const components = updateableComponents(record.tool);
  const allSelected = components.every((component) =>
    next.has(key(record, component.id)),
  );
  for (const component of components) {
    const value = key(record, component.id);
    if (allSelected) next.delete(value);
    else next.add(value);
  }
  selected.value = next;
}

function strategyFor(record: ToolRecord, componentId: string): string {
  const component = record.tool.components.find((item) => item.id === componentId);
  return (
    strategies.value[key(record, componentId)] ??
    component?.defaultStrategy ??
    component?.strategies.find((item) => item.enabled)?.id ??
    ""
  );
}

function setStrategy(record: ToolRecord, componentId: string, strategy: string): void {
  strategies.value = { ...strategies.value, [key(record, componentId)]: strategy };
}

function selectionsFor(records: ToolRecord[]): UpdateSelectionDto[] {
  return records.flatMap((record) =>
    updateableComponents(record.tool).map((component) => ({
      toolId: record.tool.id,
      componentId: component.id,
      strategyId: strategyFor(record, component.id) || null,
      targetVersion: null,
    })),
  );
}

async function updateNow(selections: UpdateSelectionDto[]): Promise<void> {
  if (updateBusy.value || !selections.length) return;
  const execution = runs.updateNow(selections).catch(() => undefined);
  selected.value = new Set();
  await router.push({ name: "runs" });
  await execution;
}

async function previewSelected(): Promise<void> {
  if (updateBusy.value || !selectedUpdates.value.length) return;
  previewing.value = true;
  try {
    await runs.preview(selectedUpdates.value);
    await router.push({ name: "confirm" });
  } catch {
    // The store exposes the backend error for the inline message.
  } finally {
    previewing.value = false;
  }
}

function openDetails(record: ToolRecord, event: MouseEvent): void {
  detailRecord.value = record;
  detailTrigger.value = event.currentTarget as HTMLElement;
  void nextTick(() => detailClose.value?.focus());
}

function closeDetails(): void {
  detailRecord.value = null;
  void nextTick(() => detailTrigger.value?.focus());
}

function providerDiagnostic(providerId: string): string {
  const report = store.snapshot?.providers.find((item) => item.providerId === providerId);
  if (!report) return "尚无诊断";
  if (!report.enabled) return "已在设置中停用";
  if (report.errors.length) return report.errors.map((item) => item.summary).join("；");
  if (!report.status) return "等待刷新";
  if (!report.status?.available) return report.status?.detail ?? "当前不可用";
  return `${report.status.version ?? "版本未知"} · ${formatDate(report.refreshedAt)}`;
}

function clearFilters(): void {
  searchInput.value = "";
  search.value = "";
  resetSideFilters();
}

function resetSideFilters(): void {
  statusFilter.value = "all";
  providerFilter.value = "all";
  categoryFilter.value = "all";
  includeHidden.value = false;
}

function closeFilterBackdrop(event: MouseEvent): void {
  if (event.target !== filterDialog.value) return;
  const bounds = filterDialog.value.getBoundingClientRect();
  if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) {
    filterDialog.value.close();
  }
}

function initialize(): void {
  void store.initialize().catch(() => undefined);
}
onMounted(initialize);
</script>

<style scoped>
.provider-scope-note {
  margin: 0 0 12px;
  color: var(--muted);
  font-size: 12px;
}

.refresh-progress__summary strong {
  overflow-wrap: anywhere;
}

.refresh-progress__count {
  flex-shrink: 0;
}

@media (max-width: 760px) {
  .tool-search {
    flex-wrap: wrap;
  }

  .tool-search .search-field {
    flex-basis: 100%;
  }

  .selection-bar {
    flex-wrap: wrap;
  }
}
</style>
