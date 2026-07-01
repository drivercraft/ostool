import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { IssueSessionResponse } from "@/types/api";

const listIssueSessions = vi.fn();
const createIssueSession = vi.fn();
const uiStore = {
  clearMessages: vi.fn(),
  setError: vi.fn(),
  setSuccess: vi.fn(),
};
const routerReplace = vi.fn();

vi.mock("@/api", () => ({
  api: {
    user: {
      listIssueSessions,
      createIssueSession,
    },
  },
}));

vi.mock("@/stores/ui", () => ({
  useUiStore: () => uiStore,
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ replace: routerReplace }),
}));

async function seedUser() {
  const { useAuthStore } = await import("@/stores/auth");
  const store = useAuthStore();
  store.user = {
    id: "user-demo",
    username: "demo",
    display_name: "Demo",
    nickname: null,
    avatar_url: null,
    email: "demo@ostool.local",
    phone: null,
    department: null,
    title: null,
    last_login_at: null,
    roles: [],
    permissions: [],
  };
  store.loaded = true;
}

function makeIssue(overrides: Partial<IssueSessionResponse["issue"]> = {}): IssueSessionResponse {
  return {
    issue: {
      id: "issue-1",
      user_id: "user-demo",
      lease_id: "lease-1",
      session_id: "session-1",
      title: "串口无法连接",
      category: "session",
      description: "点击连接后一直超时",
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

describe("IssueFeedbackView", () => {
  beforeEach(() => {
    [
      listIssueSessions,
      createIssueSession,
      uiStore.clearMessages,
      uiStore.setError,
      uiStore.setSuccess,
      routerReplace,
    ].forEach((fn) => fn.mockReset());
    listIssueSessions.mockResolvedValue({ issues: [makeIssue()] });
    createIssueSession.mockResolvedValue(makeIssue({
      id: "issue-2",
      title: "开发板无法上电",
      category: "resource",
      description: "电源操作无响应",
      priority: "urgent",
      lease_id: null,
      session_id: null,
    }));
  });

  it("renders feedback history", async () => {
    await seedUser();
    const IssueFeedbackView = (await import("./IssueFeedbackView.vue")).default;
    const wrapper = mount(IssueFeedbackView);
    await flushPromises();

    expect(wrapper.text()).toContain("提交反馈");
    expect(wrapper.text()).toContain("我的反馈");
    expect(wrapper.text()).toContain("串口无法连接");
    expect(wrapper.text()).toContain("会话问题");
    expect(wrapper.text()).toContain("待处理");
  });

  it("submits a feedback issue", async () => {
    await seedUser();
    const IssueFeedbackView = (await import("./IssueFeedbackView.vue")).default;
    const wrapper = mount(IssueFeedbackView);
    await flushPromises();

    const inputs = wrapper.findAll("input");
    await inputs[0].setValue("开发板无法上电");
    await inputs[1].setValue("lease-2");
    await inputs[2].setValue("session-2");
    const selects = wrapper.findAll("select");
    await selects[0].setValue("resource");
    await selects[1].setValue("urgent");
    await wrapper.find("textarea").setValue("电源操作无响应");
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(createIssueSession).toHaveBeenCalledWith({
      title: "开发板无法上电",
      category: "resource",
      priority: "urgent",
      lease_id: "lease-2",
      session_id: "session-2",
      description: "电源操作无响应",
    });
    expect(uiStore.setSuccess).toHaveBeenCalledWith("问题反馈已提交");
    expect(wrapper.text()).toContain("开发板无法上电");
  });
});
