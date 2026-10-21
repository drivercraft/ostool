import { afterEach, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useListPage } from "./list-pagination";

function Harness({ items, filter = "" }: { items: string[]; filter?: string }) {
  const pagination = useListPage(items, filter);
  return (
    <>
      <ul>
        {pagination.rows.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
      {pagination.footer}
    </>
  );
}
const items = Array.from({ length: 45 }, (_, i) => `row-${i + 1}`);
afterEach(cleanup);
it("keeps the current page on a live update, resets when filters change, and clamps after deletion", async () => {
  const user = userEvent.setup();
  const view = render(<Harness items={items} />);
  await user.click(screen.getByRole("button", { name: "下一页" }));
  expect(screen.getByText("row-21")).toBeTruthy();
  view.rerender(<Harness items={[...items, "row-46"]} />);
  expect(screen.getByText("第 2 / 3 页")).toBeTruthy();
  view.rerender(<Harness items={items.slice(0, 5)} />);
  expect(screen.getByText("第 1 / 1 页")).toBeTruthy();
  view.rerender(<Harness items={items} />);
  expect(screen.getByText("第 1 / 3 页")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "下一页" }));
  view.rerender(<Harness items={items} filter="new filter" />);
  expect(screen.getByText("第 1 / 3 页")).toBeTruthy();
});
it("supports page-size changes and a stable empty footer", async () => {
  const user = userEvent.setup();
  const view = render(<Harness items={items} />);
  await user.selectOptions(
    screen.getByRole("combobox", { name: "每页条数" }),
    "50",
  );
  expect(screen.getAllByRole("listitem")).toHaveLength(45);
  expect(
    (screen.getByRole("button", { name: "下一页" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  view.rerender(<Harness items={[]} />);
  expect(screen.getByText("第 1 / 1 页")).toBeTruthy();
  expect(screen.queryAllByRole("listitem")).toHaveLength(0);
  expect(
    (screen.getByRole("button", { name: "上一页" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});
