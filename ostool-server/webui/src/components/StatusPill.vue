<script setup lang="ts">
defineProps<{
  tone: "good" | "warn" | "danger" | "neutral";
  label: string;
}>();
</script>

<template>
  <span class="status-pill" :data-tone="tone">{{ label }}</span>
</template>
