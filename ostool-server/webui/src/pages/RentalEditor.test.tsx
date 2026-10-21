import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import RentalEditor from "./RentalEditor";
import { localDateTime } from "@/utils/rental-calendar";
const mocks = vi.hoisted(() => ({
  resources: {} as Record<string, unknown>,
  createRental: vi.fn(),
  updateRental: vi.fn(),
}));
vi.mock("@/api/events", () => ({
  useResource: (topic: string) => mocks.resources[topic],
}));
vi.mock("@/api/client", () => ({
  api: { createRental: mocks.createRental, updateRental: mocks.updateRental },
}));
beforeEach(() => {
  window.history.replaceState({}, "", "/admin/rentals/new");
  mocks.resources = {
    users: [
      {
        id: "u1",
        email: "user@test.local",
        display_name: "User",
        disabled: false,
      },
    ],
    boards: [{ id: "board-one", board_type: "fixture", disabled: false }],
    rentals: [],
  };
});
afterEach(cleanup);
const App = () => (
  <BrowserRouter basename="/admin">
    <Routes>
      <Route path="/rentals/new" element={<RentalEditor />} />
    </Routes>
  </BrowserRouter>
);
it("uses a full page and updates calendar conflicts without overwriting the editor draft", async () => {
  const user = userEvent.setup();
  const app = render(<App />);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.getByRole("region", { name: "租赁日历" })).toBeTruthy();
  const start = new Date();
  start.setDate(start.getDate() + 2);
  start.setHours(9, 0, 0, 0);
  const end = new Date(start.getTime() + 3600000);
  fireEvent.change(screen.getByLabelText("开始时间"), {
    target: { value: localDateTime(start) },
  });
  fireEvent.change(screen.getByLabelText("结束时间"), {
    target: { value: localDateTime(end) },
  });
  await user.type(screen.getByLabelText("备注"), "Keep my draft");
  expect(
    (screen.getByRole("button", { name: "保存租赁" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  mocks.resources.rentals = [
    {
      id: "other",
      board_id: "board-one",
      user_id: "u1",
      starts_at: start.toISOString(),
      ends_at: end.toISOString(),
      status: "scheduled",
      notes: "another booking",
    },
  ];
  app.rerender(<App />);
  expect((screen.getByLabelText("备注") as HTMLTextAreaElement).value).toBe(
    "Keep my draft",
  );
  expect(
    (screen.getByRole("button", { name: "保存租赁" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  expect(screen.getAllByText(/已有租赁|租赁冲突/).length).toBeGreaterThan(0);
});
