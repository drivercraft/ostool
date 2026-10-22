// Read dotenv as data using Node's built-in parser; never execute it as shell code.
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { parseEnv } from "node:util";
import { BlockList, isIP } from "node:net";
import { pathToFileURL } from "node:url";

export function developmentEnvironment(contents, environment) {
  const result = { ...environment };
  for (const [key, value] of Object.entries(parseEnv(contents))) {
    // Deployment database URLs, proxy settings and security policy must not
    // redirect the development launcher to a deployment's backing services.
    if (
      (key.startsWith("OSTOOL_DEV_") ||
        [
          "OSTOOL_EDITION",
          "OSTOOL_ADMIN_EMAIL",
          "OSTOOL_ADMIN_PASSWORD",
        ].includes(key)) &&
      !Object.hasOwn(environment, key)
    ) {
      result[key] = value;
    }
  }
  result.OSTOOL_DEV_CONFIG_LOADED = "1";
  return result;
}

export function developmentHost(host, wildcard = false) {
  const family = isIP(host);
  if (!family) return false;
  if (wildcard && ["0.0.0.0", "::"].includes(host)) return true;
  const ranges = new BlockList();
  if (family === 4) {
    for (const [address, prefix] of [
      ["127.0.0.0", 8],
      ["10.0.0.0", 8],
      ["172.16.0.0", 12],
      ["192.168.0.0", 16],
      ["169.254.0.0", 16],
    ])
      ranges.addSubnet(address, prefix, "ipv4");
  } else {
    for (const [address, prefix] of [
      ["::1", 128],
      ["fc00::", 7],
      ["fe80::", 10],
    ])
      ranges.addSubnet(address, prefix, "ipv6");
  }
  return ranges.check(host, family === 4 ? "ipv4" : "ipv6");
}
export function developmentNetwork(
  host,
  port,
  httpsHost = "",
  httpsPort = "3443",
) {
  if (!developmentHost(host, true))
    throw new Error(
      "OSTOOL_DEV_HOST 仅允许本机、局域网或通配地址；正式部署请使用 install.sh 和 Caddy",
    );
  for (const value of [port, ...(httpsHost ? [httpsPort] : [])])
    if (!/^\d+$/.test(value) || Number(value) < 1 || Number(value) > 65535)
      throw new Error("开发端口须为 1–65535 的整数");
  if (httpsHost && (!developmentHost(httpsHost) || host !== "127.0.0.1"))
    throw new Error(
      "HTTPS 调试须将 OSTOOL_DEV_HOST 设为 127.0.0.1，并为 OSTOOL_DEV_HTTPS_HOST 填写实际局域网 IP",
    );
}
export function developmentCaddyfile(host, port, backendPort) {
  developmentNetwork("127.0.0.1", backendPort, host, port);
  const address = isIP(host) === 6 ? `[${host}]` : host;
  return `# 自动生成的局域网 HTTPS 调试入口；正式部署使用 deploy/Caddyfile.example。
{
    admin off
    auto_https disable_redirects
    skip_install_trust
}
https://${address}:${port} {
    bind ${host}
    tls internal
    @hardware path /api/v1/loaders/* /api/v1/loader-updates/* /boot/* /share/*
    respond @hardware 404
    reverse_proxy 127.0.0.1:${backendPort}
}
`;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [, , file, script, ...args] = process.argv;
  try {
    if (file === "--check-network") {
      developmentNetwork(script, ...args);
    } else if (file === "--caddyfile") {
      process.stdout.write(developmentCaddyfile(script, ...args));
    } else {
      const env = developmentEnvironment(
        readFileSync(file, "utf8"),
        process.env,
      );
      const child = spawnSync("bash", [script, ...args], {
        env,
        stdio: "inherit",
      });
      if (child.error) throw child.error;
      if (child.signal) process.kill(process.pid, child.signal);
      else process.exitCode = child.status ?? 1;
    }
  } catch (error) {
    // Do not include dotenv contents, which may contain credentials, in errors.
    console.error(
      file.startsWith("--")
        ? `dev: ${error.message}`
        : "dev: failed to load development environment or run dev.sh",
    );
    process.exitCode = 1;
  }
}
