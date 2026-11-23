import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import { Copy, Trash2 } from "lucide-react";
import { ListActions } from "./list-actions";
import { RowAction } from "./row-action";
import { ConfirmAction } from "./forms";
import { PermissionsProvider } from "./permissions";

afterEach(cleanup);

it("opens row menus and keeps delete confirmation mounted when selecting delete", async () => {
  const remove = vi.fn().mockResolvedValue(undefined);
  render(
    <MemoryRouter>
      <ListActions>
        <RowAction label="基于此新建" icon={Copy} to="/boards/new?copy=one" />
        <ConfirmAction
          label="删除"
          icon={Trash2}
          description="删除 one？"
          action={remove}
        />
      </ListActions>
    </MemoryRouter>,
  );
  const user = userEvent.setup();
  expect(
    screen.queryByRole("button", { name: "修改" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "禁用" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("menuitem", { name: "删除" }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "更多" }));
  expect(screen.getByRole("menuitem", { name: "基于此新建" })).toHaveAttribute(
    "href",
    "/boards/new?copy=one",
  );
  await user.click(screen.getByRole("menuitem", { name: "删除" }));
  expect(screen.getByRole("dialog", { name: "删除" })).toBeVisible();
  expect(remove).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "确认" }));
  expect(remove).toHaveBeenCalledOnce();
});

it("hides menu mutations when the user lacks permission", async () => {
  render(
    <PermissionsProvider local={false}>
      <MemoryRouter>
        <ListActions>
          <RowAction
            label="基于此新建"
            icon={Copy}
            permission="devices.manage"
            to="/boards/new"
          />
          <RowAction label="转到会话" icon={Copy} to="/sessions" />
        </ListActions>
      </MemoryRouter>
    </PermissionsProvider>,
  );
  await userEvent.setup().click(screen.getByRole("button", { name: "更多" }));
  expect(
    screen.queryByRole("menuitem", { name: "基于此新建" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "转到会话" })).toBeVisible();
});

it("keeps at most three buttons and moves extra actions into more", async () => {
  render(
    <MemoryRouter>
      <ListActions
        primary={[
          <RowAction label="操作一" icon={Copy} />,
          <RowAction label="操作二" icon={Copy} />,
          <RowAction label="操作三" icon={Copy} />,
          <RowAction label="操作四" icon={Copy} />,
        ]}
      />
    </MemoryRouter>,
  );
  expect(screen.getAllByRole("button")).toHaveLength(3);
  expect(screen.getByRole("button", { name: "操作一" })).toBeVisible();
  expect(screen.getByRole("button", { name: "操作二" })).toBeVisible();
  await userEvent.setup().click(screen.getByRole("button", { name: "更多" }));
  expect(screen.getByRole("menuitem", { name: "操作三" })).toBeVisible();
  expect(screen.getByRole("menuitem", { name: "操作四" })).toBeVisible();
});
