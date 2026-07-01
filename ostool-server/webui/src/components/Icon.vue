<script setup lang="ts">
/**
 * 轻量内联 SVG 图标组件。
 *
 * 设计原则：
 *   - 纯 stroke 路径，继承 currentColor，与文字同色；
 *   - 24x24 viewBox，stroke-width 1.6，line-cap / line-join round；
 *   - 不引入外部图标库，避免新增依赖。
 */
withDefaults(defineProps<{ name: IconName; size?: number | string }>(), {
  size: 20,
});
</script>

<template>
  <svg
    class="icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
    v-html="paths[name]"
  />
</template>

<script lang="ts">
export type IconName =
  | "home"
  | "chip"
  | "book"
  | "login"
  | "logout"
  | "user"
  | "users"
  | "shield"
  | "power"
  | "terminal"
  | "server"
  | "cube"
  | "pulse"
  | "chart"
  | "search"
  | "refresh"
  | "plus"
  | "edit"
  | "trash"
  | "arrow-right"
  | "arrow-left"
  | "folder"
  | "settings"
  | "key"
  | "clipboard"
  | "link"
  | "play"
  | "bolt"
  | "check"
  | "bell"
  | "sun"
  | "moon"
  | "monitor"
  | "chevron-right"
  | "sparkles"
  | "globe"
  | "lock"
  | "cpu-board"
  | "wave"
  | "circuit"
  | "more-vertical"
  | "key-reset"
  | "ban";

