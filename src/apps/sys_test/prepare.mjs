import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  rmSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

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
