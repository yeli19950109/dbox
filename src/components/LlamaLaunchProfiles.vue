<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import type { LlamaInstalledVersion } from "../bindings";
import { errorMessage } from "../api/transport";
import { useLlamaProfiles } from "../stores/llamaProfiles";
import { useNotificationsStore } from "../stores/notifications";
import LlamaCommand from "./LlamaCommand.vue";

defineProps<{ version?: LlamaInstalledVersion }>();
const store = useLlamaProfiles();
const { profiles, activeProfile, deletingId } = storeToRefs(store);
const notifications = useNotificationsStore();
const action = ref<"create" | "rename" | "delete" | null>(null);
const name = ref("");
const nameInput = ref<HTMLInputElement | null>(null);
const error = ref("");
let disposed = false;
onBeforeUnmount(() => { disposed = true; });

function cancel() {
  action.value = null;
  error.value = "";
}

watch(() => activeProfile.value.id, cancel);

async function begin(next: "create" | "rename" | "delete") {
  if (deletingId.value) return;
  action.value = next;
  name.value = next === "rename" ? activeProfile.value.name : "";
  error.value = "";
  if (next !== "delete") {
    await nextTick();
    nameInput.value?.focus();
    nameInput.value?.select();
  }
}

function saveName() {
  if (deletingId.value) return;
  try {
    if (action.value === "create") store.create(name.value);
    else if (action.value === "rename") store.rename(activeProfile.value.id, name.value);
    cancel();
  } catch (reason) {
    error.value = errorMessage(reason, "无法保存配置名称。");
  }
}

async function remove() {
  if (action.value !== "delete" || deletingId.value || profiles.value.length < 2) return;
  const { id, name } = activeProfile.value;
  error.value = "";
  try {
    await store.remove(id);
    cancel();
  } catch (reason) {
    const message = errorMessage(reason, "无法移除配置及其密钥。");
    if (disposed) notifications.push("error", `删除配置「${name}」失败`, message);
    else error.value = `删除配置「${name}」失败。${message}`;
  }
}
</script>

<template>
  <section class="extension-panel profiles-panel" aria-labelledby="llama-config-title">
    <header class="extension-heading">
      <h2 id="llama-config-title">启动配置</h2>
      <small class="muted">所有版本共用</small>
    </header>
    <fieldset class="extension-fieldset profile-controls" :disabled="!!deletingId">
      <legend class="sr-only">配置方案管理</legend>
      <div class="profile-row">
        <label for="llama-profile" class="profile-selector">启动配置方案
          <select id="llama-profile" :value="activeProfile.id" @change="store.select(($event.target as HTMLSelectElement).value)">
            <option v-for="profile in profiles" :key="profile.id" :value="profile.id">{{ profile.name }}</option>
          </select>
        </label>
        <div class="profile-actions">
          <button type="button" class="button secondary compact" @click="begin('create')">新建配置</button>
          <button type="button" class="text-button" @click="begin('rename')">重命名</button>
          <button type="button" class="text-button remove-button" :disabled="profiles.length === 1"
            :title="profiles.length === 1 ? '至少保留一个配置' : undefined" @click="begin('delete')">删除配置</button>
        </div>
      </div>

      <form v-if="action === 'create' || action === 'rename'" class="profile-form" @submit.prevent="saveName" @keydown.esc.prevent="cancel">
        <label for="llama-profile-name">{{ action === "create" ? "新配置名称" : "配置名称" }}
          <input id="llama-profile-name" ref="nameInput" v-model="name" type="text" autocomplete="off" />
        </label>
        <div class="profile-actions">
          <button class="button primary compact">{{ action === "create" ? "创建配置" : "保存名称" }}</button>
          <button type="button" class="button secondary compact" @click="cancel">取消</button>
        </div>
      </form>

      <div v-if="action === 'delete'" class="delete-confirm" role="group" aria-label="确认删除配置">
        <p>删除「{{ activeProfile.name }}」及其保存的 API Key？</p>
        <div class="profile-actions">
          <button type="button" class="button secondary compact" @click="cancel">取消</button>
          <button type="button" class="button danger compact" @click="remove">确认删除</button>
        </div>
      </div>
    </fieldset>
    <p v-if="deletingId" class="muted" role="status">正在删除配置及其密钥…</p>
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>

    <LlamaCommand :key="activeProfile.id" :profile="activeProfile" :version="version" :disabled="!!deletingId" />
  </section>
</template>

<style scoped>
.profiles-panel { min-width: 0; gap: 16px; }
.profile-controls { border-bottom: 1px solid var(--line); padding-bottom: 16px; }
.profile-row, .profile-form, .profile-actions { display: flex; align-items: flex-end; flex-wrap: wrap; gap: 10px; }
.profile-selector, .profile-form > label { flex: 1 1 150px; min-width: 0; }
.profile-actions { align-items: center; }
.remove-button { color: var(--danger); }
.text-button:disabled { opacity: 0.5; cursor: default; }
.profile-form, .delete-confirm { padding: 12px; border-radius: 8px; background: var(--bg); }
.delete-confirm { display: grid; gap: 8px; border: 1px solid var(--danger); font-size: 13px; }
</style>
