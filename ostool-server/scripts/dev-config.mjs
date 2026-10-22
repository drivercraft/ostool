// Read dotenv as data using Node's built-in parser; never execute it as shell code.
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { parseEnv } from "node:util";
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

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const [, , file, script, ...args] = process.argv;
  try {
    const env = developmentEnvironment(readFileSync(file, "utf8"), process.env);
    const child = spawnSync("bash", [script, ...args], {
      env,
      stdio: "inherit",
    });
    if (child.error) throw child.error;
    if (child.signal) process.kill(process.pid, child.signal);
    else process.exitCode = child.status ?? 1;
  } catch {
    // Do not include dotenv contents, which may contain credentials, in errors.
    console.error("dev: failed to load development environment or run dev.sh");
    process.exitCode = 1;
  }
}
