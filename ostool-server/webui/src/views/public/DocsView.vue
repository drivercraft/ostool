<script setup lang="ts">
import { computed, ref } from "vue";

import Icon, { type IconName } from "@/components/Icon.vue";

interface DocSection {
  id: string;
  title: string;
  summary: string;
  body: string;
}

interface DocGroup {
  id: string;
  title: string;
  description: string;
  icon: IconName;
  sections: DocSection[];
}

const groups: DocGroup[] = [
  {
    id: "getting-started",
    title: "快速上手",
    description: "第一次使用平台时建议先阅读的内容。",
    icon: "sparkles",
    sections: [
      {
        id: "overview",
        title: "平台能做什么",
        summary: "了解 ostool 开发板租赁平台提供的能力。",
        body: `ostool-server 提供开发板池管理、远程串口、TFTP / HTTP Boot 与电源编排能力。
你在浏览器中即可完成开发板的申请、租用、上下电与调试，无需在工位之间来回切换。

核心能力包括：
- 按板型与标签筛选可租赁的开发板
- 创建会话后自动获得专属串口通道（基于 WebSocket）
- 内置 TFTP 与 UEFI HTTP Boot，支持上传内核、DTB 与 ramfs
- 支持自定义电源命令与中盛继电器，按需上下电`,
      },
      {
        id: "first-session",
        title: "创建第一个会话",
        summary: "登录后如何申请一块开发板并连接串口。",
        body: `1. 在“资源”页面确认目标开发板型号仍有可用数量。
2. 点击“去申请会话”或进入“用户控制台”选择型号并提交。
3. 平台会自动从池中分配一块空闲开发板，并返回会话 ID 与租约过期时间。
4. 在会话详情中可以使用串口终端、上传启动镜像、查看 TFTP 文件列表。
5. 会话到期或主动结束时，平台会自动释放开发板并清理临时文件。`,
      },
      {
        id: "session-lifecycle",
        title: "会话租约与心跳",
        summary: "会话有效期、续租策略与自动释放。",
        body: `每个会话都有租约过期时间（lease_expires_at）。客户端可以通过
/api/v1/sessions/{id}/heartbeat 接口续租；若长时间未心跳，会话会被自动释放，
绑定的开发板也会重新进入可用池。主动调用 DELETE 接口可以立即结束会话。`,
      },
    ],
  },
  {
    id: "boot-modes",
    title: "启动模式",
    description: "平台支持的开发板启动方式及配置要点。",
    icon: "bolt",
    sections: [
      {
        id: "uboot",
        title: "U-Boot + TFTP",
        summary: "通过 TFTP 拉取内核、DTB 与 ramfs 启动。",
        body: `适用于 U-Boot 引导的开发板。平台会根据板型生成 bootcmd，
自动从 TFTP 拉取镜像到指定地址后 bootm。配置时可以指定：
- kernel_load_addr / fit_load_addr / bootm_addr：加载与启动地址
- dtb_name：使用平台托管的 DTB 文件
- network_mode：DHCP 或静态 IP（board_ip / server_ip / netmask / gatewayip）`,
      },
      {
        id: "pxe",
        title: "PXE 启动",
        summary: "通过 PXE 协议进行网络启动。",
        body: `适用于支持 PXE 的网卡。平台会在会话作用域内提供 PXE 配置，
开发者只需上传对应内核与启动配置即可。`,
      },
      {
        id: "httpboot",
        title: "UEFI HTTP Boot",
        summary: "通过 UEFI HTTP Boot 加载 axloader 与镜像。",
        body: `适用于支持 UEFI HTTP Boot 的设备。上传的启动镜像复用会话级文件存储，
并在会话结束时一并清理。可按需指定 boot_arch 来区分镜像架构。`,
      },
    ],
  },
  {
    id: "files-and-serial",
    title: "文件与串口",
    description: "如何上传启动文件、查看 TFTP 状态以及连接串口。",
    icon: "terminal",
    sections: [
      {
        id: "session-files",
        title: "会话级文件",
        summary: "上传与下载会话作用域内的启动文件。",
        body: `每个会话都会获得独立的文件作用域。通过 PUT /api/v1/sessions/{id}/files/{path}
可以上传文件；通过 GET 接口可以下载或列出文件。会话结束时文件会被自动清理。`,
      },
      {
        id: "tftp-status",
        title: "TFTP 诊断",
        summary: "排查 TFTP 无法拉取镜像的问题。",
        body: `在会话详情或管理台可以查看 TFTP 状态，包括：
- provider：内置或 systemd tftpd-hpa
- resolved_server_ip / resolved_netmask：实际提供给开发板的地址
- healthy / writable：服务健康度与可写性
- last_error：最近一次错误信息`,
      },
      {
        id: "serial-terminal",
        title: "串口终端",
        summary: "通过 WebSocket 连接开发板串口。",
        body: `会话分配的开发板如果带有串口配置，平台会返回 ws_url。
客户端可以使用任意 WebSocket 终端连接该地址进行交互。ostool CLI 也提供
sterm 命令封装，可以直接通过会话 ID 启动终端。`,
      },
    ],
  },
  {
    id: "admin",
    title: "管理员操作",
    description: "面向管理员的开发板池、DTB、电源与会话管理。",
    icon: "shield",
    sections: [
      {
        id: "board-pool",
        title: "开发板池管理",
        summary: "新增、编辑、禁用开发板。",
        body: `管理员可以在“开发板”页面新增或编辑开发板，配置板型、标签、串口主键、
电源管理方式与启动配置。禁用的开发板不会进入可分配池。`,
      },
      {
        id: "power-management",
        title: "电源编排",
        summary: "自定义命令与中盛继电器。",
        body: `电源管理支持两种模式：
- custom：使用自定义的 power_on_cmd / power_off_cmd
- zhongsheng_relay：通过中盛数字量输入输出模块控制继电器`,
      },
      {
        id: "server-config",
        title: "Server 配置",
        summary: "调整 TFTP 网络接口与上传限制。",
        body: `在“Server 配置”页面可以调整 TFTP 绑定的网卡接口以及会话文件大小上限。
监听地址、数据目录、DTB 上传上限等只读字段仅作为参考展示。`,
      },
    ],
  },
];

