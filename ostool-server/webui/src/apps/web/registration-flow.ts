import type { EmailChallenge } from "@/api/client";

export type RegistrationStep = 1 | 2 | 3;
export type RegistrationChallenge = EmailChallenge & {
  email: string;
  expiresAt: number;
  verifiedCode: string;
};
export const normalizeRegistrationEmail = (email: string) =>
  email.trim().toLowerCase();
export function registrationProgress(
  email: string,
  code: string,
  challenge: RegistrationChallenge | undefined,
  now = Date.now(),
) {
  const active =
    !!challenge &&
    challenge.email === normalizeRegistrationEmail(email) &&
    challenge.expiresAt > now;
  return {
    active,
    verified: active && /^\d{6}$/.test(code) && challenge.verifiedCode === code,
  };
}
export function canVisitRegistrationStep(
  target: RegistrationStep,
  emailRequired: boolean,
  email: string,
  code: string,
  challenge: RegistrationChallenge | undefined,
  resendRequested: boolean,
  now = Date.now(),
) {
  if (target === 1) return true;
  if (!emailRequired)
    return target === 3 && !!normalizeRegistrationEmail(email);
  const progress = registrationProgress(email, code, challenge, now);
  return (
    !resendRequested && progress.active && (target === 2 || progress.verified)
  );
}
