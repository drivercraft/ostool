import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { HumanVerification } from "./human-verification";
const mocks = vi.hoisted(() => ({ captcha: vi.fn() }));
vi.mock("@/api/client", () => ({ api: mocks }));
const challenge = (id: string) => ({
  id,
  provider: "image",
  expires_in: 120,
  image_data_url: "data:image/png;base64,abc",
});
beforeEach(() => {
  vi.resetAllMocks();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
it("discards an older request when the verification action changes", async () => {
  let finish!: (value: unknown) => void;
  mocks.captcha
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValueOnce(challenge("registration-proof"));
  const changed = vi.fn();
  const view = render(<HumanVerification action="login" onChange={changed} />);
  view.rerender(<HumanVerification action="register" onChange={changed} />);
  await waitFor(() =>
    expect(view.container.firstChild).toHaveAttribute(
      "data-captcha-id",
      "registration-proof",
    ),
  );
  await act(async () => {
    finish(challenge("obsolete-login-proof"));
  });
  fireEvent.change(screen.getByLabelText("图形验证码"), {
    target: { value: "AB29Q" },
  });
  expect(changed).toHaveBeenLastCalledWith({
    captcha_id: "registration-proof",
    captcha_code: "AB29Q",
  });
});
it("clears the previous answer before requesting a replacement challenge", async () => {
  mocks.captcha
    .mockResolvedValueOnce(challenge("first"))
    .mockResolvedValueOnce(challenge("second"));
  const changed = vi.fn();
  render(<HumanVerification action="login" onChange={changed} />);
  await waitFor(() =>
    expect(screen.getByLabelText("图形验证码")).toBeEnabled(),
  );
  fireEvent.change(screen.getByLabelText("图形验证码"), {
    target: { value: "AB29Q" },
  });
  fireEvent.click(screen.getByRole("button", { name: "刷新图形验证码" }));
  await waitFor(() => expect(mocks.captcha).toHaveBeenCalledTimes(2));
  expect(screen.getByLabelText("图形验证码")).toHaveValue("");
  expect(changed).toHaveBeenLastCalledWith(undefined);
});
it.each(["login", "register", "admin_login"] as const)(
  "%s keeps the image and input until the user requests a refresh, even after expiry",
  async (action) => {
    vi.useFakeTimers();
    mocks.captcha
      .mockResolvedValueOnce(challenge("current"))
      .mockResolvedValueOnce(challenge("replacement"));
    const changed = vi.fn();
    const view = render(
      <HumanVerification action={action} onChange={changed} />,
    );
    await act(async () => {});
    fireEvent.change(screen.getByLabelText("图形验证码"), {
      target: { value: "AB29Q" },
    });
    await act(async () => {
      vi.advanceTimersByTime(600000);
    });
    expect(mocks.captcha).toHaveBeenCalledTimes(1);
    expect(view.container.firstChild).toHaveAttribute(
      "data-captcha-id",
      "current",
    );
    expect(screen.getByLabelText("图形验证码")).toHaveValue("AB29Q");
    expect(changed).toHaveBeenLastCalledWith({
      captcha_id: "current",
      captcha_code: "AB29Q",
    });

    fireEvent.click(screen.getByRole("button", { name: "刷新图形验证码" }));
    await act(async () => {});
    expect(mocks.captcha).toHaveBeenCalledTimes(2);
    expect(view.container.firstChild).toHaveAttribute(
      "data-captcha-id",
      "replacement",
    );
    expect(screen.getByLabelText("图形验证码")).toHaveValue("");
    expect(changed).toHaveBeenLastCalledWith(undefined);
  },
);