const activeId = ref(groups[0]?.sections[0]?.id ?? "");

const flatSections = computed(() => groups.flatMap((group) => group.sections));

const activeSection = computed(
  () => flatSections.value.find((section) => section.id === activeId.value) ?? null,
);

function selectSection(id: string) {
  activeId.value = id;
}
</script>

<template>
  <div class="page-body public-page-body">
    <header class="public-page-header">
      <p class="eyebrow">文档中心</p>
      <h2>使用说明与常见操作</h2>
      <p class="public-page-subtitle">
        本页面汇总了 ostool 平台的核心使用说明，更多内容会随版本迭代持续更新。
      </p>
    </header>

    <div class="docs-layout">
      <aside class="docs-toc" aria-label="文档目录">
        <div v-for="group in groups" :key="group.id" class="docs-toc-group">
          <div class="docs-toc-group-head">
            <span class="docs-toc-group-icon"><Icon :name="group.icon" :size="16" /></span>
            <h4>{{ group.title }}</h4>
          </div>
          <p class="docs-toc-group-desc">{{ group.description }}</p>
          <ul>
            <li v-for="section in group.sections" :key="section.id">
              <button
                type="button"
                class="docs-toc-link"
                :class="{ 'is-active': section.id === activeId }"
                @click="selectSection(section.id)"
              >
                {{ section.title }}
              </button>
            </li>
          </ul>
        </div>
      </aside>

      <article v-if="activeSection" class="docs-content">
        <header class="docs-content-header">
          <p class="eyebrow">{{ activeSection.summary }}</p>
          <h3>{{ activeSection.title }}</h3>
        </header>
        <pre class="docs-pre">{{ activeSection.body }}</pre>
      </article>
    </div>
  </div>
</template>
