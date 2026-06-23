import { defineStore } from "pinia";
import { ref } from "vue";

const SUCCESS_AUTO_DISMISS_MS = 4000;
const ERROR_AUTO_DISMISS_MS = 8000;

export type UiDialogTone = "success" | "error" | "info" | "danger";

export interface UiDialogState {
  tone: UiDialogTone;
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
}

export interface UiConfirmOptions {
  title: string;
  message: string;
  tone?: UiDialogTone;
  confirmLabel?: string;
  cancelLabel?: string;
}

export const useUiStore = defineStore("ui", () => {
  const title = ref("总览");
  const successMessage = ref("");
  const errorMessage = ref("");
  const dialog = ref<UiDialogState | null>(null);
  let dismissTimer: ReturnType<typeof setTimeout> | null = null;
  let confirmResolver: ((value: boolean) => void) | null = null;

  function clearTimer() {
    if (dismissTimer !== null) {
      clearTimeout(dismissTimer);
      dismissTimer = null;
    }
  }

  function scheduleDismiss(ms: number) {
    clearTimer();
    dismissTimer = setTimeout(() => {
      successMessage.value = "";
      errorMessage.value = "";
      if (!confirmResolver) {
        dialog.value = null;
      }
      dismissTimer = null;
    }, ms);
  }

  function resolvePendingConfirm(value: boolean) {
    if (confirmResolver) {
      confirmResolver(value);
      confirmResolver = null;
    }
  }

  function setTitle(value: string) {
    title.value = value;
  }

  function setSuccess(value: string) {
    resolvePendingConfirm(false);
    successMessage.value = value;
    errorMessage.value = "";
    dialog.value = {
      tone: "success",
      title: "操作成功",
      message: value,
      confirmLabel: "确定",
    };
    scheduleDismiss(SUCCESS_AUTO_DISMISS_MS);
  }

  function setError(value: string) {
    resolvePendingConfirm(false);
    errorMessage.value = value;
    successMessage.value = "";
    dialog.value = {
      tone: "error",
      title: "出现错误",
      message: value,
      confirmLabel: "确定",
    };
    scheduleDismiss(ERROR_AUTO_DISMISS_MS);
  }

  function confirm(options: UiConfirmOptions): Promise<boolean> {
    clearTimer();
    resolvePendingConfirm(false);
    successMessage.value = "";
    errorMessage.value = "";
    dialog.value = {
      tone: options.tone ?? "danger",
      title: options.title,
      message: options.message,
      confirmLabel: options.confirmLabel ?? "确定",
      cancelLabel: options.cancelLabel ?? "取消",
    };
    return new Promise((resolve) => {
      confirmResolver = resolve;
    });
  }

  function closeDialog(accepted = false) {
    clearTimer();
    const hadConfirm = Boolean(confirmResolver);
    resolvePendingConfirm(accepted);
    dialog.value = null;
    if (!hadConfirm || !accepted) {
      successMessage.value = "";
      errorMessage.value = "";
    }
  }

  function clearMessages() {
    clearTimer();
    resolvePendingConfirm(false);
    successMessage.value = "";
    errorMessage.value = "";
    dialog.value = null;
  }

  return {
    title,
    successMessage,
    errorMessage,
    dialog,
    setTitle,
    setSuccess,
    setError,
    confirm,
    closeDialog,
    clearMessages,
  };
});
