import { computed, onMounted, ref } from "vue";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";

// Reopening a profile or deleting its credential must wait for an in-flight save.
const pendingWrites = new Map<string, Promise<void>>();

async function waitForWrite(profileId: string) {
  // A failed write is reported to its caller. The next request can read or retry.
  await pendingWrites.get(profileId)?.catch(() => {});
}

async function writeKey(profileId: string, value: string) {
  const commands = getTransport().commands;
  const write = waitForWrite(profileId).then(async () => {
    await unwrapCommand(commands.llamaSaveApiKey(value, profileId));
  });
  pendingWrites.set(profileId, write);
  try {
    await write;
  } finally {
    if (pendingWrites.get(profileId) === write) pendingWrites.delete(profileId);
  }
}

export function clearLlamaProfileApiKey(profileId: string) {
  return writeKey(profileId, "");
}

export function useLlamaApiKey(profileId: string) {
  const saved = ref<string>();
  const draft = ref("");
  const edited = ref(false);
  const reading = ref(false);
  const saving = ref(false);
  const readError = ref("");
  const saveError = ref("");
  const savedHere = ref(false);

  const key = computed({
    get: () => draft.value,
    set(value: string) {
      draft.value = value;
      edited.value = true;
      saveError.value = "";
    },
  });
  const ready = computed(() => saved.value !== undefined || edited.value);
  const dirty = computed(() => edited.value && draft.value !== (saved.value ?? ""));
  const canSave = computed(() => dirty.value && !reading.value && !saving.value);
  const status = computed(() => {
    if (reading.value) return "正在读取已保存的密钥…";
    if (saving.value) return "正在保存密钥…";
    if (dirty.value) return "密钥已修改，尚未保存。";
    if (saved.value) return "密钥已保存。";
    if (savedHere.value) return "已清除保存的密钥。";
    if (readError.value && edited.value) return "本次命令将省略 API Key。";
    return "";
  });

  async function read() {
    if (reading.value || saving.value || saved.value !== undefined) return;
    reading.value = true;
    readError.value = "";
    try {
      await waitForWrite(profileId);
      saved.value = (await unwrapCommand(getTransport().commands.llamaReadApiKey(profileId))) ?? "";
      if (!edited.value) draft.value = saved.value;
    } catch (reason) {
      readError.value = errorMessage(reason, "无法读取已保存的密钥。");
    } finally {
      reading.value = false;
    }
  }

  async function save() {
    if (!canSave.value) return;
    const value = draft.value;
    saving.value = true;
    saveError.value = "";
    try {
      await writeKey(profileId, value);
      saved.value = value;
      savedHere.value = true;
      readError.value = "";
    } catch (reason) {
      saveError.value = errorMessage(reason, "无法保存密钥。");
    } finally {
      saving.value = false;
    }
  }

  onMounted(read);
  return { key, ready, canSave, readError, saveError, status, read, save };
}
