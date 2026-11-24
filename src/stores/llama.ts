import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { useLocalStorage } from "@vueuse/core";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";
import type { LlamaInventory, LlamaReleasePage } from "../bindings";

export const useLlamaStore = defineStore("llama", () => {
  const inventory = ref<LlamaInventory | null>(null);
  const selectedTag = useLocalStorage("dbox.llama.selectedVersion", "");
  const selected = computed(() => inventory.value?.versions.find(version => version.tag === selectedTag.value));
  const reading = ref(false);
  const error = ref("");
  const status = ref("");
  const pending = ref<{ action: "install" | "remove"; tag: string } | null>(null);
  const busy = computed(() => reading.value || !!pending.value);
  const remote = ref<LlamaReleasePage | null>(null);
  const page = ref(1);
  const fetching = ref(false);
  const remoteError = ref("");

  async function refresh() {
    if (busy.value) return;
    reading.value = true;
    error.value = "";
    try {
      inventory.value = await unwrapCommand(getTransport().commands.llamaInventory());
      if (!selected.value) selectedTag.value = inventory.value.versions[0]?.tag ?? "";
    } catch (reason) {
      error.value = errorMessage(reason, "无法读取已安装版本。");
    } finally {
      reading.value = false;
    }
  }

  async function browse(nextPage = 1) {
    if (fetching.value || nextPage < 1) return;
    fetching.value = true;
    remoteError.value = "";
    try {
      remote.value = await unwrapCommand(getTransport().commands.llamaReleases({ page: nextPage }));
      page.value = nextPage;
    } catch (reason) {
      remoteError.value = errorMessage(reason, "无法读取 GitHub 发布版本。");
    } finally {
      fetching.value = false;
    }
  }

  async function changeVersion(action: "install" | "remove", tag: string) {
    if (busy.value || !inventory.value) return;
    pending.value = { action, tag };
    error.value = "";
    status.value = "";
    try {
      const commands = getTransport().commands;
      if (action === "install") {
        const version = await unwrapCommand(commands.llamaInstall({ tag }));
        inventory.value.versions = [version, ...inventory.value.versions.filter(item => item.tag !== version.tag)];
        if (!selected.value) selectedTag.value = version.tag;
        status.value = `已安装 ${version.tag}。`;
      } else {
        await unwrapCommand(commands.llamaRemove({ tag }));
        inventory.value.versions = inventory.value.versions.filter(item => item.tag !== tag);
        if (!selected.value) selectedTag.value = inventory.value.versions[0]?.tag ?? "";
        status.value = `已移除 ${tag}。`;
      }
    } catch (reason) {
      error.value = errorMessage(reason, `无法${action === "install" ? "下载" : "移除"} ${tag}。`);
    } finally {
      pending.value = null;
    }
  }

  return {
    inventory, selectedTag, selected, reading, pending, busy, status, error,
    remote, page, fetching, remoteError, browse, refresh, changeVersion,
  };
});
