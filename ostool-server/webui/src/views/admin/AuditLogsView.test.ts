import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminAuditLogResponse } from "@/types/api";

const listAuditLogs = vi.fn();

const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    admin: {
      listAuditLogs,
    },
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

function makeAuditLog(overrides: Partial<AdminAuditLogResponse> = {}): AdminAuditLogResponse {
  return {
    id: "audit-1",
    actor_user_id: "user-1",
    actor_username: "alice",
    action: "dtbs.create",
    target_type: "dtbs",
    target_id: "demo.dtb",
    outcome: "success",
    ip_address: "127.0.0.1",
    user_agent: "Vitest",
    request_id: "req-1",
    metadata: { name: "demo.dtb", size_bytes: 42 },
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

describe("AuditLogsView", () => {
  beforeEach(() => {
    listAuditLogs.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    listAuditLogs.mockResolvedValue({
      logs: [
        makeAuditLog(),
        makeAuditLog({
          id: "audit-2",
          actor_username: "bob",
          action: "users.reject",
          target_type: "users",
          target_id: "user-2",
          ip_address: "10.0.0.2",
          metadata: { username: "bob" },
        }),
      ],
    });
  });

  it("loads audit logs and renders filters and columns", async () => {
    const AuditLogsView = (await import("./AuditLogsView.vue")).default;
    const wrapper = mount(AuditLogsView);

    await flushPromises();

    expect(listAuditLogs).toHaveBeenCalledWith();
    expect(wrapper.find(".admin-toolbar-left").text()).toContain("刷新");
    expect(wrapper.find(".admin-toolbar-left").text()).toContain("清空");
    expect(wrapper.find(".admin-toolbar-left .search-field").exists()).toBe(false);
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.find(".admin-toolbar-right").element.firstElementChild?.classList.contains("search-field")).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field")).toHaveLength(2);
    expect(wrapper.findAll("th").map((header) => header.text())).toEqual([
      "序号",
      "时间",
      "操作",
      "对象",
      "结果",
      "操作人",
      "IP",
      "浏览器",
    ]);
    expect(wrapper.text()).toContain("上传 DTB");
    expect(wrapper.text()).toContain("DTB 文件");
    expect(wrapper.text()).toContain("alice");
  });

  it("filters by action and expands metadata details", async () => {
    const AuditLogsView = (await import("./AuditLogsView.vue")).default;
    const wrapper = mount(AuditLogsView);

    await flushPromises();

    const actionSelect = wrapper.findAll(".filter-field select")[0];
    await actionSelect.setValue("users.reject");
    await flushPromises();

    const rows = wrapper.findAll("tbody tr.audit-row");
    expect(rows).toHaveLength(1);
    expect(rows[0].text()).toContain("拒绝注册申请");
    expect(rows[0].text()).not.toContain("上传 DTB");

    await rows[0].trigger("click");
    await flushPromises();

    expect(wrapper.find(".audit-detail-row").exists()).toBe(true);
    expect(wrapper.find(".audit-detail-pre").text()).toContain('"username": "bob"');
  });
});
