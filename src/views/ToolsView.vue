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

    <section class="toolbar" aria-label="工具筛选">
      <label class="search-field">
        <span class="sr-only">搜索工具、包或可执行文件</span>
        <span aria-hidden="true">⌕</span>
        <input
          v-model="searchInput"
          type="search"
          placeholder="搜索工具、包或可执行文件"
        />
      </label>
      <label>
        <span>状态</span>
        <select v-model="statusFilter">
          <option value="all">全部</option>
          <option value="updates">有更新</option>
          <option value="unknown">未知</option>
          <option value="failed">失败</option>
        </select>
      </label>
      <label>
        <span>Provider</span>
        <select v-model="providerFilter">
          <option value="all">全部来源</option>
          <option v-for="provider in providers" :key="provider" :value="provider">
            {{ provider }}
          </option>
        </select>
      </label>
      <label>
        <span>类别</span>
        <select v-model="categoryFilter">
          <option value="all">全部类别</option>
          <option v-for="category in categories" :key="category" :value="category">
            {{ category }}
          </option>
        </select>
      </label>
      <label class="check-control">
        <input v-model="includeHidden" type="checkbox" />
        显示已隐藏
      </label>
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
          {{ refreshProgressPosition }} / {{ refreshProgressTotal }}
        </span>
      </div>
      <div
        class="refresh-progress__track"
        :data-indeterminate="!refreshProgressTotal"
        role="progressbar"
        aria-label="刷新工具进度"
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
      <div class="provider-refreshes" aria-label="按 Provider 刷新">
        <button
          type="button"
          class="button primary compact"
          :disabled="store.loading"
          @click="refreshAll"
        >
          <span aria-hidden="true">↻</span>
          {{ store.loading ? "正在刷新" : "刷新全部" }}
        </button>
        <button
          v-for="provider in providers"
          :key="provider"
          type="button"
          class="text-button"
          @click="refreshProvider(provider)"
        >
          刷新 {{ provider }}
        </button>
      </div>
    </div>

    <div v-if="store.error" class="inline-error" role="alert">
      <span>{{ store.error }}</span>
      <button type="button" class="text-button" @click="initialize">重试</button>
    </div>
    <AppLoading v-else-if="store.loading && !store.snapshot" label="正在读取工具快照…" />
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
          :ref="measureVirtualRow"
          :data-index="virtualRow.index"
          class="virtual-list__row"
          :style="{ transform: `translateY(${virtualRow.start}px)` }"
        >
          <ToolCard
            v-if="filteredRecords[virtualRow.index]"
            :record="filteredRecords[virtualRow.index]!"
            :selected="selected"
            @details="openDetails(filteredRecords[virtualRow.index]!, $event)"
            @toggle="toggleComponent(filteredRecords[virtualRow.index]!, $event)"
            @select-updates="selectToolUpdates(filteredRecords[virtualRow.index]!)"
            @refresh="refreshTool(filteredRecords[virtualRow.index]!.tool.id)"
          />
        </div>
      </div>
    </div>

    <div v-if="selected.size" class="selection-bar" role="region" aria-label="批量更新选择">
      <span>已选择 <strong>{{ selected.size }}</strong> 个 Component</span>
      <button type="button" class="text-button" @click="selected = new Set()">
        清除
      </button>
      <button type="button" class="button primary" @click="previewSelected">
        预览更新
      </button>
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
          <strong>{{ installation.providerId }} · {{ installation.packageKind }}</strong>
          <code>{{ installation.packageName }}</code>
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
import { computed, nextTick, onMounted, ref, watch, type ComponentPublicInstance } from "vue";
import { useDebounceFn } from "@vueuse/core";
import { useVirtualizer } from "@tanstack/vue-virtual";
import { useRouter } from "vue-router";
import AppEmptyState from "../components/AppEmptyState.vue";
import AppLoading from "../components/AppLoading.vue";
import ToolCard from "../components/ToolCard.vue";
import { useRunsStore } from "../stores/runs";
import { useSnapshotStore } from "../stores/snapshot";
import type { ToolRecord } from "../utils/presentation";
import { formatDate, updateableComponents } from "../utils/presentation";

