<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useLlamaStore } from "../stores/llama";
import { formatDate } from "../utils/presentation";
import LlamaCommand from "../components/LlamaCommand.vue";

const store = useLlamaStore();
const tab = ref<"installed" | "releases">("installed");
const confirming = ref("");
const exactTag = ref("");
const releaseList = ref<HTMLElement | null>(null);
const installed = computed(() => new Set(store.inventory?.versions.map(version => version.tag)));
const exactTagValid = computed(() =>
  /^b[0-9]{1,10}$/.test(exactTag.value) && Number(exactTag.value.slice(1)) <= 0xffff_ffff,
);
const canDownload = computed(() => !store.busy && !!store.inventory);

function openTab(next: typeof tab.value) {
  tab.value = next;
  confirming.value = "";
  if (next === "releases" && !store.remote) void store.browse();
}

function remove(tag: string) {
  confirming.value = "";
  void store.changeVersion("remove", tag);
}

watch(() => store.page, () => {
  if (releaseList.value) releaseList.value.scrollTop = 0;
});
onMounted(store.refresh);
</script>

<template>
  <div class="view extension-view llama-view">
    <header class="extension-heading">
      <div>
        <h1>llama.cpp</h1>
        <p class="muted">版本目录 <code>{{ store.inventory?.root || "~/.llama-app" }}</code></p>
      </div>
      <button class="button secondary" :disabled="store.busy" @click="store.refresh">
        {{ store.reading ? "读取中…" : "刷新本地版本" }}
      </button>
    </header>
    <div class="extension-content">
      <div class="llama-layout">
        <section class="extension-panel versions-panel" aria-labelledby="llama-versions-title">
          <h2 id="llama-versions-title">版本管理</h2>
          <nav class="extension-tabs" aria-label="版本来源">
            <button :class="{ active: tab === 'installed' }" :aria-pressed="tab === 'installed'" @click="openTab('installed')">
              已安装{{ store.inventory ? ' · ' + store.inventory.versions.length : '' }}
            </button>
            <button :class="{ active: tab === 'releases' }" :aria-pressed="tab === 'releases'" @click="openTab('releases')">下载版本</button>
          </nav>

          <p v-if="store.error" class="inline-error" role="alert">{{ store.error }}</p>
          <p v-if="store.pending" class="muted" role="status">
            正在{{ store.pending.action === "install" ? "下载安装" : "移除" }} {{ store.pending.tag }}…
          </p>
          <p v-else-if="store.status" class="muted" role="status">{{ store.status }}</p>

          <template v-if="tab === 'installed'">
            <p v-if="store.reading && !store.inventory" class="muted">正在读取版本目录…</p>
            <p v-else-if="store.inventory && !store.inventory.versions.length" class="muted">尚未安装版本。</p>
            <div v-else class="version-list" role="group" aria-label="选择用于运行命令的版本">
              <div v-for="version in store.inventory?.versions" :key="version.tag" class="version-item">
                <div class="version-row">
                  <label class="version-choice">
                    <input v-model="store.selectedTag" type="radio" name="llama-version" :value="version.tag" :aria-label="'选择 ' + version.tag" />
                    <span class="version-detail">
                      <strong>{{ version.tag }}</strong>
                      <code :title="version.serverPath">{{ version.serverPath.slice(version.directory.length + 1) }}</code>
                    </span>
                  </label>
                  <button class="text-button remove-button" :aria-label="'移除 ' + version.tag" :disabled="store.busy" @click="confirming = version.tag">移除</button>
                </div>
                <div v-if="confirming === version.tag" class="remove-confirm">
                  <p>移除 {{ version.tag }} 将删除整个目录 <code>{{ version.directory }}</code>。</p>
                  <div class="extension-actions">
                    <button class="button secondary compact" @click="confirming = ''">取消</button>
                    <button class="button danger compact" :disabled="store.busy" @click="remove(version.tag)">确认移除</button>
                  </div>
                </div>
              </div>
            </div>
          </template>

          <template v-else>
            <form class="exact-download" @submit.prevent="exactTagValid && !installed.has(exactTag) && store.changeVersion('install', exactTag)">
              <label for="llama-download-tag">按版本号下载</label>
              <div class="download-controls">
                <input id="llama-download-tag" v-model.trim="exactTag" type="text" placeholder="例如 b12644" spellcheck="false"
                  :aria-invalid="!!exactTag && !exactTagValid" :aria-describedby="exactTag && !exactTagValid ? 'tag-format' : undefined" />
                <button class="button secondary compact" :disabled="!canDownload || !exactTagValid || installed.has(exactTag)">
                  {{ installed.has(exactTag) ? "已安装" : "下载" }}
                </button>
              </div>
              <small v-if="exactTag && !exactTagValid" id="tag-format">请输入以 b 开头的有效构建编号，例如 b12644。</small>
            </form>

            <div class="extension-heading">
              <h3>GitHub 发布版本</h3>
              <button class="text-button" :disabled="store.fetching" @click="store.browse(1)">刷新列表</button>
            </div>
            <p v-if="store.remoteError" class="inline-error" role="alert">{{ store.remoteError }}</p>
            <p v-if="store.fetching" class="muted" role="status">正在读取发布版本…</p>
            <template v-if="store.remote">
              <p v-if="!store.remote.releases.length" class="muted">本页没有可显示的版本。</p>
              <div v-else ref="releaseList" class="version-list remote-list" aria-label="发布版本">
                <div v-for="release in store.remote.releases" :key="release.tag" class="version-row">
                  <span class="version-detail">
                    <strong>{{ release.tag }}</strong>
                    <small>{{ formatDate(release.publishedAt) }}</small>
                    <code v-if="release.assetName">{{ release.assetName }}</code>
                  </span>
                  <button class="button secondary compact" :disabled="!release.assetName || !canDownload || installed.has(release.tag)"
                    :aria-label="'下载 ' + release.tag" @click="store.changeVersion('install', release.tag)">
                    {{ installed.has(release.tag) ? "已安装" : !release.assetName ? "无适用构建" : "下载" }}
                  </button>
                </div>
              </div>
              <nav class="pagination" aria-label="发布版本分页">
                <button class="text-button" :disabled="store.fetching || store.page === 1" @click="store.browse(store.page - 1)">上一页</button>
                <span>第 {{ store.page }} 页</span>
                <button class="text-button" :disabled="store.fetching || !store.remote.hasMore" @click="store.browse(store.page + 1)">下一页</button>
              </nav>
            </template>
          </template>
        </section>
        <LlamaCommand :version="store.selected" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.llama-layout {
  display: grid;
  grid-template-columns: minmax(285px, 0.8fr) minmax(0, 1.2fr);
  align-items: start;
  gap: 18px;
}
.versions-panel { min-width: 0; }
.versions-panel h2, .versions-panel h3 { margin: 0; }
.version-list { max-height: 48vh; overflow: auto; }
.version-item + .version-item, .remote-list .version-row + .version-row { border-top: 1px solid var(--line); }
.version-row, .download-controls, .pagination { display: flex; gap: 12px; align-items: center; justify-content: space-between; }
.version-row { padding: 12px 2px; }
.version-row .version-choice { flex: 1; min-width: 0; flex-direction: row; align-items: center; gap: 10px; cursor: pointer; }
.version-choice input[type="radio"] { flex-shrink: 0; margin: 0; accent-color: var(--accent); }
.version-detail { min-width: 0; display: grid; gap: 3px; }
.version-detail strong { color: var(--text); font-size: 14px; }
.version-detail code { color: var(--muted); }
.version-row .button { flex-shrink: 0; }
.remove-button { color: var(--danger); }
.remove-confirm { display: grid; gap: 8px; padding: 12px; border: 1px solid var(--danger); border-radius: 8px; font-size: 13px; }
.exact-download { display: grid; gap: 7px; padding-bottom: 16px; border-bottom: 1px solid var(--line); }
.download-controls input { flex: 1; font-size: 13px; }
.pagination { font-size: 12px; color: var(--muted); }
.text-button:disabled { opacity: 0.5; cursor: default; }
@media (max-width: 860px) {
  .llama-layout { grid-template-columns: 1fr; }
}
</style>
