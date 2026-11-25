import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, isAbsolute, join, relative, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const rootDir = dirname(fileURLToPath(import.meta.url));
const distDir = join(rootDir, "dist");
const webDistDir = join(rootDir, "web", "dist");
const sdkPackageDir = join(rootDir, "node_modules", "buckyos");
const sdkDistDir = join(sdkPackageDir, "dist");
const sdkTargetDir = join(distDir, "node_modules", "buckyos");

if (!existsSync(webDistDir)) {
  throw new Error(`missing web dist: ${webDistDir}`);
}
if (!existsSync(sdkDistDir)) {
  throw new Error(`missing buckyos npm dist: ${sdkDistDir}`);
}

const sdkRoot = realpathSync(sdkDistDir);
const sdkEntry = pathToFileURL(join(sdkRoot, "node.mjs")).href;
const graph = JSON.parse(execFileSync("deno", [
  "info",
  "--json",
  "--no-config",
  "--no-lock",
  "--no-npm",
  "--no-remote",
  sdkEntry,
], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 }));
const modules = new Map(
  graph.modules.map((module) => [module.specifier, module]),
);
const runtimeFiles = new Set();
const visited = new Set();
const pending = [sdkEntry];
while (pending.length > 0) {
  const specifier = pending.pop();
  if (visited.has(specifier) || specifier.startsWith("node:")) continue;
  visited.add(specifier);
  const module = modules.get(specifier);
  if (!module || module.error) {
    throw new Error(
      `invalid SDK runtime module ${specifier}: ${module?.error ?? "missing"}`,
    );
  }
  if (!specifier.startsWith("file:")) {
    throw new Error(`SDK runtime dependency is not bundled: ${specifier}`);
  }
  const source = realpathSync(fileURLToPath(specifier));
  const path = relative(sdkRoot, source);
  if (isAbsolute(path) || path === ".." || path.startsWith(`..${sep}`)) {
    throw new Error(`SDK runtime dependency is outside dist: ${source}`);
  }
  if (path.endsWith(".map") || /\.d\.[cm]?ts$/.test(path)) {
    throw new Error(`SDK runtime imports a development file: ${source}`);
  }
  runtimeFiles.add(path);
  for (const dependency of module.dependencies ?? []) {
    if (!dependency.code) continue;
    if (dependency.code.error) {
      throw new Error(
        `invalid SDK runtime dependency ${dependency.specifier}: ${dependency.code.error}`,
      );
    }
    pending.push(dependency.code.specifier);
  }
}

rmSync(distDir, { recursive: true, force: true });
mkdirSync(distDir, { recursive: true });

copyFileSync(join(rootDir, "resource_share.ts"), join(distDir, "resource_share.ts"));
copyFileSync(join(rootDir, "main.ts"), join(distDir, "main.ts"));
copyFileSync(join(rootDir, "static_web.ts"), join(distDir, "static_web.ts"));
copyFileSync(
  join(rootDir, "xllm_selftest.ts"),
  join(distDir, "xllm_selftest.ts"),
);
copyFileSync(
  join(rootDir, "support_group.ts"),
  join(distDir, "support_group.ts"),
);
const denoConfig = JSON.parse(readFileSync(join(rootDir, "deno.json"), "utf8"));
denoConfig.imports["@sys-test/websdk-node-types"] =
  "./node_modules/buckyos/dist/node.mjs";
denoConfig.vendor = true;
writeFileSync(
  join(distDir, "deno.json"),
  `${JSON.stringify(denoConfig, null, 2)}\n`,
);
cpSync(webDistDir, join(distDir, "web"), {
  recursive: true,
  filter: (source) => !source.endsWith(".map"),
});
mkdirSync(sdkTargetDir, { recursive: true });
copyFileSync(
  join(sdkPackageDir, "package.json"),
  join(sdkTargetDir, "package.json"),
);
for (const path of runtimeFiles) {
  const target = join(sdkTargetDir, "dist", path);
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(join(sdkRoot, path), target);
}
execFileSync("deno", ["cache", "main.ts"], { cwd: distDir, stdio: "inherit" });
const jsrDir = join(distDir, "vendor", "jsr.io");
for (const file of existsSync(jsrDir) ? readdirSync(jsrDir, { recursive: true }) : []) {
  const module = file.match(/^(.+)_meta\.json$/)?.[1];
  if (!module) continue;
  const { manifest } = JSON.parse(readFileSync(join(jsrDir, file), "utf8"));
  if (!manifest["/LICENSE"]) continue;
  const url = `https://jsr.io/${module.replaceAll(sep, "/")}/LICENSE`;
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`license of a vendored module: ${url}: ${response.status}`);
  }
  writeFileSync(join(jsrDir, module, "LICENSE"), await response.text());
}
