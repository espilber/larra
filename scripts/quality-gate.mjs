#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { existsSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const [model, engine, report = "quality-results.json"] = process.argv.slice(2);
if (!model || !engine || !existsSync(model) || !existsSync(engine)) {
  console.error("Usage: node scripts/quality-gate.mjs /path/model.gguf /path/pinned-engine-archive [report.json]");
  process.exit(2);
}
const result = spawnSync("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--test", "experience_quality", "--", "--ignored", "--nocapture"], {
  stdio: "inherit",
  env: { ...process.env, REBOST_TEST_MODEL: resolve(model), REBOST_ENGINE_ARCHIVE: resolve(engine), REBOST_QUALITY_REPORT: resolve(report) },
});
if (result.error) console.error(result.error.message);
if (result.status !== 0) writeFileSync(resolve(report), JSON.stringify({ status: "failed", exitCode: result.status, error: result.error?.message ?? null }, null, 2));
process.exit(result.status ?? 1);
