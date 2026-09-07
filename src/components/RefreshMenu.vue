<template>
  <div ref="root" class="refresh-control" @keydown.esc.stop.prevent="closeMenu()" @focusout="onFocusOut">
    <button
      type="button"
      class="topbar-refresh"
      :data-loading="store.loading || undefined"
      :disabled="store.loading"
      :aria-label="refreshLabel"
      :title="refreshLabel"
      @click="refresh({ scope: 'all' })"
    >
      <svg class="refresh-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M20 7v5h-5M4 17v-5h5" />
        <path d="M6.1 6.1A8 8 0 0 1 20 12M4 12a8 8 0 0 0 13.9 5.9" />
      </svg>
    </button>
    <button
      ref="trigger"
      type="button"
      class="refresh-menu-toggle"
      aria-label="选择刷新来源"
      title="选择刷新来源"
      aria-haspopup="menu"
      :aria-expanded="open"
      :aria-controls="menuId"
      :disabled="store.loading || !providers.length"
      @click="open ? closeMenu() : openMenu()"
      @keydown.down.prevent="openMenu()"
      @keydown.up.prevent="openMenu(true)"
    >
      <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="m4 6 4 4 4-4" />
      </svg>
    </button>
    <div v-if="open" :id="menuId" ref="menu" class="refresh-menu" role="menu" aria-label="按来源刷新" @keydown="navigateMenu">
      <button
        v-for="provider in providers"
        :key="provider"
        type="button"
        role="menuitem"
        tabindex="-1"
        :disabled="store.loading"
        @click="refresh({ scope: 'provider', providerId: provider })"
      >
        刷新 {{ providerLabel(provider) }}
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, useId } from "vue";
import { onClickOutside } from "@vueuse/core";
import type { RefreshScopeDto } from "../bindings";
import { useSnapshotStore } from "../stores/snapshot";
import { providerLabel } from "../utils/presentation";

const store = useSnapshotStore();
const root = ref<HTMLElement | null>(null);
const menu = ref<HTMLElement | null>(null);
const trigger = ref<HTMLButtonElement | null>(null);
const open = ref(false);
const menuId = useId();
const refreshLabel = computed(() => store.loading ? "正在刷新工具" : "刷新全部工具");
const providers = computed(() => [...store.providerIds].sort((a, b) => providerLabel(a).localeCompare(providerLabel(b))));

onClickOutside(root, () => closeMenu(false));

function closeMenu(restoreFocus = true): void {
  if (!open.value) return;
  open.value = false;
  if (restoreFocus) trigger.value?.focus();
}

async function openMenu(last = false): Promise<void> {
  if (store.loading || !providers.value.length) return;
  open.value = true;
  await nextTick();
  const items = menu.value?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]');
  items?.[last ? items.length - 1 : 0]?.focus();
}

function onFocusOut(event: FocusEvent): void {
  if (event.relatedTarget instanceof Node && !root.value?.contains(event.relatedTarget)) closeMenu(false);
}

function navigateMenu(event: KeyboardEvent): void {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const items = Array.from(menu.value?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
  if (!items.length) return;
  const current = items.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
    : (current + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
  items[next]?.focus();
}

function refresh(scope: RefreshScopeDto): void {
  if (store.loading) return;
  closeMenu();
  void store.refresh(scope).catch(() => undefined);
}
</script>
