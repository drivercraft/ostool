import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listAnnouncements = vi.fn();

vi.mock("@/api", () => ({
  api: {
    public: {
      listAnnouncements,
    },
  },
}));

describe("AnnouncementBar", () => {
  beforeEach(() => {
    listAnnouncements.mockReset();
  });

  it("renders published announcements and expands the list", async () => {
    listAnnouncements.mockResolvedValue({
      announcements: [
        {
          announcement: {
            id: "ann-1",
            title: "维护通知",
            content: "今晚维护",
            kind: "system",
            status: "published",
            pinned: true,
            created_by: null,
            updated_by: null,
            published_at: "2026-01-01T00:00:00Z",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
          },
        },
        {
          announcement: {
            id: "ann-2",
            title: "活动公告",
            content: "新增资源",
            kind: "activity",
            status: "published",
            pinned: false,
            created_by: null,
            updated_by: null,
            published_at: "2026-01-02T00:00:00Z",
            created_at: "2026-01-02T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
          },
        },
      ],
    });
    const AnnouncementBar = (await import("./AnnouncementBar.vue")).default;
    const wrapper = mount(AnnouncementBar);
    await flushPromises();

    expect(wrapper.text()).toContain("维护通知");
    expect(wrapper.text()).toContain("系统公告");
    expect(wrapper.text()).not.toContain("活动公告");

    await wrapper.find(".announcement-toggle").trigger("click");
    expect(wrapper.text()).toContain("活动公告");
    expect(wrapper.text()).toContain("活动公告");
  });

  it("does not render when there are no announcements", async () => {
    listAnnouncements.mockResolvedValue({ announcements: [] });
    const AnnouncementBar = (await import("./AnnouncementBar.vue")).default;
    const wrapper = mount(AnnouncementBar);
    await flushPromises();

    expect(wrapper.find(".announcement-banner").exists()).toBe(false);
  });
});
