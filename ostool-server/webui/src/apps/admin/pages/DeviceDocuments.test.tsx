import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import DeviceDocuments from "./DeviceDocuments";
const mocks = vi.hoisted(() => ({
  select: vi.fn(),
  save: vi.fn(),
  view: {
    id: "board",
    revision: 1,
    document: {
      title: "Original",
      summary: "",
      description: "",
      image_url: "",
      highlights: [],
      specifications: [],
    },
  },
}));
vi.mock("@/api/events", () => ({
  admin: { setDeviceDocument: mocks.select },
  useList: () => [],
  useResource: (topic: string) =>
    topic === "device_document" ? mocks.view : {},
  useErrors: () => ({}),
}));
vi.mock("@/api/client", () => ({ api: { saveDeviceDocument: mocks.save } }));
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  mocks.view = {
    ...mocks.view,
    revision: 1,
    document: { ...mocks.view.document, title: "Original" },
  };
  mocks.save.mockResolvedValue({ ...mocks.view, revision: 2 });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});
const App = () => (
  <MemoryRouter initialEntries={["/device-documents/board"]}>
    <Routes>
      <Route path="/device-documents/:boardId" element={<DeviceDocuments />} />
    </Routes>
  </MemoryRouter>
);
it("publishes only public fields and preserves the draft when a later revision arrives", async () => {
  const user = userEvent.setup();
  const app = render(<App />);
  const title = screen.getByLabelText("公开标题");
  await user.clear(title);
  await user.type(title, "My public title");
  await user.click(screen.getByRole("button", { name: "保存并发布" }));
  await waitFor(() =>
    expect(mocks.save).toHaveBeenCalledWith("board", 1, {
      title: "My public title",
      summary: "",
      description: "",
      image_url: "",
      highlights: [],
      specifications: [],
    }),
  );
  expect(screen.queryByText(/文档已被其他管理员更新/)).toBeNull();
  mocks.view = {
    ...mocks.view,
    revision: 3,
    document: { ...mocks.view.document, title: "Remote title" },
  };
  app.rerender(<App />);
  expect(screen.getByText(/文档已被其他管理员更新/)).toBeTruthy();
  expect((title as HTMLInputElement).value).toBe("My public title");
  expect(
    (screen.getByRole("button", { name: "保存并发布" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "载入最新文档" }));
  expect((title as HTMLInputElement).value).toBe("Remote title");
});
