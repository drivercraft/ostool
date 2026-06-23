import { defineStore } from "pinia";
import { ref } from "vue";

const SUCCESS_AUTO_DISMISS_MS = 4000;
const ERROR_AUTO_DISMISS_MS = 8000;

export const useUiStore = defineStore("ui", () => {
  const title = ref("总览");
  const successMessage = ref("");
  const errorMessage = ref("");
  let dismissTimer: ReturnType<typeof setTimeout> | null = null;

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
      dismissTimer = null;
    }, ms);
  }

  function setTitle(value: string) {
    title.value = value;
  }

  function setSuccess(value: string) {
    successMessage.value = value;
    errorMessage.value = "";
    scheduleDismiss(SUCCESS_AUTO_DISMISS_MS);
  }

  function setError(value: string) {
    errorMessage.value = value;
    successMessage.value = "";
    scheduleDismiss(ERROR_AUTO_DISMISS_MS);
  }

  function clearMessages() {
    clearTimer();
    successMessage.value = "";
    errorMessage.value = "";
  }

  return {
    title,
    successMessage,
    errorMessage,
    setTitle,
    setSuccess,
    setError,
    clearMessages,
  };
});
