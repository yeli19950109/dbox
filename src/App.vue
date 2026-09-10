<template>
  <div class="app-shell">
    <header class="topbar">
      <RouterLink class="brand" to="/tools" aria-label="dbox 工具主页">
        <span class="brand__mark" aria-hidden="true">d</span>
        <span>
          <strong>dbox</strong>
          <small>tool control</small>
        </span>
      </RouterLink>
      <nav aria-label="主导航">
        <RouterLink to="/tools">
          <span aria-hidden="true">⌁</span>
          工具
        </RouterLink>
        <RouterLink to="/runs">
          <span aria-hidden="true">↗</span>
          运行记录
        </RouterLink>
        <RouterLink to="/settings">
          <span aria-hidden="true">⌘</span>
          设置
        </RouterLink>
        <RouterLink to="/skills">
          <span aria-hidden="true">✦</span>
          Skills

        </RouterLink>
        <RouterLink to="/mcp"><span aria-hidden="true">⊞</span>MCP</RouterLink>
      </nav>
      <div class="topbar-actions">
        <RefreshMenu />
        <div class="connection-status">
          <span class="connection-dot" :data-connected="Boolean(snapshot.snapshot)" aria-hidden="true" />
          <span>{{ snapshot.snapshot ? "后端已连接" : "等待后端" }}</span>
        </div>
      </div>
    </header>
    <main id="main-content" class="main-content">
      <AppErrorBoundary>
        <RouterView />
      </AppErrorBoundary>
    </main>
    <NotificationCenter />
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import AppErrorBoundary from "./components/AppErrorBoundary.vue";
import NotificationCenter from "./components/NotificationCenter.vue";
import RefreshMenu from "./components/RefreshMenu.vue";
import { useRunsStore } from "./stores/runs";
import { useSnapshotStore } from "./stores/snapshot";

const snapshot = useSnapshotStore();
const runs = useRunsStore();

onMounted(() => {
  void snapshot.connectEvents().catch(() => undefined);
  void runs.connectEvents().catch(() => undefined);
  void snapshot.initialize().catch(() => undefined);
});

onBeforeUnmount(() => {
  snapshot.disconnectEvents();
  runs.disconnectEvents();
});
</script>
