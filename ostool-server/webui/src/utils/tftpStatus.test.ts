import { describe, expect, it } from "vitest";

import { describeTftpStatus } from "./tftpStatus";

describe("describeTftpStatus", () => {
  it("returns good when enabled healthy and writable", () => {
    expect(
      describeTftpStatus({
        provider: "builtin",
        enabled: true,
        healthy: true,
        writable: true,
        root_dir: "/srv/tftp",
        bind_addr_or_address: ":69",
        service_state: "active",
        last_error: null,
      }),
    ).toEqual({ tone: "good", label: "运行正常" });
  });

  it("returns warn when healthy but not writable", () => {
    expect(
      describeTftpStatus({
        provider: "builtin",
        enabled: true,
        healthy: true,
        writable: false,
        root_dir: "/srv/tftp",
        bind_addr_or_address: ":69",
        service_state: "active",
        last_error: "permission denied",
      }),
    ).toEqual({ tone: "warn", label: "不可写" });
  });
});
