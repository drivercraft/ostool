<script setup lang="ts">
/**
 * 首页 hero 配图：纯内联 SVG 抽象插画。
 *
 * 以"资源控制台"为视觉核心：浮动窗口展示开发板资源池状态、
 * 串口波形与网络启动节点，呼应平台的资源池 + 远程串口 + 网络启动能力。
 * 全部使用品牌渐变色（indigo → violet → sky），不引用外部图片。
 */
</script>

<template>
  <svg
    class="hero-art"
    viewBox="0 0 520 460"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
    focusable="false"
  >
    <defs>
      <linearGradient id="heroStroke" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#6366f1" />
        <stop offset="1" stop-color="#4338ca" />
      </linearGradient>
      <linearGradient id="heroAccent" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0" stop-color="#6366f1" />
        <stop offset="1" stop-color="#0ea5e9" />
      </linearGradient>
      <linearGradient id="heroViolet" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#7c3aed" />
        <stop offset="1" stop-color="#6366f1" />
      </linearGradient>
      <linearGradient id="heroSoft" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#eef2ff" />
        <stop offset="1" stop-color="#e0f2fe" />
      </linearGradient>
      <pattern id="heroGrid" width="24" height="24" patternUnits="userSpaceOnUse">
        <path d="M24 0H0V24" fill="none" stroke="rgba(99,102,241,0.10)" stroke-width="1" />
      </pattern>
    </defs>

    <!-- 软背景圆斑 -->
    <circle cx="80" cy="70" r="60" fill="url(#heroSoft)" opacity="0.7" />
    <circle cx="450" cy="400" r="80" fill="url(#heroSoft)" opacity="0.6" />

    <!-- 主控制台卡片 -->
    <g transform="translate(40 60)">
      <rect x="0" y="0" width="400" height="300" rx="16" fill="#ffffff" stroke="#e6e8f0" stroke-width="1" />
      <rect x="0" y="0" width="400" height="44" rx="16" fill="#f5f7ff" />
      <rect x="0" y="28" width="400" height="16" fill="#f5f7ff" />
      <circle cx="22" cy="22" r="4" fill="#fda4af" />
      <circle cx="38" cy="22" r="4" fill="#fcd34d" />
      <circle cx="54" cy="22" r="4" fill="#86efac" />
      <rect x="150" y="14" width="100" height="16" rx="8" fill="#e0e7ff" />

      <!-- 标题行 -->
      <text x="24" y="76" font-family="Inter, sans-serif" font-size="13" font-weight="700" fill="#0f1729">开发板资源池</text>
      <rect x="320" y="64" width="56" height="18" rx="9" fill="#ecfdf5" />
      <circle cx="332" cy="73" r="3" fill="#10b981" />
      <text x="340" y="77" font-family="Inter, sans-serif" font-size="10" font-weight="700" fill="#059669">在线</text>

      <!-- 资源行 -->
      <g transform="translate(24 96)">
        <rect x="0" y="0" width="352" height="40" rx="8" fill="#fbfcfe" stroke="#eef0f5" />
        <rect x="12" y="12" width="16" height="16" rx="4" fill="url(#heroViolet)" />
        <text x="38" y="25" font-family="Inter, sans-serif" font-size="11" font-weight="600" fill="#2e3856">arm64 · lab</text>
        <rect x="250" y="14" width="60" height="12" rx="6" fill="#eef0f5" />
        <rect x="250" y="14" width="42" height="12" rx="6" fill="url(#heroAccent)" />
        <rect x="318" y="13" width="22" height="14" rx="7" fill="#ecfdf5" />
      </g>
      <g transform="translate(24 142)">
        <rect x="0" y="0" width="352" height="40" rx="8" fill="#fbfcfe" stroke="#eef0f5" />
        <rect x="12" y="12" width="16" height="16" rx="4" fill="url(#heroAccent)" />
        <text x="38" y="25" font-family="Inter, sans-serif" font-size="11" font-weight="600" fill="#2e3856">stm32mp1</text>
        <rect x="250" y="14" width="60" height="12" rx="6" fill="#eef0f5" />
        <rect x="250" y="14" width="18" height="12" rx="6" fill="#f59e0b" />
        <rect x="318" y="13" width="22" height="14" rx="7" fill="#fffbeb" />
      </g>
      <g transform="translate(24 188)">
        <rect x="0" y="0" width="352" height="40" rx="8" fill="#fbfcfe" stroke="#eef0f5" />
        <rect x="12" y="12" width="16" height="16" rx="4" fill="url(#heroViolet)" />
        <text x="38" y="25" font-family="Inter, sans-serif" font-size="11" font-weight="600" fill="#2e3856">qemu-aarch64</text>
        <rect x="250" y="14" width="60" height="12" rx="6" fill="#eef0f5" />
        <rect x="250" y="14" width="52" height="12" rx="6" fill="url(#heroAccent)" />
        <rect x="318" y="13" width="22" height="14" rx="7" fill="#ecfdf5" />
      </g>
      <!-- 串口波形 -->
      <path
        d="M24 258 q8 -16 16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0 t16 0"
        stroke="url(#heroAccent)"
        stroke-width="2"
        fill="none"
        stroke-linecap="round"
        opacity="0.85"
      />
      <text x="24" y="244" font-family="Inter, sans-serif" font-size="9" font-weight="700" fill="#98a2b3" letter-spacing="1">SERIAL CONSOLE</text>
    </g>

    <!-- 浮动小卡：电源编排 -->
    <g transform="translate(330 320)">
      <rect x="0" y="0" width="150" height="74" rx="14" fill="#ffffff" stroke="#e6e8f0" stroke-width="1" filter="drop-shadow(0 10px 20px rgba(15,23,41,.08))" />
      <rect x="14" y="14" width="34" height="34" rx="9" fill="#eef2ff" />
      <path d="M31 22 v9 a6 6 0 1 0 5 -3" stroke="#6366f1" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round" />
      <text x="58" y="30" font-family="Inter, sans-serif" font-size="11" font-weight="700" fill="#0f1729">电源编排</text>
      <text x="58" y="44" font-family="Inter, sans-serif" font-size="9" fill="#697386">继电器 · 自动上下电</text>
      <circle cx="132" cy="22" r="4" fill="#10b981" />
    </g>

    <!-- 浮动小卡：网络启动 -->
    <g transform="translate(20 330)">
      <rect x="0" y="0" width="150" height="74" rx="14" fill="#ffffff" stroke="#e6e8f0" stroke-width="1" filter="drop-shadow(0 10px 20px rgba(15,23,41,.08))" />
      <rect x="14" y="14" width="34" height="34" rx="9" fill="#e0f2fe" />
      <path d="M22 36 V30 a4 4 0 0 1 8 0 V36 M22 36 H30 M30 36 V24 a4 4 0 0 1 8 0 V36 M30 36 H38" stroke="#0ea5e9" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round" />
      <text x="58" y="30" font-family="Inter, sans-serif" font-size="11" font-weight="700" fill="#0f1729">网络启动</text>
      <text x="58" y="44" font-family="Inter, sans-serif" font-size="9" fill="#697386">TFTP · UEFI HTTP</text>
      <circle cx="132" cy="22" r="4" fill="#6366f1" />
    </g>

    <!-- 装饰节点 -->
    <g transform="translate(470 90)" opacity="0.7">
      <circle cx="0" cy="0" r="24" fill="none" stroke="url(#heroStroke)" stroke-width="1.2" stroke-dasharray="3 5" />
      <circle cx="0" cy="0" r="6" fill="url(#heroAccent)" />
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
