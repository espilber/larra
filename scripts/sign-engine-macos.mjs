#!/usr/bin/env node
// No-op unless this is a signed macOS build (APPLE_SIGNING_IDENTITY set).

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "darwin" || !process.env.APPLE_SIGNING_IDENTITY) {
  process.exit(0);
}

const sh = path.join(path.dirname(fileURLToPath(import.meta.url)), "sign-engine-macos.sh");
const result = spawnSync("bash", [sh], { stdio: "inherit" });
process.exit(result.status === null ? 1 : result.status);
