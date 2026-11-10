import { rm } from "node:fs/promises";
import { basename } from "node:path";
export default async function cleanup() {
  const path = process.env.OSTOOL_E2E_LOCAL_AUTH_STATE;
  if (path && /^ostool-e2e-auth-[a-f0-9]+\.json$/.test(basename(path)))
    await rm(path, { force: true });
}
