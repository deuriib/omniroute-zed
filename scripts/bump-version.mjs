#!/usr/bin/env node
/**
 * bump-version.mjs — bump the project version across every file listed in
 * .bump-version.json, then print the next conventional-commit type to use.
 *
 * Unlike the single-file template this grew from, it bumps ALL listed files
 * that carry the current version, so Cargo.toml and extension/extension.toml
 * can never drift apart.
 *
 * Usage:
 *   node scripts/bump-version.mjs            # interactive prompt (major|minor|patch)
 *   node scripts/bump-version.mjs patch      # non-interactive
 *   node scripts/bump-version.mjs minor --dry-run
 *
 * Exit codes: 0 success, 1 error (missing config, invalid version, bad type).
 */
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url))); // scripts/ -> repo root
const CONFIG_PATH = join(ROOT, ".bump-version.json");
const TYPES = ["major", "minor", "patch"];

function usage() {
  console.error(
    [
      "Usage: node scripts/bump-version.mjs [major|minor|patch] [--dry-run]",
      "",
      "  major|minor|patch  Semver increment type (prompted if omitted).",
      "  --dry-run          Print the resulting version without writing files.",
    ].join("\n")
  );
}

function parseArgs(argv) {
  const args = { type: null, dryRun: false };
  for (const a of argv) {
    if (a === "--dry-run" || a === "-n") args.dryRun = true;
    else if (a === "--help" || a === "-h") args.help = true;
    else if (TYPES.includes(a)) args.type = a;
    else {
      console.error(`Unknown argument: ${a}`);
      usage();
      process.exit(1);
    }
  }
  return args;
}

function readConfig() {
  if (!existsSync(CONFIG_PATH)) {
    console.error(`Missing config: ${CONFIG_PATH}`);
    process.exit(1);
  }
  try {
    return JSON.parse(readFileSync(CONFIG_PATH, "utf8"));
  } catch (err) {
    console.error(`Invalid JSON in ${CONFIG_PATH}: ${err.message}`);
    process.exit(1);
  }
}

const VERSION_RE =
  /("version"\s*:\s*")(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(")|^(version\s*=\s*["']?)(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)/m;

/** Read the current version out of one listed file, or null when absent. */
function readVersion(rel) {
  const p = join(ROOT, rel);
  if (!existsSync(p)) return null;
  const text = readFileSync(p, "utf8");
  if (rel.endsWith(".json")) {
    try {
      const j = JSON.parse(text);
      if (typeof j.version === "string") return { path: p, rel, raw: text, current: j.version };
    } catch {
      /* fall through to regex scan */
    }
  }
  const m = text.match(VERSION_RE);
  if (m) return { path: p, rel, raw: text, current: m[2] ?? m[5] };
  return null;
}

function bump(v, type) {
  const [main, ...preParts] = v.split("-");
  const [maj, min, pat] = main.split(".").map((n) => Number(n));
  if ([maj, min, pat].some((n) => Number.isNaN(n))) {
    console.error(`Current version is not semver: "${v}"`);
    process.exit(1);
  }
  let next;
  if (type === "major") next = [maj + 1, 0, 0];
  else if (type === "minor") next = [maj, min + 1, 0];
  else next = [maj, min, pat + 1];
  // Dropping any pre-release suffix on a real bump.
  void preParts;
  return next.join(".");
}

async function promptType() {
  const readline = await import("node:readline/promises");
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  try {
    const answer = (await rl.question("Bump type (major|minor|patch): ")).trim().toLowerCase();
    if (!TYPES.includes(answer)) {
      console.error(`Invalid type "${answer}". Expected: ${TYPES.join(" | ")}`);
      process.exit(1);
    }
    return answer;
  } finally {
    rl.close();
  }
}

function writeVersion(file, next) {
  if (file.rel.endsWith(".json")) {
    const j = JSON.parse(file.raw);
    j.version = next;
    writeFileSync(file.path, JSON.stringify(j, null, 2) + "\n", "utf8");
    return;
  }
  // Regex-preserving replacement for toml/composer/plain-text.
  const updated = file.raw.replace(
    VERSION_RE,
    (full, q1, _v1, q2, p1, _v2) => (q1 !== undefined ? `${q1}${next}${q2 ?? '"'}` : `${p1}${next}`)
  );
  writeFileSync(file.path, updated, "utf8");
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    usage();
    process.exit(0);
  }
  const config = readConfig();
  const rels = config.files ?? [];
  const found = rels.map(readVersion).filter(Boolean);
  if (found.length === 0) {
    console.error(`No version found in any of: ${rels.join(", ") || "(config.files empty)"}`);
    process.exit(1);
  }
  const current = found[0].current;
  const stale = found.filter((f) => f.current !== current);
  if (stale.length > 0) {
    console.error(
      `Version drift detected (expected ${current}): ${stale.map((f) => `${f.rel}=${f.current}`).join(", ")}`
    );
    process.exit(1);
  }
  const type = args.type ?? (process.stdin.isTTY ? await promptType() : null);
  if (!type) {
    console.error("No bump type given and stdin is not a TTY.");
    usage();
    process.exit(1);
  }
  const next = bump(current, type);
  if (args.dryRun) {
    for (const f of found) console.log(`[dry-run] ${f.rel}: ${current} -> ${next} (${type})`);
    process.exit(0);
  }
  for (const f of found) writeVersion(f, next);
  for (const f of found) console.log(`Bumped ${f.rel}: ${current} -> ${next} (${type})`);
  console.log(`Next: commit, tag v${next}, push tag (release + publish workflows take it from there).`);
}

main().catch((err) => {
  console.error(err?.stack || String(err));
  process.exit(1);
});
