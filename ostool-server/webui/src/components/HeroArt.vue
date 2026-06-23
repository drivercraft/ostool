<script setup lang="ts">
/**
 * 首页 hero 配图：纯内联 SVG 抽象插画。
 *
 * 表现"开发板资源池 + 远程串口 + 网络启动"三层组合：
 *   - 上层：远程终端窗口（波形代表串口流）
 *   - 中层：开发板（CPU + 走线 + 状态灯）
 *   - 下层：网络拓扑节点（点对点连接）
 *
 * 不引用任何外部图片，所有形状用 currentColor 与品牌色变量着色，
 * 与页面整体去框化语言保持一致。
 */
</script>

<template>
  <svg
    class="hero-art"
    viewBox="0 0 520 420"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    focusable="false"
  >
    <defs>
      <linearGradient id="heroStroke" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#0f6f53" />
        <stop offset="1" stop-color="#0a5641" />
      </linearGradient>
      <linearGradient id="heroAccent" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0" stop-color="#1f6b4f" />
        <stop offset="1" stop-color="#3a8f6e" />
      </linearGradient>
      <pattern id="heroGrid" width="24" height="24" patternUnits="userSpaceOnUse">
        <path d="M24 0H0V24" fill="none" stroke="rgba(15,111,83,0.08)" stroke-width="1" />
      </pattern>
    </defs>

    <!-- 背景细网格 -->
    <rect x="20" y="20" width="480" height="380" rx="20" fill="url(#heroGrid)" />

    <!-- 上层：远程终端 -->
    <g transform="translate(60 50)">
      <rect x="0" y="0" width="320" height="120" rx="10" fill="#ffffff" stroke="url(#heroStroke)" stroke-width="1.5" />
      <circle cx="18" cy="20" r="3" fill="#a13a30" />
      <circle cx="30" cy="20" r="3" fill="#8a5a14" />
      <circle cx="42" cy="20" r="3" fill="#1f6b4f" />
      <path d="M16 44 l10 10 -10 10" stroke="url(#heroStroke)" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round" />
      <path d="M36 68 h40" stroke="url(#heroStroke)" stroke-width="1.8" stroke-linecap="round" />
      <!-- 串口波形 -->
      <path
        d="M16 92 q8 -14 16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0"
        stroke="url(#heroAccent)"
        stroke-width="1.6"
        fill="none"
        stroke-linecap="round"
      />
    </g>

    <!-- 中层：开发板 -->
    <g transform="translate(100 190)">
      <rect x="0" y="0" width="240" height="140" rx="8" fill="#f0f7f3" stroke="url(#heroStroke)" stroke-width="1.5" />
      <!-- CPU -->
      <rect x="90" y="40" width="60" height="60" rx="4" fill="#ffffff" stroke="url(#heroStroke)" stroke-width="1.5" />
      <rect x="108" y="58" width="24" height="24" rx="2" fill="url(#heroAccent)" opacity="0.18" />
      <!-- 走线 -->
      <path d="M30 30 H80 M30 30 V50 M30 50 H70" stroke="url(#heroStroke)" stroke-width="1.2" fill="none" />
      <path d="M210 30 H160 M210 30 V50 M210 50 H170" stroke="url(#heroStroke)" stroke-width="1.2" fill="none" />
      <path d="M30 110 H80 M30 110 V90 M30 90 H70" stroke="url(#heroStroke)" stroke-width="1.2" fill="none" />
      <path d="M210 110 H160 M210 110 V90 M210 90 H170" stroke="url(#heroStroke)" stroke-width="1.2" fill="none" />
      <!-- 状态灯 -->
      <circle cx="30" cy="20" r="3" fill="#1f6b4f" />
      <circle cx="42" cy="20" r="3" fill="#8a5a14" />
      <circle cx="210" cy="20" r="3" fill="#1f6b4f" />
      <!-- 引脚 -->
      <g stroke="url(#heroStroke)" stroke-width="1">
        <path d="M0 50 H-8 M0 70 H-8 M0 90 H-8" />
        <path d="M240 50 H248 M240 70 H248 M240 90 H248" />
      </g>
    </g>

    <!-- 下层：网络拓扑 -->
    <g transform="translate(140 350)">
      <circle cx="0" cy="0" r="6" fill="url(#heroAccent)" />
      <circle cx="60" cy="-30" r="5" fill="#ffffff" stroke="url(#heroStroke)" stroke-width="1.5" />
      <circle cx="120" cy="0" r="6" fill="url(#heroAccent)" />
      <circle cx="180" cy="-30" r="5" fill="#ffffff" stroke="url(#heroStroke)" stroke-width="1.5" />
      <circle cx="240" cy="0" r="6" fill="url(#heroAccent)" />
      <path d="M0 0 Q30 -20 60 -30 Q90 -40 120 0 Q150 40 180 -30 Q210 -60 240 0" stroke="url(#heroStroke)" stroke-width="1.2" fill="none" />
      <path d="M0 0 H120 M120 0 H240" stroke="url(#heroStroke)" stroke-width="1" stroke-dasharray="3 4" opacity="0.6" />
    </g>

    <!-- 装饰角标 -->
    <g transform="translate(440 70)" opacity="0.6">
      <circle cx="0" cy="0" r="22" fill="none" stroke="url(#heroStroke)" stroke-width="1" stroke-dasharray="3 4" />
      <path d="M-6 0 H6 M0 -6 V6" stroke="url(#heroAccent)" stroke-width="1.5" stroke-linecap="round" />
    </g>
  </svg>
</template>

<style scoped>
.hero-art {
  display: block;
  width: 100%;
  height: auto;
  max-width: 520px;
}
</style>
