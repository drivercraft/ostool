import { defineStore } from "pinia";
import { ref } from "vue";

export const useUiStore = defineStore("ui", () => {
  const title = ref("总览");
  const successMessage = ref("");
  const errorMessage = ref("");

  function setTitle(value: string) {
    title.value = value;
  }

  function setSuccess(value: string) {
    successMessage.value = value;
    errorMessage.value = "";
  }

  function setError(value: string) {
    errorMessage.value = value;
    successMessage.value = "";
  }

  function clearMessages() {
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
