import { ref } from "vue";
import { defineStore } from "pinia";
import type {
  ManifestDocumentDto,
  ManifestInputDto,
  ManifestValidationDto,
  SaveManifestRequestDto,
  SettingsDocumentDto,
  SettingsValueDto,
} from "../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";
import { useSnapshotStore } from "./snapshot";

export const useSettingsStore = defineStore("settings", () => {
  const document = ref<SettingsDocumentDto | null>(null);
  const manifest = ref<ManifestDocumentDto | null>(null);
  const validation = ref<ManifestValidationDto | null>(null);
  const loading = ref(false);
  const saving = ref(false);
  const error = ref<string | null>(null);

  async function load(): Promise<SettingsDocumentDto> {
    loading.value = true;
    error.value = null;
    try {
      const next = await unwrapCommand(getTransport().commands.settings());
      document.value = next;
      return next;
    } catch (reason) {
      error.value = errorMessage(reason, "无法连接 dbox 后端并读取设置。");
      throw reason;
    } finally {
      loading.value = false;
    }
  }

  async function save(settings: SettingsValueDto): Promise<SettingsDocumentDto> {
    if (!document.value) throw new Error("设置尚未加载");
    saving.value = true;
    error.value = null;
    try {
      const next = await unwrapCommand(
        getTransport().commands.saveSettings({
          expectedRevision: document.value.revision,
          settings,
        }),
      );
      document.value = next;
      return next;
    } catch (reason) {
      error.value = errorMessage(reason, "后端暂时不可用，设置未保存。");
      throw reason;
    } finally {
      saving.value = false;
    }
  }

  async function readManifest(fileName: string): Promise<ManifestDocumentDto> {
    const next = await unwrapCommand(
      getTransport().commands.readManifest({ fileName }),
    );
    manifest.value = next;
    validation.value = null;
    return next;
  }

  async function validateManifest(
    input: ManifestInputDto,
  ): Promise<ManifestValidationDto> {
    const result = await unwrapCommand(
      getTransport().commands.validateManifest(input),
    );
    validation.value = result;
    return result;
  }

  async function saveManifest(request: SaveManifestRequestDto): Promise<void> {
    const saved = await unwrapCommand(
      getTransport().commands.saveManifest(request),
    );
    manifest.value = {
      fileName: saved.validation.fileName,
      revision: saved.revision,
      contents: saved.validation.normalizedToml,
    };
    validation.value = saved.validation;
    useSnapshotStore().acceptSnapshot(saved.snapshot);
  }

  return {
    document,
    manifest,
    validation,
    loading,
    saving,
    error,
    load,
    save,
    readManifest,
    validateManifest,
    saveManifest,
  };
});
