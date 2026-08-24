import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  renameSync,
  rmSync,
} from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(fileURLToPath(import.meta.url));
const dappMetaDir = join(rootDir, "dapp_meta");
const dappDistDir = join(rootDir, "dapp_dist");
const pikgName = "jarvis.buckyos.bns.did-0.7.0.pikg";
const toolPath = join(rootDir, "..", "..", "tools", "buckyos-tool", "buckyos");
const rootfsDir = join(rootDir, "..", "..", "rootfs");

function runPikgTool(args) {
  const result = spawnSync(toolPath, args, {
    cwd: rootDir,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`buckyos ${args.join(" ")} failed with ${result.status}`);
  }
}

function installGeneratedFile(source, target) {
  mkdirSync(dirname(target), { recursive: true });
  const transaction = `${process.pid}-${Date.now()}`;
  const temporary = join(dirname(target), `.${transaction}-${basename(target)}.tmp`);
  const backup = join(dirname(target), `.${transaction}-${basename(target)}.bak`);
  const hadTarget = existsSync(target);
  copyFileSync(source, temporary);
  try {
    if (hadTarget) {
      renameSync(target, backup);
    }
    renameSync(temporary, target);
    rmSync(backup, { force: true });
  } catch (error) {
    rmSync(target, { force: true });
    if (existsSync(backup)) {
      renameSync(backup, target);
    }
    rmSync(temporary, { force: true });
    throw error;
  }
}

runPikgTool(["pikg", "build", dappMetaDir]);
runPikgTool(["pikg", "pack", dappDistDir]);
const builtPikg = join(dappDistDir, pikgName);
runPikgTool(["pikg", "info", builtPikg]);

const rootfsPikg = join(rootfsDir, "data", "cache", pikgName);
installGeneratedFile(builtPikg, rootfsPikg);
runPikgTool(["pikg", "info", rootfsPikg]);
