import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  renameSync,
  rmSync,
} from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(fileURLToPath(import.meta.url));
const distDir = join(rootDir, "dist");
const webDistDir = join(rootDir, "web", "dist");
const sdkPackageDir = join(rootDir, "node_modules", "buckyos");
const sdkDistDir = join(sdkPackageDir, "dist");
const sdkTargetDir = join(distDir, "node_modules", "buckyos");
const dappMetaDir = join(rootDir, "dapp_meta");
const dappDistDir = join(rootDir, "dapp_dist");
const toolPackageRoot = process.env.BUCKYOS_SDK_TOOL_PACKAGE_ROOT;
if (!toolPackageRoot) {
  throw new Error(
    "BUCKYOS_SDK_TOOL_PACKAGE_ROOT must reference the extracted immutable SDK/Tool artifact",
  );
}
const toolLauncher = join(toolPackageRoot, "cli", "launcher.mjs");
const pikgName = "buckyos-systest.buckyos.bns.did-0.5.1.pikg";
const rootfsDir = join(rootDir, "..", "..", "rootfs");

function runPikgTool(args) {
  if (!existsSync(toolLauncher)) {
    throw new Error(`missing SDK Tool package launcher: ${toolLauncher}`);
  }
  const result = spawnSync(process.execPath, [toolLauncher, ...args], {
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

function installGeneratedFiles(files) {
  const transaction = `${process.pid}-${Date.now()}`;
  const prepared = files.map(({ source, target }) => {
    mkdirSync(dirname(target), { recursive: true });
    const temporary = join(dirname(target), `.${transaction}-${basename(target)}.tmp`);
    const backup = join(dirname(target), `.${transaction}-${basename(target)}.bak`);
    copyFileSync(source, temporary);
    return { target, temporary, backup, hadTarget: existsSync(target), installed: false };
  });
  try {
    for (const item of prepared) {
      if (item.hadTarget) {
        renameSync(item.target, item.backup);
      }
      renameSync(item.temporary, item.target);
      item.installed = true;
    }
    for (const item of prepared) {
      rmSync(item.backup, { force: true });
    }
  } catch (error) {
    for (const item of [...prepared].reverse()) {
      if (item.installed) {
        rmSync(item.target, { force: true });
      }
      if (existsSync(item.backup)) {
        renameSync(item.backup, item.target);
      }
      rmSync(item.temporary, { force: true });
    }
    throw error;
  }
}

if (!existsSync(webDistDir)) {
  throw new Error(`missing web dist: ${webDistDir}`);
}
if (!existsSync(sdkDistDir)) {
  throw new Error(`missing buckyos websdk dist: ${sdkDistDir}`);
}

rmSync(distDir, { recursive: true, force: true });
mkdirSync(distDir, { recursive: true });

copyFileSync(join(rootDir, "main.ts"), join(distDir, "main.ts"));
copyFileSync(join(rootDir, "deno.json"), join(distDir, "deno.json"));
cpSync(webDistDir, join(distDir, "web"), { recursive: true });
mkdirSync(sdkTargetDir, { recursive: true });
copyFileSync(
  join(sdkPackageDir, "package.json"),
  join(sdkTargetDir, "package.json"),
);
cpSync(sdkDistDir, join(sdkTargetDir, "dist"), { recursive: true });

runPikgTool(["pikg", "build", dappMetaDir]);
runPikgTool(["pikg", "pack", dappDistDir]);
const builtPikg = join(dappDistDir, pikgName);
runPikgTool(["pikg", "info", builtPikg]);

const rootfsPikg = join(rootfsDir, "data", "cache", pikgName);
const rootfsAppDoc = join(
  rootfsDir,
  "local",
  "did_docs",
  "buckyos-systest.buckyos.bns.did.doc.json",
);
installGeneratedFiles([
  { source: builtPikg, target: rootfsPikg },
  { source: join(dappDistDir, "APPDOC.json"), target: rootfsAppDoc },
]);
runPikgTool(["pikg", "info", rootfsPikg]);
