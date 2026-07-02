<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";

import Icon, { type IconName } from "@/components/Icon.vue";
import { type UiThemeMode, useUiStore } from "@/stores/ui";

const ui = useUiStore();
const open = ref(false);

const themeOptions: Array<{ value: UiThemeMode; label: string; icon: IconName }> = [
  { value: "light", label: "明亮", icon: "sun" },
  { value: "dark", label: "黑暗", icon: "moon" },
  { value: "system", label: "跟随系统", icon: "monitor" },
];

const themeButtonIcon = computed<IconName>(() => (ui.effectiveTheme === "dark" ? "moon" : "sun"));

function selectThemeMode(value: UiThemeMode) {
  ui.setThemeMode(value);
  open.value = false;
}

function closeOnDocumentClick(event: MouseEvent) {
  if ((event.target as HTMLElement | null)?.closest(".theme-menu-shell")) {
    return;
  }
  open.value = false;
}

function closeOnEscape(event: KeyboardEvent) {
  if (event.key === "Escape") {
    open.value = false;
  }
}

onMounted(() => {
  document.addEventListener("click", closeOnDocumentClick);
  document.addEventListener("keydown", closeOnEscape);
});

onUnmounted(() => {
  document.removeEventListener("click", closeOnDocumentClick);
  document.removeEventListener("keydown", closeOnEscape);
});
</script>

<template>
  <div class="header-menu-shell theme-menu-shell">
    <button
      class="btn-icon-only"
      type="button"
      title="明暗主题"
      aria-label="明暗主题"
      :aria-expanded="open"
      @click="open = !open"
    >
      <Icon :name="themeButtonIcon" :size="18" />
    </button>
    <div v-if="open" class="header-menu">
      <div class="header-menu-title">主题</div>
      <button
        v-for="option in themeOptions"
        :key="option.value"
        class="header-menu-item"
        :class="{ 'is-active': ui.themeMode === option.value }"
        type="button"
        @click="selectThemeMode(option.value)"
      >
        <span>
          <Icon :name="option.icon" :size="15" />
          {{ option.label }}
        </span>
        <Icon v-if="ui.themeMode === option.value" name="check" :size="14" />
      </button>
    </div>
  </div>
</template>
