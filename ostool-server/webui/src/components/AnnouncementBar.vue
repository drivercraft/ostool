<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

import { api } from "@/api";
import Icon from "@/components/Icon.vue";
import type { AnnouncementResponse } from "@/types/api";
import { getAnnouncementKindLabel } from "@/utils/announcement";

const loading = ref(true);
const expanded = ref(false);
const announcements = ref<AnnouncementResponse[]>([]);

const visibleAnnouncements = computed(() =>
  expanded.value ? announcements.value : announcements.value.slice(0, 1),
);

function formatDate(value: string | null) {
  return value
    ? new Date(value).toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" })
    : "";
}

async function loadAnnouncements() {
  loading.value = true;
  try {
    const response = await api.public.listAnnouncements();
    announcements.value = response.announcements;
  } catch {
    announcements.value = [];
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  void loadAnnouncements();
});
</script>

<template>
  <section v-if="!loading && announcements.length > 0" class="announcement-banner" aria-label="平台公告">
    <span class="announcement-icon"><Icon name="bell" :size="16" /></span>
    <div class="announcement-list">
      <article
        v-for="item in visibleAnnouncements"
        :key="item.announcement.id"
        class="announcement-item"
        :class="{ 'is-pinned': item.announcement.pinned }"
      >
        <div class="announcement-title">
          <strong>{{ item.announcement.title }}</strong>
          <span>{{ getAnnouncementKindLabel(item.announcement.kind) }}</span>
          <span v-if="item.announcement.pinned">置顶</span>
          <span>{{ formatDate(item.announcement.published_at || item.announcement.created_at) }}</span>
        </div>
        <p>{{ item.announcement.content }}</p>
      </article>
    </div>
    <button
      v-if="announcements.length > 1"
      class="announcement-toggle"
      type="button"
      @click="expanded = !expanded"
    >
      {{ expanded ? "收起" : `展开 ${announcements.length} 条` }}
    </button>
  </section>
</template>