const store = useSnapshotStore();
const runs = useRunsStore();
const router = useRouter();
const searchInput = ref("");
const search = ref("");
const statusFilter = ref("all");
const providerFilter = ref("all");
const categoryFilter = ref("all");
const includeHidden = ref(false);
const selected = ref<Set<string>>(new Set());
const strategies = ref<Record<string, string>>({});
const detailRecord = ref<ToolRecord | null>(null);
const detailClose = ref<HTMLButtonElement | null>(null);
const detailTrigger = ref<HTMLElement | null>(null);
const scrollElement = ref<HTMLElement | null>(null);

const updateSearch = useDebounceFn((value: string) => {
  search.value = value.trim().toLocaleLowerCase();
}, 120);

watch(searchInput, (value) => void updateSearch(value));

const providers = computed(() => [
  ...new Set(store.toolRecords.flatMap((record) => record.providerIds)),
]);
const categories = computed(() => [
  ...new Set(store.toolRecords.flatMap((record) => record.tool.categories)),
]);
const providerErrors = computed(
  () => store.snapshot?.providers.filter((provider) => provider.errors.length) ?? [],
);
const lastChecked = computed(() => formatDate(store.snapshot?.refreshedAt));
const refreshProgressTotal = computed(() => store.currentRefreshProgress?.total ?? 0);
const refreshProgressCompleted = computed(
  () => store.currentRefreshProgress?.completed ?? 0,
);
const refreshProgressPosition = computed(() =>
  refreshProgressTotal.value
    ? Math.min(refreshProgressCompleted.value + 1, refreshProgressTotal.value)
    : 0,
);
const refreshProgressTitle = computed(() =>
  store.currentRefreshProgress?.providerId
    ? `正在检查 ${store.currentRefreshProgress.providerId}`
    : "正在准备刷新",
);
const refreshProgressDetail = computed(() =>
  refreshProgressTotal.value
    ? `正在处理第 ${refreshProgressPosition.value} 个，共 ${refreshProgressTotal.value} 个 Provider`
    : "正在连接后端并准备检查 Provider…",
);
const refreshProgressBarStyle = computed(() => {
  if (!refreshProgressTotal.value) return undefined;
  const percent = Math.round(
    (refreshProgressCompleted.value / refreshProgressTotal.value) * 100,
  );
  return { width: `${Math.max(6, percent)}%` };
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
    if (statusFilter.value === "updates" && record.tool.status.hasUpdates !== true) return false;
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
  }),
);

const virtualizer = useVirtualizer(
  computed(() => ({
    count: filteredRecords.value.length,
    getScrollElement: () => scrollElement.value,
    estimateSize: () => 210,
    overscan: 5,
    initialRect: { width: 900, height: 720 },
  })),
);
const virtualRows = computed(() => virtualizer.value.getVirtualItems());

function measureVirtualRow(element: Element | ComponentPublicInstance | null): void {
  if (element instanceof Element) virtualizer.value.measureElement(element);
}

function key(record: ToolRecord, componentId: string): string {
  return `${record.tool.id}\u0000${componentId}`;
}

function toggleComponent(record: ToolRecord, componentId: string): void {
  const next = new Set(selected.value);
  const value = key(record, componentId);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  selected.value = next;
}

function selectToolUpdates(record: ToolRecord): void {
  const next = new Set(selected.value);
  for (const component of updateableComponents(record.tool)) {
    next.add(key(record, component.id));
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

async function previewSelected(): Promise<void> {
  const selections = store.toolRecords.flatMap((record) =>
    record.tool.components.flatMap((component) =>
      selected.value.has(key(record, component.id))
        ? [
            {
              toolId: record.tool.id,
              componentId: component.id,
              strategyId: strategyFor(record, component.id) || null,
              targetVersion: null,
            },
          ]
        : [],
    ),
  );
  await runs.preview(selections);
  await router.push({ name: "confirm" });
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
  if (!report.status?.available) return report.status?.detail ?? "当前不可用";
  return `${report.status.version ?? "版本未知"} · ${formatDate(report.refreshedAt)}`;
}

function clearFilters(): void {
  searchInput.value = "";
  search.value = "";
  statusFilter.value = "all";
  providerFilter.value = "all";
  categoryFilter.value = "all";
  includeHidden.value = true;
}

function initialize(): void {
  void store.initialize().catch(() => undefined);
}
function refreshAll(): void {
  void store.refresh({ scope: "all" }).catch(() => undefined);
}
function refreshProvider(providerId: string): void {
  void store.refresh({ scope: "provider", providerId }).catch(() => undefined);
}
function refreshTool(toolId: string): void {
  void store.refresh({ scope: "tool", toolId }).catch(() => undefined);
}

onMounted(initialize);
</script>
