import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ListLayoutProvider, ListLayoutToggle } from "./list-layout";
import { DataTable, Row, Cell } from "./forms";
afterEach(() => {
  cleanup();
  localStorage.clear();
});
it("switches to labeled cards, preserves row actions and restores the selection", async () => {
  const action = vi.fn(),
    user = userEvent.setup();
  const App = () => (
    <ListLayoutProvider storageKey="test-list">
      <ListLayoutToggle />
      <DataTable headers={["开发板", "状态", "操作"]}>
        <Row>
          <Cell>board-one</Cell>
          <Cell>空闲</Cell>
          <Cell>
            <button onClick={action}>编辑</button>
          </Cell>
        </Row>
      </DataTable>
    </ListLayoutProvider>
  );
  const app = render(<App />);
  await user.click(screen.getByRole("button", { name: "切换列表布局" }));
  await user.click(screen.getByRole("menuitemradio", { name: /卡片视图/ }));
  expect(screen.getByRole("list", { name: "卡片列表" })).toBeTruthy();
  expect(screen.getByText("空闲")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "编辑" }));
  expect(action).toHaveBeenCalledTimes(1);
  app.unmount();
  render(<App />);
  expect(screen.getByRole("list", { name: "卡片列表" })).toBeTruthy();
});
