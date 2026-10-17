import { isIP } from 'node:net'
import { resolve } from 'node:path'
import { defineConfig, devices } from '@playwright/test'
import dv from './playwright.dv.config'

const baseURL = process.env.BUCKYOS_UI_DV_BASE_URL || 'https://test.buckyos.io'
const gatewayIp = process.env.BUCKYOS_UI_DV_GATEWAY_IP
if (gatewayIp && !isIP(gatewayIp)) throw new Error('BUCKYOS_UI_DV_GATEWAY_IP must be an IP address')
const spki = process.env.BUCKYOS_UI_DV_TLS_SPKI
if (spki && !/^[A-Za-z0-9+/]{43}=$/.test(spki)) throw new Error('BUCKYOS_UI_DV_TLS_SPKI must be a SHA-256 SPKI hash in base64')
const host = new URL(baseURL).hostname
const address = gatewayIp?.includes(':') ? `[${gatewayIp}]` : gatewayIp
const outputDir = resolve(process.env.BUCKYOS_E2E_OUTPUT_DIR || 'test-results/aiworkspace-zone')

export default defineConfig(dv, {
  testMatch: ['aiworkspace.spec.ts', 'aiworkspace.mobile.spec.ts', 'aiworkspace.journeys.spec.ts', 'aiworkspace.logic.spec.ts', 'aiworkspace.integrity.spec.ts'],
  projects: [
    { name: 'chromium-real-zone', testMatch: ['aiworkspace.spec.ts', 'aiworkspace.journeys.spec.ts', 'aiworkspace.logic.spec.ts', 'aiworkspace.integrity.spec.ts'], use: { ...devices['Desktop Chrome'], viewport: { width: 1600, height: 1000 } } },
    { name: 'chromium-real-zone-mobile', testMatch: 'aiworkspace.mobile.spec.ts', use: { ...devices['Pixel 7'], viewport: { width: 375, height: 812 } } },
  ],
  outputDir,
  reporter: [['line'], ['json', { outputFile: resolve(outputDir, 'report.json') }]],
  use: {
    baseURL,
    viewport: { width: 1600, height: 1000 },
    launchOptions: {
      args: [
        ...(address ? [`--host-resolver-rules=MAP ${host} ${address}, MAP *.${host} ${address}`] : []),
        ...(spki ? [`--ignore-certificate-errors-spki-list=${spki}`] : []),
      ],
    },
  },
})
