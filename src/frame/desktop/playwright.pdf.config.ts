import { existsSync } from 'node:fs'
import { resolve } from 'node:path'
import { defineConfig } from '@playwright/test'
import workspace from './playwright.aiworkspace.config'

const input = process.env.AIWS_PDF_FILE
if (!input || !existsSync(input)) throw new Error('Set AIWS_PDF_FILE to the PDF to compare')
for (const name of ['AIWS_BIN', 'NFS_BIN']) {
  if (!process.env[name] || !existsSync(process.env[name]!)) throw new Error(`${name} must point to a prebuilt isolated-test binary`)
}
const output = resolve(process.env.AIWS_PDF_OUTPUT ?? 'test-results/pdf-fidelity')
export default defineConfig({
  ...workspace,
  testMatch: 'pdf-fidelity.spec.ts',
  timeout: 120_000,
  outputDir: `${output}/artifacts`,
  reporter: [['list'], ['json', { outputFile: `${output}/results.json` }], ['html', { outputFolder: `${output}/html`, open: 'never' }]],
  webServer: Array.isArray(workspace.webServer) ? workspace.webServer.slice(0, 2) : workspace.webServer,
  projects: ['editable', 'raster'].map(name => ({ name, use: { ...workspace.use, launchOptions: { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH } } })),
})
