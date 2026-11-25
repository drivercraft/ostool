import { describe, expect, it } from "vitest";
import {
  canVisitRegistrationStep,
  registrationProgress,
  type RegistrationChallenge,
} from "./registration-flow";

const challenge: RegistrationChallenge = {
  challenge_id: "proof",
  email: "member@example.test",
  expires_in: 600,
  resend_after: 60,
  message: "accepted",
  expiresAt: 10000,
  verifiedCode: "123456",
};
describe("registration step guards", () => {
  it("requires a live server-verified code before account setup", () => {
    expect(
      canVisitRegistrationStep(
        3,
        true,
        challenge.email,
        "123456",
        undefined,
        false,
        100,
      ),
    ).toBe(false);
    expect(
      canVisitRegistrationStep(
        3,
        true,
        challenge.email,
        "123456",
        { ...challenge, verifiedCode: "" },
        false,
        100,
      ),
    ).toBe(false);
    expect(
      canVisitRegistrationStep(
        3,
        true,
        challenge.email,
        "123456",
        challenge,
        false,
        100,
      ),
    ).toBe(true);
    expect(
      canVisitRegistrationStep(
        2,
        true,
        challenge.email,
        "",
        challenge,
        false,
        100,
      ),
    ).toBe(true);
  });
  it("allows normalized email edits but rejects email or code substitution", () => {
    expect(
      registrationProgress(" MEMBER@EXAMPLE.TEST ", "123456", challenge, 100)
        .verified,
    ).toBe(true);
    expect(
      registrationProgress("other@example.test", "123456", challenge, 100)
        .active,
    ).toBe(false);
    expect(
      registrationProgress(challenge.email, "654321", challenge, 100).verified,
    ).toBe(false);
  });
  it("expires verification at the exact deadline while allowing return to email entry", () => {
    for (const target of [2, 3] as const)
      expect(
        canVisitRegistrationStep(
          target,
          true,
          challenge.email,
          "123456",
          challenge,
          false,
          10000,
        ),
      ).toBe(false);
    expect(
      canVisitRegistrationStep(
        1,
        true,
        challenge.email,
        "123456",
        challenge,
        false,
        10000,
      ),
    ).toBe(true);
  });
  it("requires a new send after requesting a resend", () => {
    for (const target of [2, 3] as const)
      expect(
        canVisitRegistrationStep(
          target,
          true,
          challenge.email,
          "123456",
          challenge,
          true,
          100,
        ),
      ).toBe(false);
  });
  it("skips email verification only for sites that disable it", () => {
    expect(
      canVisitRegistrationStep(
        3,
        false,
        challenge.email,
        "",
        undefined,
        false,
        100,
      ),
    ).toBe(true);
    expect(
      canVisitRegistrationStep(
        2,
        false,
        challenge.email,
        "",
        undefined,
        false,
        100,
      ),
    ).toBe(false);
    expect(
      canVisitRegistrationStep(3, false, "  ", "", undefined, false, 100),
    ).toBe(false);
  });
});
