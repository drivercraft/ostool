<script setup lang="ts">
import { computed, ref } from "vue";

import Icon, { type IconName } from "@/components/Icon.vue";
import type { UiDialogState } from "@/stores/ui";

const props = defineProps<{
  dialog: UiDialogState;
}>();

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();

const pointerDownOnOverlay = ref(false);

const iconName = computed<IconName>(() => {
  if (props.dialog.tone === "success") {
    return "check";
  }
  if (props.dialog.tone === "error" || props.dialog.tone === "danger") {
    return "ban";
  }
  return "bell";
});

function onOverlayPointerDown(event: PointerEvent) {
  pointerDownOnOverlay.value = event.target === event.currentTarget;
}

function onOverlayClick(event: MouseEvent) {
  if (pointerDownOnOverlay.value && event.target === event.currentTarget) {
    emit("cancel");
  }
  pointerDownOnOverlay.value = false;
}
</script>

<template>
  <div
    class="modal-overlay app-dialog-overlay"
    @pointerdown="onOverlayPointerDown"
    @click="onOverlayClick"
  >
    <section class="modal-card app-dialog-card" role="dialog" aria-modal="true">
      <header class="modal-header">
        <h3>{{ dialog.title }}</h3>
        <button class="btn-icon-only modal-close-button" type="button" title="关闭" @click="emit('cancel')">
          ×
        </button>
      </header>

      <div class="modal-body app-dialog-body">
        <div class="app-dialog-icon" :data-tone="dialog.tone">
          <Icon :name="iconName" :size="22" />
        </div>
        <p>{{ dialog.message }}</p>
      </div>

      <footer class="modal-actions">
        <button type="button" class="btn btn-primary" @click="emit('confirm')">
          {{ dialog.confirmLabel }}
        </button>
        <button
          v-if="dialog.cancelLabel"
          type="button"
          class="btn btn-ghost"
          @click="emit('cancel')"
        >
          {{ dialog.cancelLabel }}
        </button>
      </footer>
    </section>
  </div>
</template>
