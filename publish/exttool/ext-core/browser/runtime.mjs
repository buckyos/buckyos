import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

export const here = path.dirname(fileURLToPath(import.meta.url));
// In a payload this directory is store/ext-core/browser, next to chromium/, playwright/ and conda/.
export const store = path.dirname(here);

const require = createRequire(import.meta.url);

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

export function loadPlaywright() {
  const browsers = path.join(store, 'chromium');
  if (fs.existsSync(browsers)) {
    process.env.PLAYWRIGHT_BROWSERS_PATH ??= browsers;
    // The payload's browser finds its libraries through its wrapper script, not through the system's paths.
    if (process.env.PLAYWRIGHT_BROWSERS_PATH === browsers) process.env.PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS ??= '1';
  }
  return require(playwrightDir());
}

// { playwright, chromium, revision }: versions.json written by the build, else read from playwright-core itself.
export function readVersions() {
  const file = path.join(here, 'versions.json');
  if (fs.existsSync(file)) return JSON.parse(fs.readFileSync(file, 'utf8'));
  const dir = playwrightDir();
  const read = (name) => JSON.parse(fs.readFileSync(path.join(dir, name), 'utf8'));
  const chromium = read('browsers.json').browsers.find((b) => b.name === 'chromium');
  return { playwright: read('package.json').version, chromium: chromium.browserVersion, revision: chromium.revision };
}
