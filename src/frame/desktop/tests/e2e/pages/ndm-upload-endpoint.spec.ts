import { expect, test, type Page } from '@playwright/test'

type Call = { method: string; endpoint?: string; sessionId?: string; sessionToken?: string }

declare global {
  interface Window {
    __uploadEndpointTest: { calls: Call[]; failNext: boolean; buckyos: object; ndm: object; bucket: object }
  }
}

async function prepare(page: Page, serviceUrl: string, override?: { token: string; baseUrl?: string }) {
  await page.route('**/__upload_endpoint_test', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>Upload endpoint regression</title>' }))
  await page.route(/\/(?:buckyos\.js|(?:buckyos|buckyos-websdk)\/dist\/browser\.mjs)(?:\?.*)?$/, route => route.fulfill({
    contentType: 'text/javascript',
    body: 'export const { buckyos, ndm, bucket } = window.__uploadEndpointTest; export const accessDenialOf = () => null;',
  }))
  await page.route('**/src/runtime.ts*', route => route.fulfill({
    contentType: 'text/javascript',
    body: 'export const DESKTOP_USE_MOCK = false; export const isMockRuntime = () => false; export const waitForMockLatency = async () => {};',
  }))
  await page.goto('/__upload_endpoint_test')
  await page.evaluate(({ serviceUrl, override }) => {
    if (override) localStorage.setItem('homestation.dev', JSON.stringify(override))
    const calls: Call[] = []
    const object = { objectId: 'cyfile:test-upload', name: 'attachment.txt', size: 5 }
    const rpc = async (method: string) => {
      calls.push({ method })
      if (method === 'object.register') return { objId: object.objectId }
      if (method === 'apps.staging.finalize') return { handle: 'staged-test' }
      if (method === 'apps.inspect') return { plan: {}, status: {} }
      throw new Error(`Unexpected RPC: ${method}`)
    }
    window.__uploadEndpointTest = {
      calls,
      failNext: false,
      buckyos: {
        getZoneServiceURL: () => serviceUrl,
        getAccountInfo: async () => ({ session_token: 'test-token', user_id: 'alice' }),
        getServiceRpcClient: () => ({ getSessionToken: () => 'test-token', call: rpc }),
      },
      ndm: {
        importFiles: async (_files: File[], options: { endpoint: string; sessionToken: string }) => {
          calls.push({ method: 'importFiles', endpoint: options.endpoint, sessionToken: options.sessionToken })
          return { sessionId: 'import-test', items: [object], selection: object }
        },
        ensureUploaded: async (sessionId: string, options: { endpoint: string; sessionToken: string }) => {
          calls.push({ method: 'ensureUploaded', sessionId, endpoint: options.endpoint, sessionToken: options.sessionToken })
          if (window.__uploadEndpointTest.failNext) {
            window.__uploadEndpointTest.failNext = false
            throw new Error('transient upload failure')
          }
          return { objectStatus: 'ready' }
        },
        releaseImportSession: (sessionId: string) => calls.push({ method: 'releaseImportSession', sessionId }),
      },
      bucket: { userBucketId: (appid: string, owner: string, name: string) => `user/${appid}/${owner}/${name}` },
    }
    const originalFetch = window.fetch.bind(window)
    window.fetch = async (input, init) => {
      if (new URL(String(input), location.origin).pathname === '/kapi/homestation') {
        const { method } = JSON.parse(String(init?.body))
        return Response.json({ result: await rpc(method) })
      }
      return originalFetch(input, init)
    }
  }, { serviceUrl, override })
}

const variants = [
  { label: 'relative without trailing slash', base: '', suffix: '' },
  { label: 'relative with trailing slash', base: '', suffix: '/' },
  { label: 'absolute with port', base: 'https://zone.example:8443', suffix: '/' },
]

for (const entry of ['homestation', 'messagehub', 'playground', 'pikg'] as const) {
  for (const variant of variants) {
    test(`${entry} passes an absolute NDM endpoint for ${variant.label}`, async ({ page }) => {
      const service = entry === 'homestation' ? 'homestation' : 'control-panel'
      await prepare(page, `${variant.base}/kapi/${service}${variant.suffix}`)
      const calls = await page.evaluate(async entry => {
        const file = new File(['bytes'], 'attachment.txt', { type: 'text/plain' })
        const signal = new AbortController().signal
        if (entry === 'homestation') {
          const path = '/src/app/homestation/api/transport.ts'
          const { homeStationTransport } = await import(path)
          await homeStationTransport().upload(file, { name: file.name, mime: file.type })
        } else if (entry === 'messagehub') {
          const path = '/src/app/messagehub/api/upload.ts'
          const { uploadAttachments, releaseAttachmentUploads } = await import(path)
          await uploadAttachments([{ file }])
          releaseAttachmentUploads([{ file }])
        } else if (entry === 'playground') {
          const path = '/src/app/ai-center/api/resourceImport.ts'
          const { importPlaygroundResource } = await import(path)
          await importPlaygroundResource(file, signal)
        } else {
          const path = '/src/app/app-service/realPikg.ts'
          const { inspectPikgFile } = await import(path)
          await inspectPikgFile(file, signal, () => {})
        }
        return window.__uploadEndpointTest.calls
      }, entry)
      const endpoint = variant.base || new URL(page.url()).origin
      expect(calls.filter(call => call.endpoint !== undefined)).toEqual([
        { method: 'importFiles', endpoint, sessionToken: 'test-token' },
        { method: 'ensureUploaded', endpoint, sessionToken: 'test-token', sessionId: 'import-test' },
      ])
      expect(calls.filter(call => call.method === 'releaseImportSession')).toHaveLength(1)
      if (entry === 'homestation') expect(calls.map(call => call.method)).toEqual(['importFiles', 'ensureUploaded', 'object.register', 'releaseImportSession'])
    })
  }
}

for (const baseUrl of [undefined, '/kapi/homestation/', 'https://dev.example:9443/kapi/homestation/']) {
  test(`HomeStation dev override resolves ${baseUrl ?? 'its default relative endpoint'}`, async ({ page }) => {
    await prepare(page, '/kapi/homestation/', { token: 'override-token', baseUrl })
    const result = await page.evaluate(async () => {
      const path = '/src/app/homestation/api/transport.ts'
      const { homeStationTransport, setContentHome } = await import(path)
      setContentHome('me')
      const transport = homeStationTransport()
      await transport.upload(new File(['bytes'], 'attachment.txt'), { name: 'attachment.txt', mime: 'text/plain' })
      return { calls: window.__uploadEndpointTest.calls, contentUrl: transport.contentUrl('cyfile:test-upload') }
    })
    const endpoint = baseUrl?.startsWith('https:') ? 'https://dev.example:9443' : new URL(page.url()).origin
    expect(result.calls.filter(call => call.endpoint !== undefined)).toEqual([
      { method: 'importFiles', endpoint, sessionToken: 'override-token' },
      { method: 'ensureUploaded', endpoint, sessionToken: 'override-token', sessionId: 'import-test' },
    ])
    expect(result.contentUrl).toBe(`${baseUrl?.startsWith('https:') ? endpoint : ''}/home/me/objects/cyfile%3Atest-upload/content?access=override-token`)
  })
}

for (const mode of ['zone', 'dev-override'] as const) {
  test(`HomeStation ${mode} retries the same import and registers only after upload succeeds`, async ({ page }) => {
    await prepare(page, '/kapi/homestation/', mode === 'dev-override' ? { token: 'test-token' } : undefined)
    const result = await page.evaluate(async () => {
      const path = '/src/app/homestation/api/transport.ts'
      const { homeStationTransport } = await import(path)
      const transport = homeStationTransport()
      const file = new File(['bytes'], 'attachment.txt')
      const meta = { name: file.name, mime: 'text/plain' }
      window.__uploadEndpointTest.failNext = true
      let error = ''
      try { await transport.upload(file, meta) } catch (failure) { error = String(failure) }
      const failedCalls = [...window.__uploadEndpointTest.calls]
      const uploaded = await transport.upload(file, meta)
      return { error, failedCalls, uploaded, calls: window.__uploadEndpointTest.calls }
    })
    expect(result.error).toContain('transient upload failure')
    expect(result.failedCalls.map(call => call.method)).toEqual(['importFiles', 'ensureUploaded'])
    expect(result.uploaded).toEqual({ objId: 'cyfile:test-upload' })
    expect(result.calls.map(call => call.method)).toEqual(['importFiles', 'ensureUploaded', 'ensureUploaded', 'object.register', 'releaseImportSession'])
    expect(result.calls.filter(call => call.method === 'ensureUploaded')).toEqual(Array(2).fill({
      method: 'ensureUploaded', sessionId: 'import-test', endpoint: new URL(page.url()).origin, sessionToken: 'test-token',
    }))
  })
}
