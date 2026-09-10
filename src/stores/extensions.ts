import { defineStore } from "pinia";
import { ref } from "vue";
import type { ExtensionPlan, ExtensionResult, ResourceKind } from "../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";
import { useRunsStore } from "./runs";

export const useExtensionOperations = defineStore(
  "extension-operations",
  () => {
    const plan = ref<ExtensionPlan | null>(null);
    const resource = ref<ResourceKind | null>(null);
    const result = ref<ExtensionResult | null>(null);
    const error = ref<string | null>(null);
    const pending = ref(false);
    const runId = ref<string | null>(null);
    const preparing = ref(false);
    let poll: ReturnType<typeof setTimeout> | undefined;
    async function preview(
      request: () => Promise<ExtensionPlan>,
      kind?: ResourceKind,
    ) {
      if (pending.value || preparing.value) return;
      resource.value = kind ?? null;
      error.value = null;
      result.value = null;
      preparing.value = true;
      plan.value = null;
      try {
        plan.value = await request();
        resource.value = plan.value.resource;
      } catch (e) {
        error.value = errorMessage(e, "预览失败");
      } finally {
        preparing.value = false;
      }
    }
    async function refreshResult() {
      if (!runId.value) return;
      try {
        const next = await unwrapCommand(
          getTransport().commands.extensionOperationResult({ id: runId.value }),
        );
        result.value = next;
        if (!["queued", "running"].includes(next.status)) {
          pending.value = false;
          await useRunsStore()
            .loadHistory()
            .catch(() => undefined);
          return;
        }
      } catch (e) {
        error.value = errorMessage(e, "读取运行结果失败，将重试");
      }
      if (pending.value) poll = setTimeout(() => void refreshResult(), 1000);
    }
    async function confirm() {
      if (!plan.value || pending.value) return;
      pending.value = true;
      error.value = null;
      try {
        await useRunsStore().connectEvents();
        const started = await unwrapCommand(
          getTransport().commands.confirmExtensionOperation({
            planId: plan.value.planId,
            planHash: plan.value.planHash,
          }),
        );
        runId.value = started.runId;
        plan.value = null;
        clearTimeout(poll);
        void refreshResult();
      } catch (e) {
        pending.value = false;
        error.value = errorMessage(e, "执行失败，请重新预览");
        plan.value = null;
      }
    }
    async function cancel() {
      if (!runId.value) return;
      try {
        await unwrapCommand(
          getTransport().commands.cancel({ runId: runId.value }),
        );
      } catch (e) {
        error.value = errorMessage(e, "取消失败");
      }
    }
    return {
      resource,
      plan,
      result,
      pending,
      preparing,
      runId,
      error,
      preview,
      confirm,
      cancel,
    };
  },
);
