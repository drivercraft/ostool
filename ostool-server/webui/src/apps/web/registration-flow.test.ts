import { describe, expect, it } from "vitest";
import {
  createRegistrationChallenge,
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
        100,
      ),
    ).toBe(true);
    expect(
      canVisitRegistrationStep(2, true, challenge.email, "", challenge, 100),
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
        10000,
      ),
    ).toBe(true);
  });
  it("requires verification again after a successful resend", () => {
    const resent = createRegistrationChallenge(
      " MEMBER@EXAMPLE.TEST ",
      { ...challenge, challenge_id: "fresh-proof" },
      100,
    );
    expect(resent.email).toBe(challenge.email);
    expect(resent.expiresAt).toBe(600100);
    expect(resent.verifiedCode).toBe("");
    expect(
      canVisitRegistrationStep(2, true, challenge.email, "", resent, 100),
    ).toBe(true);
    expect(
      canVisitRegistrationStep(3, true, challenge.email, "123456", resent, 100),
    ).toBe(false);
  });
  it("skips email verification only for sites that disable it", () => {
    expect(
      canVisitRegistrationStep(3, false, challenge.email, "", undefined, 100),
    ).toBe(true);
    expect(
      canVisitRegistrationStep(2, false, challenge.email, "", undefined, 100),
    ).toBe(false);
    expect(canVisitRegistrationStep(3, false, "  ", "", undefined, 100)).toBe(
      false,
    );
  });
});