const paths: Record<IconName, string> = {
  home: `<path d="M3 11.5 12 4l9 7.5" /><path d="M5 10v9a1 1 0 0 0 1 1h4v-6h4v6h4a1 1 0 0 0 1-1v-9" />`,
  chip: `<rect x="6" y="6" width="12" height="12" rx="2" /><rect x="9.5" y="9.5" width="5" height="5" rx="1" /><path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3" />`,
  "cpu-board": `<rect x="4" y="5" width="16" height="14" rx="2" /><rect x="8" y="9" width="8" height="6" rx="1" /><path d="M8 2v3M12 2v3M16 2v3M8 19v3M12 19v3M16 19v3M2 9h2M2 14h2M20 9h2M20 14h2" />`,
  book: `<path d="M4 5a2 2 0 0 1 2-2h12v16H6a2 2 0 0 1-2-2z" /><path d="M4 17a2 2 0 0 1 2-2h12" /><path d="M8 7h7M8 10h7" />`,
  login: `<path d="M15 12H4" /><path d="m10 8 4 4-4 4" /><path d="M14 4h5a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-5" />`,
  logout: `<path d="M10 12h11" /><path d="m17 8 4 4-4 4" /><path d="M10 4H5a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1h5" />`,
  user: `<circle cx="12" cy="8" r="4" /><path d="M4 20a8 8 0 0 1 16 0" />`,
  users: `<circle cx="9" cy="8" r="3.2" /><path d="M3 19a6 6 0 0 1 12 0" /><path d="M16 5.5a3 3 0 0 1 0 5.8" /><path d="M17.5 19a6 6 0 0 0-3-5.2" />`,
  shield: `<path d="M12 3 5 6v6c0 4 3 6.5 7 9 4-2.5 7-5 7-9V6z" /><path d="m9 12 2 2 4-4" />`,
  power: `<path d="M12 3v8" /><path d="M7.5 6.5a7 7 0 1 0 9 0" />`,
  terminal: `<rect x="3" y="4" width="18" height="16" rx="2" /><path d="m7 9 3 3-3 3" /><path d="M13 15h4" />`,
  server: `<rect x="3" y="4" width="18" height="7" rx="1.5" /><rect x="3" y="13" width="18" height="7" rx="1.5" /><path d="M7 7.5h.01M7 16.5h.01" />`,
  cube: `<path d="M12 3 4 7v10l8 4 8-4V7z" /><path d="m4 7 8 4 8-4" /><path d="M12 21V11" />`,
  pulse: `<path d="M3 12h4l2-5 3 10 2-5h7" />`,
  chart: `<path d="M4 4v16h16" /><path d="M8 16v-4M12 16V8M16 16v-6" />`,
  search: `<circle cx="11" cy="11" r="6" /><path d="m20 20-4-4" />`,
  refresh: `<path d="M4 12a8 8 0 0 1 14-5.3L21 9" /><path d="M21 4v5h-5" /><path d="M20 12a8 8 0 0 1-14 5.3L3 15" /><path d="M3 20v-5h5" />`,
  plus: `<path d="M12 5v14M5 12h14" />`,
  edit: `<path d="M4 20h4l10-10-4-4L4 16z" /><path d="m13.5 6.5 4 4" />`,
  trash: `<path d="M4 7h16" /><path d="M9 7V4h6v3" /><path d="M6 7v13a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V7" /><path d="M10 11v6M14 11v6" />`,
  "arrow-right": `<path d="M5 12h14" /><path d="m13 6 6 6-6 6" />`,
  "arrow-left": `<path d="M19 12H5" /><path d="m11 6-6 6 6 6" />`,
  folder: `<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />`,
  settings: `<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.6 1.6 0 0 0 .3 1.7l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-1.7-.3 1.6 1.6 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.7.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0 .3-1.7 1.6 1.6 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.7l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.7.3h.1a1.6 1.6 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.7-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.7v.1a1.6 1.6 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1z" />`,
  key: `<circle cx="8" cy="14" r="4" /><path d="m11 11 9-9" /><path d="m16 6 3 3" /><path d="m18 4 2 2" />`,
  clipboard: `<rect x="5" y="4" width="14" height="17" rx="2" /><path d="M9 4a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2" /><path d="M9 11h6M9 15h4" />`,
  link: `<path d="M10 13a4 4 0 0 0 6 .5l2.5-2.5a4 4 0 0 0-5.6-5.6L11.5 7" /><path d="M14 11a4 4 0 0 0-6-.5L5.5 13a4 4 0 0 0 5.6 5.6l1.4-1.4" />`,
  play: `<path d="M7 4v16l13-8z" />`,
  bolt: `<path d="M13 3 5 13h6l-1 8 8-10h-6z" />`,
  check: `<path d="m5 12 4 4 10-10" />`,
  bell: `<path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9" /><path d="M10 21a2 2 0 0 0 4 0" />`,
  sun: `<circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" />`,
  moon: `<path d="M20 15.5A8.5 8.5 0 0 1 8.5 4 7 7 0 1 0 20 15.5z" />`,
  monitor: `<rect x="3" y="4" width="18" height="13" rx="2" /><path d="M8 21h8M12 17v4" />`,
  "chevron-right": `<path d="m9 6 6 6-6 6" />`,
  sparkles: `<path d="M12 3v4M12 17v4M3 12h4M17 12h4" /><path d="m6.3 6.3 2.4 2.4M15.3 15.3l2.4 2.4M17.7 6.3l-2.4 2.4M8.7 15.3l-2.4 2.4" />`,
  globe: `<circle cx="12" cy="12" r="9" /><path d="M3 12h18" /><path d="M12 3a14 14 0 0 1 0 18 14 14 0 0 1 0-18z" />`,
  lock: `<rect x="5" y="11" width="14" height="9" rx="2" /><path d="M8 11V8a4 4 0 0 1 8 0v3" />`,
  wave: `<path d="M3 12c2 0 2-5 4-5s2 10 4 10 2-10 4-10 2 5 4 5" />`,
  circuit: `<circle cx="6" cy="6" r="1.5" /><circle cx="18" cy="6" r="1.5" /><circle cx="6" cy="18" r="1.5" /><circle cx="18" cy="18" r="1.5" /><circle cx="12" cy="12" r="2.5" /><path d="M7.5 6h9M6 7.5v9M18 7.5v9M7.5 18h9M9.6 9.6 6 6M14.4 9.6 18 6M9.6 14.4 6 18M14.4 14.4 18 18" />`,
  "more-vertical": `<circle cx="12" cy="5" r="1.6" /><circle cx="12" cy="12" r="1.6" /><circle cx="12" cy="19" r="1.6" />`,
  "key-reset": `<circle cx="8" cy="14" r="4" /><path d="m11 11 9-9" /><path d="m16 6 3 3" /><path d="M14 4h6v6" />`,
  ban: `<circle cx="12" cy="12" r="9" /><path d="m5.6 5.6 12.8 12.8" />`,
};
</script>

<style scoped>
.icon {
  display: inline-block;
  flex-shrink: 0;
  vertical-align: middle;
}
</style>
