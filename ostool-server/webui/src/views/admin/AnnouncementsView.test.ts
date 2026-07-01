import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AnnouncementResponse } from "@/types/api";

const listAnnouncements = vi.fn();
const createAnnouncement = vi.fn();
const updateAnnouncement = vi.fn();
const deleteAnnouncement = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
  confirm: vi.fn(),
};
const authStore = {
  hasPermission: vi.fn(),
};

vi.mock("@/api", () => ({
  api: {
    admin: {
      listAnnouncements,
      createAnnouncement,
      updateAnnouncement,
      deleteAnnouncement,
    },
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("@/stores/auth", () => ({
  useAuthStore: () => authStore,
}));

function makeAnnouncement(overrides: Partial<AnnouncementResponse["announcement"]> = {}): AnnouncementResponse {
  return {
    announcement: {
      id: "ann-1",
      title: "维护通知",
      content: "今晚 22:00 进行维护",
      kind: "system",
      status: "published",
      pinned: true,
      created_by: "admin",
      updated_by: "admin",
      published_at: "2026-01-01T00:00:00Z",
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      ...overrides,
    },
  };
}

describe("AnnouncementsView", () => {
  beforeEach(() => {
    [
      listAnnouncements,
      createAnnouncement,
      updateAnnouncement,
      deleteAnnouncement,
      uiStore.clearMessages,
      uiStore.setError,
      uiStore.setSuccess,
      uiStore.confirm,
      authStore.hasPermission,
    ].forEach((fn) => fn.mockReset());
    authStore.hasPermission.mockReturnValue(true);
    uiStore.confirm.mockResolvedValue(true);
    listAnnouncements.mockResolvedValue({ announcements: [makeAnnouncement()] });
    createAnnouncement.mockResolvedValue(makeAnnouncement({ id: "ann-2", title: "活动公告", kind: "activity" }));
    updateAnnouncement.mockResolvedValue(makeAnnouncement({ title: "维护通知更新", status: "hidden" }));
    deleteAnnouncement.mockResolvedValue(undefined);
  });

  it("renders announcements and toolbar filters", async () => {
    const AnnouncementsView = (await import("./AnnouncementsView.vue")).default;
    const wrapper = mount(AnnouncementsView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("新增公告");
    expect(wrapper.find(".admin-toolbar-left").text()).not.toContain("刷新");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field")).toHaveLength(2);
    expect(wrapper.text()).toContain("维护通知");
    expect(wrapper.text()).toContain("系统公告");
    expect(wrapper.text()).toContain("已发布");
    expect(wrapper.text()).toContain("是");
  });

  it("creates an announcement", async () => {
    const AnnouncementsView = (await import("./AnnouncementsView.vue")).default;
    const wrapper = mount(AnnouncementsView);
    await flushPromises();

    await wrapper.find(".admin-toolbar-left .btn-primary").trigger("click");
    await flushPromises();
    const selects = wrapper.findAll(".modal-card select");
    await selects[0].setValue("activity");
    await selects[1].setValue("published");
    await wrapper.find(".modal-card input[type='text']").setValue("活动公告");
    await wrapper.find(".modal-card input[type='checkbox']").setValue(true);
    await wrapper.find(".modal-card textarea").setValue("开放新一批开发板资源");
    await wrapper.find(".modal-card form").trigger("submit");
    await flushPromises();

    expect(createAnnouncement).toHaveBeenCalledWith({
      title: "活动公告",
      content: "开放新一批开发板资源",
      kind: "activity",
      status: "published",
      pinned: true,
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("公告已创建");
  });

  it("deletes an announcement", async () => {
    const AnnouncementsView = (await import("./AnnouncementsView.vue")).default;
    const wrapper = mount(AnnouncementsView);
    await flushPromises();

    await wrapper.find('button[title="删除公告"]').trigger("click");
    await flushPromises();

    expect(deleteAnnouncement).toHaveBeenCalledWith("ann-1");
    expect(uiStore.setSuccess).toHaveBeenCalledWith("公告已删除");
    expect(wrapper.text()).toContain("暂无公告数据");
  });
});
