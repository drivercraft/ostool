import { expect, it } from "vitest";
import { formatMailSender, splitMailSender } from "./mail-sender";
it("round trips plain addresses, Unicode and quoted sender names", () => {
  for (const name of ["", "开发板工作室", 'Lab, "A"']) {
    const address = "noreply@example.test";
    expect(splitMailSender(formatMailSender(name, address))).toEqual({
      name,
      address,
    });
  }
  expect(splitMailSender("ostool <noreply@example.test>")).toEqual({
    name: "ostool",
    address: "noreply@example.test",
  });
});
it("preserves a display name before the address is filled and clears empty drafts", () => {
  expect(splitMailSender(formatMailSender("ostool", ""))).toEqual({
    name: "ostool",
    address: "",
  });
  expect(formatMailSender("", "")).toBeNull();
});
