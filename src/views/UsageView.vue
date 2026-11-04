<template>
  <div class="view extension-view usage-view">
    <header class="extension-heading">
      <div>
        <p class="eyebrow">Codex · 本地会话日志</p>
        <h1>用量统计</h1>
        <p>从 Codex JSONL 会话日志逐请求统计 token，模型名已归一化。</p>
      </div>
      <div class="extension-actions">
        <button
          class="button secondary"
          :disabled="store.loading"
          @click="store.load()"
        >
          {{ store.loading ? "统计中…" : "刷新" }}
        </button>
      </div>
    </header>
    <nav class="extension-tabs" aria-label="统计时间范围">
      <button
        v-for="window in USAGE_WINDOWS"
        :key="String(window)"
        :class="{ active: store.days === window }"
        :aria-pressed="store.days === window"
        @click="store.load(window)"
      >
        {{ window === null ? "全部" : `近 ${window} 天` }}
      </button>
    </nav>

    <div class="extension-content">
      <p v-if="store.error" class="inline-error" role="alert">{{ store.error }}</p>

      <section v-if="report && !report.available" class="extension-panel">
        <h2>未找到 Codex 目录</h2>
        <p class="muted">
          {{ report.codexHome }} 不存在。安装并使用 Codex 后会在这里显示用量；
          可通过 CODEX_HOME 环境变量指定其他位置。
        </p>
      </section>

      <template v-if="report && report.available">
        <section class="usage-stats" aria-label="用量汇总">
          <div class="usage-stat">
            <span>请求数</span>
            <strong data-testid="usage-requests">{{ formatCount(report.totals.requests) }}</strong>
            <small>{{ formatCount(report.sessions) }} 个会话</small>
          </div>
          <div class="usage-stat">
            <span>总 token</span>
            <strong data-testid="usage-total">{{ formatTokens(report.totals.totalTokens) }}</strong>
            <small>输入 + 输出</small>
          </div>
          <div class="usage-stat">
            <span>输入</span>
            <strong>{{ formatTokens(report.totals.inputTokens) }}</strong>
            <small>缓存命中 {{ cacheRate(report.totals) }}</small>
          </div>
          <div class="usage-stat">
            <span>输出</span>
            <strong>{{ formatTokens(report.totals.outputTokens) }}</strong>
            <small>含推理 token</small>
          </div>
        </section>

        <section class="extension-panel">
          <h2>按模型</h2>
          <p v-if="report.models.length === 0" class="muted">该时间范围内没有用量。</p>
          <div v-else class="usage-table-wrap">
            <table class="usage-table">
              <thead>
                <tr>
                  <th scope="col">模型</th>
                  <th scope="col">请求</th>
                  <th scope="col">输入</th>
                  <th scope="col">缓存命中</th>
                  <th scope="col">输出</th>
                  <th scope="col">总计</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="row in report.models" :key="row.model">
                  <th scope="row"><code>{{ row.model }}</code></th>
                  <td>{{ formatCount(row.usage.requests) }}</td>
                  <td>{{ formatTokens(row.usage.inputTokens) }}</td>
                  <td>{{ formatTokens(row.usage.cachedInputTokens) }}</td>
                  <td>{{ formatTokens(row.usage.outputTokens) }}</td>
                  <td>{{ formatTokens(row.usage.totalTokens) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>

        <section v-if="report.daily.length" class="extension-panel">
          <h2>按天</h2>
          <ol class="usage-daily">
            <li v-for="day in dailyRows" :key="day.date">
              <span class="usage-daily__date">{{ day.date }}</span>
              <span class="usage-daily__bar" aria-hidden="true">
                <span :style="{ width: `${day.share}%` }" />
              </span>
              <span class="usage-daily__value">
                {{ formatTokens(day.usage.totalTokens) }} · {{ formatCount(day.usage.requests) }} 次
              </span>
            </li>
          </ol>
        </section>

        <details v-if="report.deferred.length" class="extension-panel">
          <summary>{{ report.deferred.length }} 个会话文件未计入</summary>
          <p class="muted">
            这些文件的用量无法可靠归属（例如子代理的父会话缺失），为避免重复计算暂不统计。
          </p>
          <ul class="usage-deferred">
            <li v-for="file in report.deferred" :key="file.path">
              <code>{{ file.path }}</code>
              <small>{{ file.reason }}</small>
            </li>
          </ul>
        </details>

        <p class="muted usage-footnote">
          扫描 {{ report.codexHome }} 下 {{ formatCount(report.filesScanned) }} 个会话文件；
          跳过子代理重放的父会话请求 {{ formatCount(report.replayedRequests) }} 次、
          重复副本 {{ formatCount(report.duplicateRequests) }} 次。
        </p>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted } from "vue";
import type { TokenUsageDto } from "../bindings";
import { USAGE_WINDOWS, useUsageStore } from "../stores/usage";

const store = useUsageStore();
const report = computed(() => store.report);

const countFormat = new Intl.NumberFormat("zh-CN");
const compactFormat = new Intl.NumberFormat("en-US", {
  notation: "compact",
  maximumFractionDigits: 1,
});

function formatCount(value: number): string {
  return countFormat.format(value);
}

function formatTokens(value: number): string {
  return value < 10_000 ? countFormat.format(value) : compactFormat.format(value);
}

function cacheRate(usage: TokenUsageDto): string {
  if (usage.inputTokens === 0) return "—";
  return `${Math.round((usage.cachedInputTokens / usage.inputTokens) * 100)}%`;
}

const dailyRows = computed(() => {
  const daily = report.value?.daily ?? [];
  const max = Math.max(1, ...daily.map((day) => day.usage.totalTokens));
  return [...daily]
    .reverse()
    .map((day) => ({ ...day, share: Math.max(1, (day.usage.totalTokens / max) * 100) }));
});

onMounted(() => {
  void store.load();
});
</script>

<style scoped>
.usage-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 12px;
}
.usage-stat {
  padding: 18px 20px;
  border: 1px solid var(--line);
  border-radius: 16px;
  background: var(--panel);
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.usage-stat span {
  color: var(--muted);
  font-size: 13px;
}
.usage-stat strong {
  font-size: 26px;
  letter-spacing: -0.4px;
  font-variant-numeric: tabular-nums;
}
.usage-table-wrap {
  overflow-x: auto;
}
.usage-table {
  width: 100%;
  border-collapse: collapse;
  font-variant-numeric: tabular-nums;
}
.usage-table th,
.usage-table td {
  padding: 8px 10px;
  border-bottom: 1px solid var(--line);
  text-align: right;
  white-space: nowrap;
}
.usage-table th:first-child {
  text-align: left;
}
.usage-table thead th {
  color: var(--muted);
  font-weight: 500;
  font-size: 13px;
}
.usage-daily {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.usage-daily li {
  display: grid;
  grid-template-columns: 96px 1fr auto;
  align-items: center;
  gap: 12px;
  font-variant-numeric: tabular-nums;
}
.usage-daily__bar {
  height: 8px;
  border-radius: 4px;
  background: var(--panel-raised);
  overflow: hidden;
}
.usage-daily__bar span {
  display: block;
  height: 100%;
  border-radius: 4px;
  background: var(--accent);
}
.usage-daily__date,
.usage-daily__value {
  color: var(--muted);
  font-size: 13px;
}
.usage-deferred {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.usage-deferred li {
  display: flex;
  flex-direction: column;
  gap: 2px;
  word-break: break-all;
}
.usage-footnote {
  font-size: 12px;
}
</style>
