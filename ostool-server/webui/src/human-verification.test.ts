import { describe, expect, it } from "vitest";
import {
  parseHumanChallenge,
  parseVerificationHostnames,
} from "./human-verification";
const id = "00000000-0000-4000-8000-000000000001";
const image = {
  id,
  action: "login",
  provider: "image",
  image_data_url: "data:image/png;base64,iVBORw==",
  expires_in: 120,
};
const cloudflare = {
  id,
  action: "login",
  provider: "turnstile",
  site_key: "public-site-key",
  cdata: id,
  expires_in: 300,
};
describe("human verification contract", () => {
  it("accepts local PNG and bound Cloudflare challenges", () => {
    expect(parseHumanChallenge(image, "login")).toEqual(image);
    expect(parseHumanChallenge(cloudflare, "login")).toEqual(cloudflare);
  });
  it("rejects wrong-purpose, stale or incomplete provider responses", () => {
    for (const value of [
      null,
      { ...image, action: "registration_email" },
      { ...image, id: "invalid" },
      { ...image, expires_in: 0 },
      { ...image, expires_in: 301 },
      { ...image, provider: "disabled" },
      { ...image, image_data_url: "data:image/svg+xml,<svg/>" },
      { ...cloudflare, site_key: "" },
      { ...cloudflare, cdata: "different-challenge" },
      { ...cloudflare, cdata: undefined },
    ]) {
      expect(() => parseHumanChallenge(value, "login")).toThrow(
        "人机验证配置无效",
      );
    }
  });
  it("returns only the documented public challenge fields", () => {
    expect(
      parseHumanChallenge({ ...image, secret: "must-not-propagate" }, "login"),
    ).toEqual(image);
  });
  it("normalizes hostname lists from newlines, spaces and both comma forms", () => {
    expect(
      parseVerificationHostnames(
        " Boards.Example.test，localhost\nboards.example.test 127.0.0.1 ",
      ),
    ).toEqual(["boards.example.test", "localhost", "127.0.0.1"]);
  });
});
