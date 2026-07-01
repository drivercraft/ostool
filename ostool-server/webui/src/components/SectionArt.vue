<script setup lang="ts">
/**
 * 首页各 section 的配图组件。
 *
 * 通过 variant 选择不同的纯 SVG 抽象插画，与 HeroArt 保持同一视觉语言：
 *   - pool: 资源池 — 中央调度 hub 连接多块开发板节点
 *   - workflow: 端到端闭环 — 镜像上传 → 网络启动 → 开发板 → 串口回环
 *
 * 全部使用 currentColor / 品牌色变量，不引用外部图片。
 */
defineProps<{
  variant: "pool" | "workflow";
}>();
</script>

<template>
  <svg
    v-if="variant === 'pool'"
    class="section-art"
    viewBox="0 0 560 460"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    focusable="false"
  >
    <defs>
      <linearGradient id="poolStroke" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="var(--c-brand-light)" />
        <stop offset="1" stop-color="var(--c-brand-dark)" />
      </linearGradient>
      <linearGradient id="poolAccent" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0" stop-color="var(--c-brand-light)" />
        <stop offset="1" stop-color="var(--c-violet)" />
      </linearGradient>
      <pattern id="poolGrid" width="26" height="26" patternUnits="userSpaceOnUse">
        <path d="M26 0H0V26" fill="none" stroke="rgba(99,102,241,0.08)" stroke-width="1" />
      </pattern>
    </defs>

    <rect x="10" y="10" width="540" height="440" rx="20" fill="url(#poolGrid)" />

    <!-- 连接线（先画在底层） -->
    <g stroke="url(#poolStroke)" stroke-width="1.4" fill="none" opacity="0.55">
      <path d="M280 230 L120 110" />
      <path d="M280 230 L440 110" />
      <path d="M280 230 L100 240" />
      <path d="M280 230 L460 240" />
      <path d="M280 230 L140 370" />
      <path d="M280 230 L420 370" />
    </g>

    <!-- 中央调度 hub -->
    <g transform="translate(280 230)">
      <circle cx="0" cy="0" r="44" fill="var(--c-surface)" stroke="url(#poolStroke)" stroke-width="1.6" />
      <circle cx="0" cy="0" r="44" fill="none" stroke="url(#poolAccent)" stroke-width="1.4" stroke-dasharray="3 5" opacity="0.6" />
      <circle cx="0" cy="0" r="14" fill="url(#poolAccent)" />
      <path d="M-22 0 H-30 M22 0 H30 M0 -22 V-30 M0 22 V30" stroke="url(#poolStroke)" stroke-width="1.6" stroke-linecap="round" />
    </g>

    <!-- 周围开发板节点（6 块） -->
    <g v-for="(node, i) in [
      { x: 120, y: 110 },
      { x: 440, y: 110 },
      { x: 100, y: 240 },
      { x: 460, y: 240 },
      { x: 140, y: 370 },
      { x: 420, y: 370 },
    ]" :key="i" :transform="`translate(${node.x} ${node.y})`">
      <rect x="-34" y="-24" width="68" height="48" rx="6" fill="var(--c-surface)" stroke="url(#poolStroke)" stroke-width="1.4" />
      <rect x="-14" y="-10" width="28" height="20" rx="2" fill="url(#poolAccent)" opacity="0.2" />
      <circle cx="-24" cy="-14" r="2.4" fill="var(--c-success)" />
      <circle cx="-16" cy="-14" r="2.4" fill="var(--c-warning)" />
      <path d="M-34 -4 H-40 M34 -4 H40" stroke="url(#poolStroke)" stroke-width="1" />
      <path d="M-34 8 H-40 M34 8 H40" stroke="url(#poolStroke)" stroke-width="1" />
    </g>

    <!-- 装饰角标 -->
    <g transform="translate(510 50)" opacity="0.6">
      <circle cx="0" cy="0" r="18" fill="none" stroke="url(#poolStroke)" stroke-width="1" stroke-dasharray="3 4" />
      <circle cx="0" cy="0" r="4" fill="url(#poolAccent)" />
    </g>
  </svg>

  <svg
    v-else-if="variant === 'workflow'"
    class="section-art"
    viewBox="0 0 560 460"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    focusable="false"
  >
    <defs>
      <linearGradient id="wfStroke" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="var(--c-brand-light)" />
        <stop offset="1" stop-color="var(--c-brand-dark)" />
      </linearGradient>
      <linearGradient id="wfAccent" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0" stop-color="var(--c-sky)" />
        <stop offset="1" stop-color="var(--c-brand-light)" />
      </linearGradient>
      <pattern id="wfGrid" width="26" height="26" patternUnits="userSpaceOnUse">
        <path d="M26 0H0V26" fill="none" stroke="rgba(124,58,237,0.07)" stroke-width="1" />
      </pattern>
    </defs>

    <rect x="10" y="10" width="540" height="440" rx="20" fill="url(#wfGrid)" />

    <!-- 流水线主轴 -->
    <path d="M70 230 H490" stroke="url(#wfStroke)" stroke-width="1.4" stroke-dasharray="2 6" opacity="0.5" />

    <!-- 节点 1: 镜像上传（cube + 上箭头） -->
    <g transform="translate(90 230)">
      <rect x="-44" y="-44" width="88" height="88" rx="12" fill="var(--c-surface)" stroke="url(#wfStroke)" stroke-width="1.6" />
      <path d="M0 -22 L-16 -12 V4 L0 14 L16 4 V-12 Z" fill="none" stroke="url(#wfAccent)" stroke-width="1.6" stroke-linejoin="round" />
      <path d="M-16 -12 L0 -2 L16 -12 M0 -2 V14" stroke="url(#wfAccent)" stroke-width="1.6" stroke-linejoin="round" />
      <path d="M-22 -30 V-38 M-22 -38 H-18 M-22 -38 l3 -3 M-22 -38 l-3 3" stroke="url(#wfStroke)" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" opacity="0.7" />
      <text x="0" y="60" text-anchor="middle" font-size="11" fill="var(--c-text-muted)" font-family="inherit">Image</text>
    </g>

    <!-- 箭头 1 -->
    <path d="M148 230 H188" stroke="url(#wfAccent)" stroke-width="1.8" stroke-linecap="round" />
    <path d="M184 224 l8 6 -8 6" stroke="url(#wfAccent)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" fill="none" />

    <!-- 节点 2: 网络启动（server） -->
    <g transform="translate(230 230)">
      <rect x="-44" y="-44" width="88" height="88" rx="12" fill="var(--c-surface)" stroke="url(#wfStroke)" stroke-width="1.6" />
      <rect x="-18" y="-20" width="36" height="14" rx="2" fill="none" stroke="url(#wfAccent)" stroke-width="1.6" />
      <rect x="-18" y="-2" width="36" height="14" rx="2" fill="none" stroke="url(#wfAccent)" stroke-width="1.6" />
      <circle cx="-11" cy="-13" r="2" fill="var(--c-success)" />
      <circle cx="-11" cy="5" r="2" fill="var(--c-success)" />
      <text x="0" y="60" text-anchor="middle" font-size="11" fill="var(--c-text-muted)" font-family="inherit">Boot</text>
    </g>

    <!-- 箭头 2 -->
    <path d="M288 230 H328" stroke="url(#wfAccent)" stroke-width="1.8" stroke-linecap="round" />
    <path d="M324 224 l8 6 -8 6" stroke="url(#wfAccent)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" fill="none" />

    <!-- 节点 3: 开发板 -->
    <g transform="translate(370 230)">
      <rect x="-44" y="-44" width="88" height="88" rx="12" fill="var(--c-surface)" stroke="url(#wfStroke)" stroke-width="1.6" />
      <rect x="-20" y="-16" width="40" height="32" rx="3" fill="var(--c-surface-2)" stroke="url(#wfStroke)" stroke-width="1.4" />
      <rect x="-8" y="-6" width="16" height="12" rx="1.5" fill="url(#wfAccent)" opacity="0.25" />
      <path d="M-20 -8 H-26 M-20 0 H-26 M20 -8 H26 M20 0 H26" stroke="url(#wfStroke)" stroke-width="1.2" />
      <circle cx="-14" cy="-10" r="1.8" fill="var(--c-success)" />
      <text x="0" y="60" text-anchor="middle" font-size="11" fill="var(--c-text-muted)" font-family="inherit">Board</text>
    </g>

    <!-- 闭环回环箭头：从 board → 回到 image（顶部弧线） -->
    <path d="M370 186 C 370 110, 90 110, 90 186" stroke="url(#wfAccent)" stroke-width="1.6" fill="none" stroke-dasharray="4 5" opacity="0.7" />
    <path d="M90 182 l-6 -8 -6 8" stroke="url(#wfAccent)" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none" opacity="0.7" transform="rotate(180 90 186)" />

    <!-- 串口波形输出（底部） -->
    <g transform="translate(150 360)">
      <path d="M0 0 q8 -14 16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0" stroke="url(#wfAccent)" stroke-width="1.6" fill="none" stroke-linecap="round" opacity="0.8" />
      <path d="M-14 0 H-26" stroke="url(#wfStroke)" stroke-width="1.4" stroke-linecap="round" />
      <text x="120" y="22" text-anchor="middle" font-size="11" fill="var(--c-text-muted)" font-family="inherit">Serial Output</text>
    </g>
  </svg>
</template>

<style scoped>
.section-art {
  display: block;
  width: 100%;
  height: auto;
  max-width: 680px;
}
</style>
