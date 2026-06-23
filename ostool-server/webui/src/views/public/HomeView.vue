<script setup lang="ts">
import { RouterLink } from "vue-router";

import HeroArt from "@/components/HeroArt.vue";
import Icon, { type IconName } from "@/components/Icon.vue";

interface Feature {
  eyebrow: string;
  title: string;
  description: string;
  icon: IconName;
}

interface Step {
  step: string;
  title: string;
  description: string;
  icon: IconName;
}

interface Section {
  id: string;
  eyebrow: string;
  title: string;
  description: string;
  art: "pool" | "workflow";
  features: Feature[];
}

const heroCta = [
  { to: "/resources", label: "浏览可用资源", primary: true, icon: "arrow-right" as IconName },
  { to: "/docs", label: "查看使用文档", primary: false, icon: "book" as IconName },
];

const heroStats = [
  { label: "资源池", value: "统一调度" },
  { label: "远程访问", value: "WebSocket 串口" },
  { label: "启动方式", value: "TFTP / HTTP Boot" },
];

const sections: Section[] = [
  {
    id: "capabilities",
    eyebrow: "Platform Capabilities",
    title: "把硬件实验室变成可调度的共享资源",
    description:
      "ostool-server 把开发板资源池、远程串口、网络启动与电源编排整合到统一平台，让 OS 与嵌入式团队不再为工位、USB 线与启动脚本分配而耗费时间。",
    art: "pool",
    features: [
      {
        eyebrow: "Resource Pool",
        title: "按需租赁的开发板池",
        description:
          "按板型、标签与可用状态筛选并申请开发板，平台自动从池中分配空闲设备并维护租约，避免多团队抢占同一块硬件。",
        icon: "cpu-board",
      },
      {
        eyebrow: "Remote Console",
        title: "远程串口终端",
        description:
          "通过 WebSocket 直接访问开发板串口，免去插拔 USB 线与切换工位，调试体验与本地终端一致。",
        icon: "terminal",
      },
      {
        eyebrow: "Network Boot",
        title: "TFTP / UEFI HTTP Boot",
        description:
          "内置 TFTP 与 UEFI HTTP Boot，可在会话作用域内上传内核、设备树与 ramfs，按板型自动生成启动命令。",
        icon: "server",
      },
      {
        eyebrow: "Power Orchestration",
        title: "电源与启动编排",
        description:
          "支持自定义电源命令与中盛继电器，按需上下电并自动完成 U-Boot 流程，无需人工值守。",
        icon: "power",
      },
    ],
  },
  {
    id: "workflow",
    eyebrow: "End-to-End Workflow",
    title: "从镜像上传到远程启动的完整闭环",
    description:
      "平台覆盖镜像托管、网络启动、串口交互与会话生命周期管理，所有步骤都通过统一的 API 与 Web 控制台暴露。",
    art: "workflow",
    features: [
      {
        eyebrow: "Image Hosting",
        title: "会话级镜像托管",
        description:
          "每个会话都有独立文件作用域，上传的内核、DTB 与 ramfs 仅对该会话可见，结束后自动清理。",
        icon: "cube",
      },
      {
        eyebrow: "Boot Automation",
        title: "自动启动命令",
        description:
          "平台根据板型生成 bootcmd、加载地址与网络配置，开发者无需手写 U-Boot 脚本即可启动镜像。",
        icon: "bolt",
      },
      {
        eyebrow: "Lease Lifecycle",
        title: "租约与心跳",
        description:
          "通过心跳接口维持租约，长时无心跳的会话会自动释放，绑定的开发板重新进入可用池。",
        icon: "pulse",
      },
      {
        eyebrow: "Observability",
        title: "运行态可观测",
        description:
          "管理台提供 TFTP 诊断、会话状态、电源事件与资源统计，问题出现时可以快速定位。",
        icon: "chart",
      },
    ],
  },
];

