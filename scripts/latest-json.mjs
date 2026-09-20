#!/usr/bin/env node
// Build or merge updater latest.json.
// Fragments live in src-tauri/target/updater (Vite empties dist/ on each build).
// Repository URL comes from src-tauri/Cargo.toml (same source the app uses).
//
//   node scripts/latest-json.mjs --bundle-dir src-tauri/target/release/bundle/macos --triple aarch64-apple-darwin
//   node scripts/latest-json.mjs --combine

import { copyFile, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const CARGO_TOML = path.join(ROOT, "src-tauri/Cargo.toml");
const OUT_DIR = path.join(ROOT, "src-tauri/target/updater");
const MERGED = path.join(OUT_DIR, "latest.json");
const DIST_MERGED = path.join(ROOT, "dist/latest.json");

const TRIPLE_TO_PLATFORM = {
  "aarch64-apple-darwin": "darwin-aarch64",
  "x86_64-apple-darwin": "darwin-x86_64",
  "x86_64-pc-windows-msvc": "windows-x86_64",
  "aarch64-pc-windows-msvc": "windows-aarch64",
};

function packageField(toml, key) {
  const block = toml.match(/\[package\]([\s\S]*?)(?:\n\[|$)/);
  if (!block) return null;
  const match = block[1].match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m"));
  return match ? match[1] : null;
}

function repoRoot(repository) {
  return repository
    .trim()
    .replace(/\/+$/, "")
    .replace(/\.git$/, "");
}

function latestJsonUrl(repository) {
  const repo = repoRoot(repository);
  if (!repo.startsWith("https://")) return null;
  return `${repo}/releases/latest/download/latest.json`;
}

function assetUrl(repository, version, filename) {
  const repo = repoRoot(repository);
  return `${repo}/releases/download/v${version}/${filename}`;
}

// GitHub asset names. Mac updater archives are both Rebost.app.tar.gz on disk;
// Windows URLs must match installer-names.sh (the public NSIS filename).
function publicUpdaterFilename(version, platform) {
  switch (platform) {
    case "darwin-aarch64":
      return `Rebost-${version}-darwin-aarch64.app.tar.gz`;
    case "darwin-x86_64":
      return `Rebost-${version}-darwin-x86_64.app.tar.gz`;
    case "windows-x86_64":
      return `Rebost-${version}-Windows.exe`;
    case "windows-aarch64":
      return `Rebost-${version}-Windows-ARM.exe`;
    default:
      throw new Error(`No public updater name for ${platform}`);
  }
}

async function stageMacUpdaterArchive(pair, version, platform) {
  if (!pair.filename.endsWith(".app.tar.gz")) return;
  await mkdir(OUT_DIR, { recursive: true });
  const publicName = publicUpdaterFilename(version, platform);
  await copyFile(pair.artifact, path.join(OUT_DIR, publicName));
  await copyFile(pair.sig, path.join(OUT_DIR, `${publicName}.sig`));
}

function parseArgs(argv) {
  const out = { combine: false, bundleDir: null, triple: null, notes: "" };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--combine") out.combine = true;
    else if (arg === "--bundle-dir") out.bundleDir = argv[++i];
    else if (arg === "--triple") out.triple = argv[++i];
    else if (arg === "--notes") out.notes = argv[++i] ?? "";
    else if (arg === "--help" || arg === "-h") out.help = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return out;
}

async function findPair(bundleDir) {
  const names = await readdir(bundleDir);
  const archives = names.filter(
    (name) => name.endsWith(".app.tar.gz") || (name.endsWith(".exe") && !name.endsWith(".sig")),
  );
  for (const name of archives) {
    const sig = `${name}.sig`;
    if (names.includes(sig)) {
      return {
        artifact: path.join(bundleDir, name),
        sig: path.join(bundleDir, sig),
        filename: name,
      };
    }
  }
  return null;
}

function emptyManifest(version, notes) {
  return {
    version,
    notes: notes || "",
    pub_date: new Date().toISOString(),
    platforms: {},
  };
}

async function writeJson(file, value) {
  await mkdir(path.dirname(file), { recursive: true });
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`);
}

async function addPlatform({ bundleDir, triple, notes }) {
  const toml = await readFile(CARGO_TOML, "utf8");
  const version = packageField(toml, "version");
  const repository = packageField(toml, "repository");
  if (!version || !repository) {
    throw new Error("Could not read package.version / package.repository from Cargo.toml");
  }
  const platform = TRIPLE_TO_PLATFORM[triple];
  if (!platform) {
    throw new Error(`Unsupported triple: ${triple}`);
  }
  const pair = await findPair(bundleDir);
  if (!pair) {
    throw new Error(`No updater artifact + .sig in ${bundleDir}`);
  }
  const signature = (await readFile(pair.sig, "utf8")).trim();
  if (!signature) {
    throw new Error(`Empty signature file: ${pair.sig}`);
  }

  const publicName = publicUpdaterFilename(version, platform);
  const fragment = emptyManifest(version, notes);
  fragment.platforms[platform] = {
    signature,
    url: assetUrl(repository, version, publicName),
  };
  const fragmentPath = path.join(OUT_DIR, `latest-${platform}.json`);
  await writeJson(fragmentPath, fragment);
  await stageMacUpdaterArchive(pair, version, platform);
  await combine({ notes });
  return { fragmentPath, platform, filename: publicName, endpoint: latestJsonUrl(repository) };
}

async function combine({ notes }) {
  const toml = await readFile(CARGO_TOML, "utf8");
  const version = packageField(toml, "version");
  const repository = packageField(toml, "repository");
  if (!version || !repository) {
    throw new Error("Could not read package.version / package.repository from Cargo.toml");
  }
  await mkdir(OUT_DIR, { recursive: true });
  let names = [];
  try {
    names = (await readdir(OUT_DIR)).filter(
      (name) => name.startsWith("latest-") && name.endsWith(".json"),
    );
  } catch {
    names = [];
  }
  const merged = emptyManifest(version, notes);
  for (const name of names.sort()) {
    const fragment = JSON.parse(await readFile(path.join(OUT_DIR, name), "utf8"));
    if (fragment.version !== version) continue;
    Object.assign(merged.platforms, fragment.platforms);
    if (!notes && fragment.notes) merged.notes = fragment.notes;
    if (fragment.pub_date) merged.pub_date = fragment.pub_date;
  }
  await writeJson(MERGED, merged);
  await mkdir(path.dirname(DIST_MERGED), { recursive: true });
  await copyFile(MERGED, DIST_MERGED);
  return {
    merged: MERGED,
    platforms: Object.keys(merged.platforms),
    endpoint: latestJsonUrl(repository),
  };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(`Usage:
  node scripts/latest-json.mjs --bundle-dir DIR --triple TRIPLE
  node scripts/latest-json.mjs --combine`);
    return;
  }
  if (args.combine) {
    const result = await combine({ notes: args.notes });
    console.log(`Wrote ${result.merged} (${result.platforms.join(", ") || "no platforms"})`);
    console.log(`App endpoint: ${result.endpoint}`);
    return;
  }
  if (!args.bundleDir || !args.triple) {
    throw new Error("Need --bundle-dir and --triple, or --combine");
  }
  const bundleDir = path.resolve(args.bundleDir);
  const result = await addPlatform({
    bundleDir,
    triple: args.triple,
    notes: args.notes,
  });
  console.log(`Updater fragment: ${result.fragmentPath}`);
  console.log(`Asset: ${result.filename} (${result.platform})`);
  console.log(`Merged: ${MERGED}`);
  console.log(`App endpoint: ${result.endpoint}`);
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  main().catch((error) => {
    console.error(error.message || error);
    process.exit(1);
  });
}

export { latestJsonUrl, repoRoot, assetUrl, packageField, publicUpdaterFilename };
