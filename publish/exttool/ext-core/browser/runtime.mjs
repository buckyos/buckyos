import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

export const here = path.dirname(fileURLToPath(import.meta.url));
// In a payload this directory is store/ext-core/browser, next to playwright/ and conda/.
export const store = path.dirname(here);

const require = createRequire(import.meta.url);

// versions.json written by the build: { playwright, chromium, revision, binary }. Absent in development.
function packaged() {
  const file = path.join(here, 'versions.json');
  return fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : null;
}

// The playwright-core shipped with the payload, else whatever module resolution finds (development).
function playwrightDir() {
  const bundled = path.join(store, 'playwright', 'node_modules', 'playwright-core');
  if (fs.existsSync(path.join(bundled, 'package.json'))) return bundled;
  try {
    return path.dirname(require.resolve('playwright-core/package.json'));
  } catch {
    throw new Error(`playwright-core is neither at ${bundled} nor resolvable as a module`);
  }
}

// The browser is an online item of the package: the node downloads it into <tools>/online/chromium, a Playwright browser
// directory. <tools> is the package itself when that is unpacked as the tools directory, else the directory its staging
// or version directory is in. `binary` is the browser's own executable in there: the payload's overlay does not have it.
function onlineBrowsers() {
  const binary = packaged()?.binary;
  if (!binary) return null;
  const root = path.resolve(store, '../..');
  const tools = [root];
  if (path.basename(path.dirname(root)) === 'staging') tools.push(path.resolve(root, '../..'));
  else if (path.basename(path.resolve(root, '../..')) === 'versions') tools.push(path.resolve(root, '../../..'));
  return tools.map((dir) => path.join(dir, 'online', 'chromium')).find((browsers) => fs.existsSync(path.join(browsers, binary))) ?? null;
}

export function loadPlaywright() {
  const browsers = onlineBrowsers();
  if (browsers) {
    process.env.PLAYWRIGHT_BROWSERS_PATH ||= browsers;
    // The node's browser finds its libraries through its wrapper script, not through the system's paths.
    if (process.env.PLAYWRIGHT_BROWSERS_PATH === browsers) process.env.PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS ??= '1';
  }
  return require(playwrightDir());
}

// Whether the browser `playwright` is about to start is on this machine. A payload starts the node's own, or the one
// PLAYWRIGHT_BROWSERS_PATH names, and never one in the account's Playwright cache.
export function browserIsHere(playwright) {
  if (packaged()?.binary && !process.env.PLAYWRIGHT_BROWSERS_PATH) return false;
  return fs.existsSync(playwright.chromium.executablePath());
}

export const BROWSER_NOT_HERE =
  'Chrome for Testing is not downloaded on this node yet: the owner downloads it in Desktop Settings > Extension tools, or with `buckyos ext-tools download ext-core chromium`';

// { playwright, chromium, revision }: versions.json written by the build, else read from playwright-core itself.
export function readVersions() {
  const versions = packaged();
  if (versions) return versions;
  const dir = playwrightDir();
  const read = (name) => JSON.parse(fs.readFileSync(path.join(dir, name), 'utf8'));
  const chromium = read('browsers.json').browsers.find((b) => b.name === 'chromium');
  return { playwright: read('package.json').version, chromium: chromium.browserVersion, revision: chromium.revision };
}
