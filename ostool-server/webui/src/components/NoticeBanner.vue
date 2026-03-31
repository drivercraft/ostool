<script setup lang="ts">
defineProps<{
  tone: "success" | "error";
  message: string;
}>();
</script>

<template>
  <div class="notice" :data-tone="tone">
    {{ message }}
  </div>
</template>
