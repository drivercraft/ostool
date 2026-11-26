import { expect, it } from "vitest";
import { durationWarning } from "./auto-approval";

const policy = {
  min_available_minutes: 30,
  max_duration_minutes: 480,
  max_concurrent_sessions: 2,
  board_types: [],
};
it("warns outside the inclusive automatic approval duration, including fractional minutes", () => {
  for (const minutes of [30, 60, 480]) {
    expect(durationWarning(0, minutes * 60000, policy)).toBeNull();
  }
  for (const minutes of [29.999, 480.001]) {
    expect(durationWarning(0, minutes * 60000, policy)).toContain("转人工审核");
  }
});
it("respects customized roles and leaves invalid times to form validation", () => {
  expect(
    durationWarning(0, 20 * 60000, { ...policy, min_available_minutes: 10 }),
  ).toBeNull();
  for (const [start, end] of [
    [NaN, 1],
    [0, Infinity],
    [1, 0],
    [1, 1],
  ]) {
    expect(durationWarning(start, end, policy)).toBeNull();
  }
});