const steps: Step[] = [
  {
    step: "01",
    title: "浏览资源",
    description: "在资源页查看平台当前支持的开发板类型与可用数量。",
    icon: "search",
  },
  {
    step: "02",
    title: "登录账号",
    description: "使用平台账号登录，认证后获得属于自己的会话与租约。",
    icon: "login",
  },
  {
    step: "03",
    title: "申请会话",
    description: "为指定开发板类型创建会话，自动分配空闲设备并启动串口与启动通道。",
    icon: "link",
  },
  {
    step: "04",
    title: "进入控制台",
    description: "在用户控制台中查看租约、上传镜像、连接串口并按需上下电。",
    icon: "terminal",
  },
];
</script>

<template>
  <div class="home-page">
    <section class="hero">
      <div class="hero-inner">
        <div class="hero-copy">
          <p class="eyebrow">Enterprise Hardware Lab Platform</p>
          <h1 class="hero-title">
            面向团队的<span class="gradient-text">开发板租赁</span>与远程调试平台
          </h1>
          <div class="hero-art-wrap hero-art-wrap--inline" aria-hidden="true">
            <HeroArt />
          </div>
          <p class="hero-subtitle">
            ostool-server 把开发板资源池、远程串口、网络启动与电源编排统一到一个平台，
            帮助 OS 与嵌入式团队把硬件实验室变成可预约、可审计、可自动化的共享基础设施。
          </p>
          <div class="hero-actions">
            <RouterLink
              v-for="cta in heroCta"
              :key="cta.to"
              :to="cta.to"
              :class="cta.primary ? 'primary-button' : 'ghost-button'"
            >
              {{ cta.label }}
              <Icon :name="cta.icon" :size="16" class="btn-icon" />
            </RouterLink>
          </div>
          <dl class="hero-stats">
            <div v-for="stat in heroStats" :key="stat.label">
              <dt>{{ stat.label }}</dt>
              <dd>{{ stat.value }}</dd>
            </div>
          </dl>
        </div>
        <div class="hero-visual" aria-hidden="true">
          <div class="hero-art-wrap"><HeroArt /></div>
        </div>
      </div>
      <div class="hero-scroll-hint" aria-hidden="true">
        <Icon name="chevron-right" :size="16" />
      </div>
    </section>

    <section
      v-for="(section, index) in sections"
      :key="section.id"
      class="home-section"
      :class="{ 'home-section-alt': index % 2 === 1 }"
    >
      <div class="home-section-inner">
        <header class="home-section-header">
          <p class="eyebrow">{{ section.eyebrow }}</p>
          <h3>{{ section.title }}</h3>
          <p class="home-section-lead">{{ section.description }}</p>
        </header>
        <div class="feature-grid">
          <article v-for="feature in section.features" :key="feature.title" class="feature-card">
            <span class="feature-icon">
              <Icon :name="feature.icon" :size="22" />
            </span>
            <p class="feature-eyebrow">{{ feature.eyebrow }}</p>
            <h4>{{ feature.title }}</h4>
            <p>{{ feature.description }}</p>
          </article>
        </div>
      </div>
    </section>

    <section class="home-section home-section-alt">
      <div class="home-section-inner">
        <header class="home-section-header">
          <p class="eyebrow">Getting Started</p>
          <h3>四步即可上手一块开发板</h3>
          <p class="home-section-lead">
            从浏览资源到启动镜像，整套流程都在浏览器内完成，无需在工位之间切换。
          </p>
        </header>
        <div class="steps">
          <div v-for="step in steps" :key="step.step" class="step-card">
            <span class="step-icon">
              <Icon :name="step.icon" :size="20" />
            </span>
            <div class="step-num">{{ step.step }}</div>
            <h4>{{ step.title }}</h4>
            <p>{{ step.description }}</p>
          </div>
        </div>
      </div>
    </section>

    <section class="cta-band">
      <div>
        <h3>准备好开始你的开发板项目了吗？</h3>
        <p>登录后即可在用户控制台中创建会话、上传镜像并连接开发板。</p>
      </div>
      <div class="cta-actions">
        <RouterLink class="btn btn-light" to="/login">
          立即登录
          <Icon name="arrow-right" :size="16" class="btn-icon" />
        </RouterLink>
        <RouterLink class="btn btn-ghost-light" to="/register">注册账号</RouterLink>
      </div>
    </section>
  </div>
</template>
