import { expect, it } from "vitest";
import { parsePasswordResetLink } from "./password-reset";
const id = "12345678-1234-4234-8234-123456789abc";
const code = "a".repeat(41) + "_-";
it("reads reset credentials only from the email fragment", () => {
  expect(parsePasswordResetLink(`#challenge_id=${id}&code=${code}`)).toEqual({
    challenge_id: id,
    code,
  });
});
it("rejects missing, malformed, and old six-digit credentials", () => {
  for (const fragment of [
    "",
    `#challenge_id=${id}`,
    `#challenge_id=${id}&code=123456`,
    `#challenge_id=invalid&code=${code}`,
    `#challenge_id=${id}&code=${"a".repeat(42)}!`,
  ]) {
    expect(parsePasswordResetLink(fragment)).toBeNull();
  }
});
