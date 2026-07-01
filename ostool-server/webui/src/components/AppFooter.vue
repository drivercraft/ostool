<script setup lang="ts">
import { RouterLink } from "vue-router";

import Icon from "@/components/Icon.vue";
</script>

<template>
  <footer class="public-footer">
    <div class="public-footer-inner">
      <div class="footer-brand">
        <span class="brand-mark"><Icon name="circuit" :size="18" /></span>
        <div>
          <strong>ostool</strong>
          <span>开发板租赁平台</span>
        </div>
      </div>
      <nav class="footer-nav">
        <RouterLink to="/">首页</RouterLink>
        <RouterLink to="/resources">可用资源</RouterLink>
        <RouterLink to="/docs">使用文档</RouterLink>
        <RouterLink to="/terms">用户协议</RouterLink>
        <RouterLink to="/privacy">隐私政策</RouterLink>
        <RouterLink to="/login">登录</RouterLink>
      </nav>
      <span class="footer-copy">© ostool 开发板租赁平台</span>
    </div>
  </footer>
</template>
