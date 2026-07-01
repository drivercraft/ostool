import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AdminUserResponse, IssueSessionResponse } from "@/types/api";

const listIssueSessions = vi.fn();
const updateIssueSession = vi.fn();
const deleteIssueSession = vi.fn();
const listAdminUsers = vi.fn();
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
      listIssueSessions,
      updateIssueSession,
      deleteIssueSession,
      listAdminUsers,
    },
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("@/stores/auth", () => ({
  useAuthStore: () => authStore,
}));

function makeUser(): AdminUserResponse {
  return {
    id: "user-1",
    username: "alice",
    display_name: "Alice",
    nickname: null,
    avatar_url: null,
    email: "alice@example.com",
    phone: null,
    department: null,
    title: null,
    disabled: false,
    status: "active",
    last_login_at: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

function makeIssue(overrides: Partial<IssueSessionResponse["issue"]> = {}): IssueSessionResponse {
  return {
    issue: {
      id: "issue-1",
      user_id: "user-1",
      lease_id: "lease-1",
      session_id: "session-1",
      title: "串口无法连接",
      category: "serial",
      description: "串口控制台没有输出",
      state: "open",
      priority: "high",
      handler_user_id: null,
      resolution: null,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      resolved_at: null,
      ...overrides,
    },
  };
}

describe("IssueSessionsView", () => {
  beforeEach(() => {
    listIssueSessions.mockReset();
    updateIssueSession.mockReset();
    deleteIssueSession.mockReset();
    listAdminUsers.mockReset();
    uiStore.clearMessages.mockReset();
    uiStore.setError.mockReset();
    uiStore.setSuccess.mockReset();
    uiStore.confirm.mockReset();
    authStore.hasPermission.mockReset();
    authStore.hasPermission.mockReturnValue(true);
    uiStore.confirm.mockResolvedValue(true);
    listIssueSessions.mockResolvedValue({ issues: [makeIssue()] });
    listAdminUsers.mockResolvedValue({ users: [makeUser()] });
    updateIssueSession.mockResolvedValue(makeIssue({ state: "resolved", resolution: "已重启串口服务" }));
    deleteIssueSession.mockResolvedValue(undefined);
  });

  it("renders issue sessions with filters on the right", async () => {
    const IssueSessionsView = (await import("./IssueSessionsView.vue")).default;
    const wrapper = mount(IssueSessionsView);
    await flushPromises();

    expect(wrapper.find(".admin-toolbar-left").text()).toContain("刷新");
    expect(wrapper.find(".admin-toolbar-right .search-field").exists()).toBe(true);
    expect(wrapper.findAll(".admin-toolbar-right .filter-field")).toHaveLength(2);
    expect(wrapper.text()).toContain("串口无法连接");
    expect(wrapper.text()).toContain("Alice");
    expect(wrapper.text()).toContain("高");
    expect(wrapper.text()).toContain("待处理");
  });

  it("updates an issue session", async () => {
    const IssueSessionsView = (await import("./IssueSessionsView.vue")).default;
    const wrapper = mount(IssueSessionsView);
    await flushPromises();

    await wrapper.find('button[title="处理"]').trigger("click");
    await flushPromises();
    await wrapper.findAll("select")[2].setValue("resolved");
    await wrapper.findAll("select")[3].setValue("normal");
    await wrapper.find("textarea").setValue("已重启串口服务");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(updateIssueSession).toHaveBeenCalledWith("issue-1", {
      state: "resolved",
      priority: "normal",
      resolution: "已重启串口服务",
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已更新问题会话 issue-1");
  });

  it("deletes an issue session and refreshes the list", async () => {
    listIssueSessions
      .mockResolvedValueOnce({ issues: [makeIssue()] })
      .mockResolvedValueOnce({ issues: [] });
    const IssueSessionsView = (await import("./IssueSessionsView.vue")).default;
    const wrapper = mount(IssueSessionsView);
    await flushPromises();

    await wrapper.find('button[title="删除"]').trigger("click");
    await flushPromises();

    expect(deleteIssueSession).toHaveBeenCalledWith("issue-1");
    expect(uiStore.setSuccess).toHaveBeenCalledWith("已删除问题会话 issue-1");
    expect(listIssueSessions).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("暂无问题会话数据");
  });
});
