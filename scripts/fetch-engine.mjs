#!/usr/bin/env node
// Download the pinned llama.cpp archive for this build target and stage it
// into src-tauri/resources/engine/ (the only file Tauri bundles).
// Optional CUDA / OpenCL pins in pin.rs are not staged; the app downloads
// those at warmup when the GPU is present.
//
//   node scripts/fetch-engine.mjs              # host (or TAURI_ENV_TARGET_TRIPLE)
//   node scripts/fetch-engine.mjs --all        # cache macOS + Windows installer pins
//   node scripts/fetch-engine.mjs --dry-run    # print the pin(s), no download

import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, rm, copyFile, writeFile, rename } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const PIN_RS = path.join(ROOT, "src-tauri/src/engine/pin.rs");
const CACHE = path.join(ROOT, "src-tauri/resources/engine-cache");
const STAGE = path.join(ROOT, "src-tauri/resources/engine");
const KEEP_IN_STAGE = new Set(["README.md", ".gitkeep"]);

const TRIPLES = {
  "aarch64-apple-darwin": ["macos", "aarch64"],
  "x86_64-apple-darwin": ["macos", "x86_64"],
  "x86_64-pc-windows-msvc": ["windows", "x86_64"],
  "aarch64-pc-windows-msvc": ["windows", "aarch64"],
  "x86_64-unknown-linux-gnu": ["linux", "x86_64"],
  "aarch64-unknown-linux-gnu": ["linux", "aarch64"],
};

function parsePins(text) {
  const pins = [];
  const re = /EnginePin \{([^}]+)\}/g;
  let match;
  while ((match = re.exec(text))) {
    const block = match[1];
    const field = (name) => {
      const found = block.match(new RegExp(`${name}: "([^"]+)"`));
      return found ? found[1] : null;
    };
    const os = field("os");
    const arch = field("arch");
    const url = field("url");
    const sha256 = field("sha256");
    const fileName = field("file_name");
    const accelerator = field("accelerator");
    if (os && arch && url && sha256 && fileName) {
      pins.push({ os, arch, url, sha256, fileName, accelerator });
    }
  }
  if (pins.length === 0) {
    throw new Error(`no EnginePin entries in ${PIN_RS}`);
  }
  return pins;
}

function hostOsArch() {
  const os = { darwin: "macos", win32: "windows", linux: "linux" }[process.platform];
  const arch = { arm64: "aarch64", x64: "x86_64" }[process.arch];
  if (!os || !arch) {
    throw new Error(`unsupported host ${process.platform}/${process.arch}`);
  }
  return { os, arch };
}

function osArchFromEnv() {
  const triple = process.env.TAURI_ENV_TARGET_TRIPLE;
  if (triple && TRIPLES[triple]) {
    return TRIPLES[triple];
  }
  const plat = process.env.TAURI_ENV_PLATFORM;
  const arch = process.env.TAURI_ENV_ARCH;
  if (!plat || !arch) {
    return null;
  }
  const os = { darwin: "macos", macos: "macos", windows: "windows", win32: "windows", linux: "linux" }[
    plat
  ];
  const mappedArch = { arm64: "aarch64", aarch64: "aarch64", x64: "x86_64", x86_64: "x86_64" }[arch];
  if (!os || !mappedArch) {
    return null;
  }
  return [os, mappedArch];
}

function bundledAccelerator(os, arch) {
  if (os === "macos") return "Metal";
  if (os === "linux") return "Vulkan";
  if (os === "windows" && arch === "x86_64") return "Vulkan";
  if (os === "windows" && arch === "aarch64") return "CPU";
  return null;
}

function pickBundledPin(all, os, arch) {
  const accel = bundledAccelerator(os, arch);
  const pin = all.find((p) => p.os === os && p.arch === arch && p.accelerator === accel);
  if (!pin) {
    throw new Error(`no bundled engine pin for ${os}/${arch}`);
  }
  return pin;
}

function selectPins(all, args) {
  if (args.includes("--all")) {
    const includeLinux = args.includes("--include-linux");
    return all.filter((pin) => {
      if (!includeLinux && pin.os === "linux") return false;
      return pin.accelerator === bundledAccelerator(pin.os, pin.arch);
    });
  }
  const tripleFlag = args.find((a) => a.startsWith("--triple="));
  if (tripleFlag) {
    const triple = tripleFlag.slice("--triple=".length);
    const mapped = TRIPLES[triple];
    if (!mapped) {
      throw new Error(`no engine pin for target ${triple}`);
    }
    const [os, arch] = mapped;
    return [pickBundledPin(all, os, arch)];
  }
  const fromEnv = osArchFromEnv();
  if (fromEnv) {
    const [os, arch] = fromEnv;
    return [pickBundledPin(all, os, arch)];
  }
  const { os, arch } = hostOsArch();
  return [pickBundledPin(all, os, arch)];
}

async function downloadPin(pin) {
  await mkdir(CACHE, { recursive: true });
  const dest = path.join(CACHE, pin.fileName);
  try {
    const existing = await readFile(dest);
    const digest = createHash("sha256").update(existing).digest("hex");
    if (digest === pin.sha256) {
      console.log(`cached ${pin.fileName}`);
      return dest;
    }
  } catch {
    // download
  }
  console.log(`GET ${pin.url}`);
  const response = await fetch(pin.url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`${response.status} ${pin.url}`);
  }
  const buf = Buffer.from(await response.arrayBuffer());
  const digest = createHash("sha256").update(buf).digest("hex");
  if (digest !== pin.sha256) {
    throw new Error(`SHA-256 mismatch for ${pin.fileName}: got ${digest}`);
  }
  const tmp = `${dest}.part`;
  await writeFile(tmp, buf);
  await rename(tmp, dest);
  console.log(`stored ${pin.fileName}`);
  return dest;
}

async function stagePin(pin, cacheFile) {
  await mkdir(STAGE, { recursive: true });
  for (const name of await readdir(STAGE)) {
    if (KEEP_IN_STAGE.has(name)) {
      continue;
    }
    await rm(path.join(STAGE, name), { recursive: true, force: true });
  }
  await copyFile(cacheFile, path.join(STAGE, pin.fileName));
  console.log(`staged ${pin.fileName} for bundle`);
}

const args = process.argv.slice(2);
const pins = parsePins(await readFile(PIN_RS, "utf8"));
const selected = selectPins(pins, args);
if (selected.length === 0) {
  throw new Error("no pins selected");
}
if (args.includes("--dry-run")) {
  for (const pin of selected) {
    console.log(`${pin.os}/${pin.arch} ${pin.fileName} ${pin.accelerator}`);
  }
  process.exit(0);
}

const downloaded = [];
for (const pin of selected) {
  downloaded.push([pin, await downloadPin(pin)]);
}
if (!args.includes("--all")) {
  await stagePin(downloaded[0][0], downloaded[0][1]);
} else {
  const { os, arch } = hostOsArch();
  const host = downloaded.find(([pin]) => pin.os === os && pin.arch === arch);
  if (host) {
    await stagePin(host[0], host[1]);
  }
}
