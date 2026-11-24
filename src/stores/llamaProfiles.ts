import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { useLocalStorage } from "@vueuse/core";
import { clearLlamaProfileApiKey } from "./llamaApiKey";

const defaults = {
  model: "", host: "127.0.0.1", port: "8080", context: "", threads: "", gpuLayers: "",
};

export interface LlamaLaunchProfile {
  id: string;
  name: string;
  config: typeof defaults;
}

interface ProfileList {
  activeId: string;
  profiles: LlamaLaunchProfile[];
}

const storageKey = "dbox.llama.profiles";

function initialProfiles(): ProfileList {
  const config = { ...defaults };
  try {
    if (localStorage.getItem(storageKey) === null) {
      const previous = JSON.parse(localStorage.getItem("dbox.llama.command") ?? "null");
      for (const field of Object.keys(defaults) as (keyof typeof defaults)[]) {
        if (typeof previous?.[field] === "string") config[field] = previous[field];
      }
    }
  } catch {
    // Older or unreadable WebView data must not prevent opening the editor.
  }
  // The stable default ID reuses the v0.3.1 Keychain / IndexedDB credential.
  return { activeId: "default", profiles: [{ id: "default", name: "默认配置", config }] };
}

export const useLlamaProfiles = defineStore("llamaProfiles", () => {
  const state = useLocalStorage<ProfileList>(storageKey, initialProfiles(), { flush: "sync" });
  const profiles = computed(() => state.value.profiles);
  const activeProfile = computed(() => profiles.value.find(p => p.id === state.value.activeId) ?? profiles.value[0]!);
  const deletingId = ref<string | null>(null);

  function nameFor(input: string, currentId?: string) {
    const name = input.trim();
    if (!name) throw new Error("请输入配置名称。");
    if (profiles.value.some(p => p.id !== currentId && p.name.toLowerCase() === name.toLowerCase())) {
      throw new Error("配置名称已存在。");
    }
    return name;
  }

  function select(id: string) {
    if (!deletingId.value && profiles.value.some(p => p.id === id)) state.value.activeId = id;
  }

  function create(input: string) {
    if (deletingId.value) throw new Error("正在删除配置。");
    const name = nameFor(input);
    const profile = { id: crypto.randomUUID(), name, config: { ...defaults } };
    state.value = { activeId: profile.id, profiles: [...profiles.value, profile] };
  }

  function rename(id: string, input: string) {
    if (deletingId.value) throw new Error("正在删除配置。");
    const profile = profiles.value.find(p => p.id === id);
    if (!profile) throw new Error("找不到这份配置。");
    profile.name = nameFor(input, id);
  }

  async function remove(id: string) {
    if (deletingId.value) throw new Error("正在删除配置。");
    if (!profiles.value.some(p => p.id === id)) return;
    if (profiles.value.length === 1) throw new Error("至少保留一份启动配置。");
    deletingId.value = id;
    try {
      await clearLlamaProfileApiKey(id);
      const remaining = profiles.value.filter(p => p.id !== id);
      if (!remaining.length) throw new Error("其他配置已被删除，已保留当前配置。");
      state.value = {
        profiles: remaining,
        activeId: state.value.activeId === id ? remaining[0]!.id : state.value.activeId,
      };
    } finally {
      deletingId.value = null;
    }
  }

  return { profiles, activeProfile, deletingId, select, create, rename, remove };
});
