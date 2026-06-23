<script setup lang="ts">
import { RouterLink } from "vue-router";

import HeroArt from "@/components/HeroArt.vue";
import Icon, { type IconName } from "@/components/Icon.vue";
import SectionArt from "@/components/SectionArt.vue";

interface Feature {
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
    title: "把硬件实验室变成可调度的共享资源",
    description:
      "ostool-server 把开发板资源池、远程串口、网络启动与电源编排整合到统一平台，让 OS 与嵌入式团队不再为工位、USB 线与启动脚本分配而耗费时间。",
    art: "pool",
    features: [
      {
        title: "按需租赁的开发板池",
        description:
          "按板型、标签与可用状态筛选并申请开发板，平台自动从池中分配空闲设备并维护租约，避免多团队抢占同一块硬件。",
        icon: "cpu-board",
      },
      {
        title: "远程串口终端",
        description:
          "通过 WebSocket 直接访问开发板串口，免去插拔 USB 线与切换工位，调试体验与本地终端一致。",
        icon: "terminal",
      },
      {
        title: "TFTP / UEFI HTTP Boot",
        description:
          "内置 TFTP 与 UEFI HTTP Boot，可在会话作用域内上传内核、设备树与 ramfs，按板型自动生成启动命令。",
        icon: "server",
      },
      {
        title: "电源与启动编排",
        description:
          "支持自定义电源命令与中盛继电器，按需上下电并自动完成 U-Boot 流程，无需人工值守。",
        icon: "power",
      },
    ],
  },
  {
    id: "workflow",
    title: "从镜像上传到远程启动的完整闭环",
    description:
      "平台覆盖镜像托管、网络启动、串口交互与会话生命周期管理，所有步骤都通过统一的 API 与 Web 控制台暴露。",
    art: "workflow",
    features: [
      {
        title: "会话级镜像托管",
        description:
          "每个会话都有独立文件作用域，上传的内核、DTB 与 ramfs 仅对该会话可见，结束后自动清理。",
        icon: "cube",
      },
      {
        title: "自动启动命令",
        description:
          "平台根据板型生成 bootcmd、加载地址与网络配置，开发者无需手写 U-Boot 脚本即可启动镜像。",
        icon: "bolt",
      },
      {
        title: "租约与心跳",
        description:
          "通过心跳接口维持租约，长时无心跳的会话会自动释放，绑定的开发板重新进入可用池。",
        icon: "pulse",
      },
      {
        title: "运行态可观测",
        description:
          "管理台提供 TFTP 诊断、会话状态、电源事件与资源统计，问题出现时可以快速定位。",
        icon: "chart",
      },
    ],
  },
  {
    id: "governance",
    title: "让共享硬件有清晰的权限、租约与边界",
    description:
      "平台把用户、角色、会话和资源占用关系显式化，适合多团队共用同一批开发板，同时避免长期占用、误操作和不可追踪的调试行为。",
    art: "pool",
    features: [
      {
        title: "账号与登录态",
        description:
          "用户通过平台账号进入控制台，资源申请、会话创建和后续操作都与身份绑定，方便团队内协作和追踪。",
        icon: "users",
      },
      {
        title: "角色与权限控制",
        description:
          "管理员可以区分普通用户与管理权限，限制开发板配置、会话管理、DTB 文件和服务配置等敏感入口。",
        icon: "shield",
      },
      {
        title: "租约生命周期",
        description:
          "每次使用都落在明确的会话和租约中，心跳失效后自动释放资源，避免开发板被静默占用。",
        icon: "lock",
      },
      {
        title: "可审计的操作路径",
        description:
          "资源状态、会话事件、上传文件和电源操作集中呈现，问题发生时可以从平台侧还原关键上下文。",
        icon: "clipboard",
      },
    ],
  },
  {
    id: "operations",
    title: "把开发板、串口、电源和启动文件纳入日常运维",
    description:
      "统一管理开发板库存、板型配置、串口通道、TFTP 文件和电源后端，让硬件实验室不再依赖散落在个人电脑上的脚本与记录。",
    art: "workflow",
    features: [
      {
        title: "开发板库存管理",
        description:
          "维护板型、标签、连接方式和可用状态，资源页和管理台共享同一份实时库存视图。",
        icon: "folder",
      },
      {
        title: "串口会话集中接入",
        description:
          "串口路径和会话绑定由服务器管理，用户在浏览器中连接，不需要知道设备接在哪台机器上。",
        icon: "wave",
      },
      {
        title: "设备树文件管理",
        description:
          "按板型维护可用 DTB 文件，启动时选择匹配的设备树，降低手动复制和误用文件的概率。",
        icon: "circuit",
      },
      {
        title: "服务级部署与诊断",
        description:
          "服务器侧承载 API、TFTP、串口和电源管理，并提供运行配置与诊断入口，适合长期部署在实验室环境。",
        icon: "settings",
      },
    ],
  },
  {
    id: "automation",
    title: "为 CLI、脚本和 CI 留出自动化入口",
    description:
      "除了 Web 控制台，平台能力也可以被命令行和自动化流程复用，让本地构建、远程烧录、启动验证和回归测试串成稳定链路。",
    art: "workflow",
    features: [
      {
        title: "统一 API 编排",
        description:
          "资源查询、会话申请、文件上传、心跳和释放等能力都围绕 API 组织，方便工具链复用同一套调度逻辑。",
        icon: "globe",
      },
      {
        title: "衔接 ostool 工作流",
        description:
          "本地镜像构建、FIT 镜像、U-Boot 交互和远程运行可以面向同一套开发板资源池展开。",
        icon: "terminal",
      },
      {
        title: "可重复启动环境",
        description:
          "启动文件、板型参数和网络启动命令由平台维护，减少不同机器、不同成员之间的环境漂移。",
        icon: "refresh",
      },
      {
        title: "面向回归验证",
        description:
          "会话租赁和远程串口让自动化测试可以真正落到开发板上，适合做内核、驱动和系统镜像的冒烟验证。",
        icon: "check",
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
              :class="cta.primary ? 'btn btn-primary' : 'btn btn-ghost'"
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
      :class="[`home-section--${section.id}`, { 'home-section-alt': index % 2 === 1 }]"
    >
      <div class="home-section-inner">
        <header class="home-section-header home-section-header--center">
          <h3>{{ section.title }}</h3>
          <p class="home-section-lead">{{ section.description }}</p>
        </header>

        <div v-if="section.id === 'capabilities'" class="capability-showcase">
          <div class="split-media home-section-media capability-showcase-art" aria-hidden="true">
            <SectionArt :variant="section.art" class="split-art" />
            <div class="capability-chip capability-chip--top">
              <Icon name="pulse" :size="16" />
              <span>Lease-aware</span>
            </div>
            <div class="capability-chip capability-chip--bottom">
              <Icon name="power" :size="16" />
              <span>Remote power</span>
            </div>
          </div>
          <div class="home-section-content capability-copy">
            <div class="feature-list">
              <article v-for="feature in section.features" :key="feature.title" class="feature-card">
                <span class="feature-icon">
                  <Icon :name="feature.icon" :size="20" />
                </span>
                <div>
                  <h4>{{ feature.title }}</h4>
                  <p>{{ feature.description }}</p>
                </div>
              </article>
            </div>
          </div>
        </div>

        <div v-else-if="section.id === 'workflow'" class="workflow-showcase">
          <div class="workflow-heading">
            <div class="split-media home-section-media workflow-art" aria-hidden="true">
              <SectionArt :variant="section.art" class="split-art" />
            </div>
          </div>
          <div class="workflow-track">
            <article
              v-for="(feature, featureIndex) in section.features"
              :key="feature.title"
              class="feature-card workflow-card"
            >
              <span class="workflow-index">{{ String(featureIndex + 1).padStart(2, "0") }}</span>
              <span class="feature-icon">
                <Icon :name="feature.icon" :size="20" />
              </span>
              <h4>{{ feature.title }}</h4>
              <p>{{ feature.description }}</p>
            </article>
          </div>
        </div>

        <div v-else-if="section.id === 'governance'" class="governance-grid">
          <article v-for="feature in section.features" :key="feature.title" class="governance-card">
            <span class="feature-icon">
              <Icon :name="feature.icon" :size="22" />
            </span>
            <h4>{{ feature.title }}</h4>
            <p>{{ feature.description }}</p>
          </article>
        </div>

        <div v-else-if="section.id === 'operations'" class="ops-showcase">
          <div class="ops-panel" aria-hidden="true">
            <div class="ops-panel-topline">
              <span></span>
              <span></span>
              <span></span>
            </div>
            <div class="ops-board-row">
              <span>board-runner-01</span>
              <strong>leased</strong>
            </div>
            <div class="ops-board-row">
              <span>virtual-runner</span>
              <strong>ready</strong>
            </div>
            <div class="ops-board-row">
              <span>power-node-03</span>
              <strong>booting</strong>
            </div>
            <div class="ops-signal">
              <Icon name="wave" :size="18" />
              <span>serial / tftp / power channels online</span>
            </div>
          </div>
          <div class="ops-list">
            <article v-for="feature in section.features" :key="feature.title" class="ops-item">
              <span class="feature-icon">
                <Icon :name="feature.icon" :size="20" />
              </span>
              <div>
                <h4>{{ feature.title }}</h4>
                <p>{{ feature.description }}</p>
              </div>
            </article>
          </div>
        </div>

        <div v-else class="integration-strip">
          <article v-for="feature in section.features" :key="feature.title" class="integration-item">
            <span class="feature-icon">
              <Icon :name="feature.icon" :size="20" />
            </span>
            <div>
              <h4>{{ feature.title }}</h4>
              <p>{{ feature.description }}</p>
            </div>
          </article>
        </div>
      </div>
    </section>

    <section class="home-section home-section-alt home-section--steps">
      <div class="home-section-inner">
        <div class="steps-showcase">
          <header class="home-section-header home-section-header--center">
            <h3>四步即可上手一块开发板</h3>
            <p class="home-section-lead">
              从浏览资源到启动镜像，整套流程都在浏览器内完成，无需在工位之间切换。
            </p>
          </header>
          <div class="steps">
            <div v-for="step in steps" :key="step.step" class="step-card">
              <div class="step-marker">
                <span class="step-icon">
                  <Icon :name="step.icon" :size="20" />
                </span>
                <div class="step-num">{{ step.step }}</div>
              </div>
              <h4>{{ step.title }}</h4>
              <p>{{ step.description }}</p>
            </div>
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
        <RouterLink class="btn btn-primary" to="/login">
          立即登录
          <Icon name="arrow-right" :size="16" class="btn-icon" />
        </RouterLink>
        <RouterLink class="btn btn-ghost" to="/register">注册账号</RouterLink>
      </div>
    </section>
  </div>
</template>
