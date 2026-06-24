import { computed, ref } from "vue";

import { api } from "@/api";
import { useUiStore } from "@/stores/ui";
import type { LeaseResponse } from "@/types/api";

export function formatLeaseTime(iso: string): string {
  const parsed = Date.parse(iso);
  if (!Number.isFinite(parsed)) {
    return iso;
  }
  return new Date(parsed).toLocaleString();
}

export function remainingLeaseLabel(iso: string): string {
  const parsed = Date.parse(iso);
  if (!Number.isFinite(parsed)) {
    return "-";
  }
  const remaining = parsed - Date.now();
  if (remaining <= 0) {
    return "已过期";
  }
  const minutes = Math.floor(remaining / 60000);
  if (minutes < 60) {
    return `${minutes} 分钟`;
  }
  const hours = Math.floor(minutes / 60);
  return `${hours} 小时 ${minutes % 60} 分钟`;
}

export function useUserLeases() {
  const ui = useUiStore();
  const leases = ref<LeaseResponse[]>([]);
  const loading = ref(true);

  const activeLeases = computed(() =>
    leases.value.filter((item) => item.lease.state === "active"),
  );
  const activeSessions = computed(() =>
    leases.value.filter((item) => item.lease.state === "active" && item.session),
  );

  async function loadLeases() {
    loading.value = true;
    try {
      const leaseList = await api.listUserLeases();
      leases.value = leaseList.leases;
    } catch (error) {
      ui.setError((error as Error).message);
    } finally {
      loading.value = false;
    }
  }

  async function releaseLease(leaseId: string) {
    const confirmed = await ui.confirm({
      tone: "danger",
      title: "释放租赁",
      message: `确认释放租赁 ${leaseId}？相关开发板将归还到资源池。`,
      confirmLabel: "释放",
    });
    if (!confirmed) {
      return;
    }
    try {
      await api.deleteLease(leaseId);
      ui.setSuccess(`已释放租赁 ${leaseId}`);
      await loadLeases();
    } catch (error) {
      ui.setError((error as Error).message);
    }
  }

  return {
    leases,
    loading,
    activeLeases,
    activeSessions,
    loadLeases,
    releaseLease,
  };
}
