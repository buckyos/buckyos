import { defineConfig } from '@playwright/test'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
process.env.TEXT_EDITOR_NFSP_ROOT ??= mkdtempSync(join(tmpdir(), 'text-editor-nfsp-'))
export default defineConfig({ testDir: './tests/real-nfsp', workers: 1, use: { baseURL: 'http://127.0.0.1:5179', trace: 'retain-on-failure' }, webServer: [
  { command: 'node tests/harness/real-nfsp.mjs', port: 3261, reuseExistingServer: false },
  { command: 'pnpm exec vite --mode test --host 127.0.0.1 --port 5179', port: 5179, env: { VITE_NFSP_TARGET: 'http://127.0.0.1:3261' }, reuseExistingServer: false },
] })
