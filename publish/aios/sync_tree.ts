// File-level upstream sync of the worker entrypoint (R-15): new files are copied, files unchanged since the last sync are
// updated, files changed locally are kept and reported, and nothing is ever deleted (that is the "reset volume" operation).
// Runs on the image's own Deno: the entrypoint must not depend on any extension tool.
//
//   deno run --no-config --allow-read --allow-write sync_tree.ts <src> <dst> <meta_path> <log_path>
import * as crypto from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";

type Stats = { copied: number; updated: number; kept_local: number; removed_from_upstream: number };

function sha256Of(path: string): string {
  const hash = crypto.createHash("sha256");
  hash.update(readFileSync(path));
  return hash.digest("hex");
}

function* filesUnder(root: string): Generator<string> {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (statSync(path, { throwIfNoEntry: false })?.isDirectory()) yield* filesUnder(path);
    else if (existsSync(path)) yield path;
  }
}

function copyKeepingTimes(source: string, target: string): void {
  copyFileSync(source, target);
  const { atime, mtime } = statSync(source);
  utimesSync(target, atime, mtime);
}

function readSnapshot(metaPath: string): Record<string, string> {
  try {
    return JSON.parse(readFileSync(metaPath, "utf8")).upstream ?? {};
  } catch {
    return {};
  }
}

export function syncTree(src: string, dst: string, metaPath: string, logPath: string): Stats {
  mkdirSync(dst, { recursive: true });
  mkdirSync(dirname(metaPath), { recursive: true });
  mkdirSync(dirname(logPath), { recursive: true });
  const upstream = readSnapshot(metaPath);
  const stats: Stats = { copied: 0, updated: 0, kept_local: 0, removed_from_upstream: 0 };
  const events: Record<string, unknown>[] = [];
  const next: Record<string, string> = {};
  for (const file of filesUnder(src)) {
    const rel = relative(src, file).split(sep).join("/");
    const target = join(dst, rel);
    const upstreamHash = sha256Of(file);
    next[rel] = upstreamHash;
    if (!existsSync(target)) {
      mkdirSync(dirname(target), { recursive: true });
      copyKeepingTimes(file, target);
      stats.copied += 1;
      continue;
    }
    const remembered = upstream[rel];
    if (remembered === undefined) {
      // There before we tracked it: locally owned, unless it happens to be identical.
      if (sha256Of(target) !== upstreamHash) {
        events.push({ path: rel, reason: "local_unknown_kept" });
        stats.kept_local += 1;
      }
      continue;
    }
    if (remembered === upstreamHash) continue;
    const localHash = sha256Of(target);
    if (localHash === remembered) {
      copyKeepingTimes(file, target);
      stats.updated += 1;
      continue;
    }
    events.push({ path: rel, reason: "local_divergence_kept", local_sha256: localHash, upstream_sha256: upstreamHash, snapshot_sha256: remembered });
    stats.kept_local += 1;
  }
  for (const rel of Object.keys(upstream)) {
    if (!(rel in next) && existsSync(join(dst, rel))) {
      stats.removed_from_upstream += 1;
      events.push({ path: rel, reason: "upstream_removed_local_kept" });
    }
  }
  const updated_at = Date.now() / 1000;
  writeFileSync(metaPath, JSON.stringify({ updated_at, upstream: next }, null, 2));
  writeFileSync(logPath, JSON.stringify({ updated_at, stats, events }, null, 2));
  return stats;
}

if (import.meta.main) {
  const [src, dst, metaPath, logPath] = Deno.args;
  console.log(JSON.stringify(syncTree(src, dst, metaPath, logPath)));
}
