import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import DeviceDetail from "./DeviceDetail";
const mocks = vi.hoisted(() => ({
  select: vi.fn(),
  devices: [
    { id: "arm-board", board_type: "arm64", tags: ["lab"], disabled: false },
  ],
  view: {
    id: "arm-board",
    revision: 1,
    document: {
      title: "Explorer",
      summary: "For your kernel",
      description: "<script>private code must stay text</script>",
      image_url: "",
      highlights: ["Serial console"],
      specifications: [{ name: "Memory", value: "8 GiB" }],
    },
  },
}));
vi.mock("@/api/events", () => ({
  site: { setDeviceDocument: mocks.select },
  useDeviceDocument: () => mocks.view,
  usePublicDevices: () => mocks.devices,
}));
vi.mock("@/edition", () => ({ useEdition: () => ({ enabled: true }) }));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
it("presents only authored information and links to the selected board's application", () => {
  render(
    <MemoryRouter initialEntries={["/devices/arm-board"]}>
      <Routes>
        <Route path="/devices/:boardId" element={<DeviceDetail />} />
      </Routes>
    </MemoryRouter>,
  );
  expect(
    screen.getByRole("heading", { level: 1, name: "Explorer" }),
  ).toBeTruthy();
  expect(screen.getByText("8 GiB")).toBeTruthy();
  expect(
    screen.getByText("<script>private code must stay text</script>"),
  ).toBeTruthy();
  expect(document.querySelector("script")).toBeNull();
  for (const link of screen.getAllByRole("link", { name: "申请租赁" }))
    expect(link.getAttribute("href")).toBe("/rental-request?board=arm-board");
  expect(mocks.select).toHaveBeenCalledWith("arm-board");
});
it("reports a removed board rather than inventing its specification", () => {
  render(
    <MemoryRouter initialEntries={["/devices/missing"]}>
      <Routes>
        <Route path="/devices/:boardId" element={<DeviceDetail />} />
      </Routes>
    </MemoryRouter>,
  );
  expect(screen.getByText(/该开发板不存在/)).toBeTruthy();
  expect(screen.queryByText("8 GiB")).toBeNull();
});
