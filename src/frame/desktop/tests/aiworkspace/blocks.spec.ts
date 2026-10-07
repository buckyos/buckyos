import type { Page } from '@playwright/test'
import { test, expect, openCanvas, type Api, fitAll, setCanvasMode } from './fixtures'
import type { BlockDefPayload } from '../../src/app/aiworkspace/api/types'

const ALICE = 'tok-alice', BOB = 'tok-bob'
async function htmlSetup(page: Page, api: Api, extra: Partial<BlockDefPayload> = {}, noSource = false) {
  const ws = await api.demo(ALICE, 'quarterly', `review ${Date.now()}`)
  const r = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'def-ext', type_id: 'buckyos.block-def', parent_id: 'data', order_key: 'zx', payload: { def_id: 'review.ext', version: 1, kind: 'html', title: 'Review', html: { html: '<div id="ready">ready</div>', js: 'aiws.ready()' }, ...extra } },
    { op: 'entity.create', entity_id: 'blk-ext', type_id: 'buckyos.cell', parent_id: 'sf-analysis', order_key: 'zzx', placement: { x: 40, y: 720, w: 400, h: 200 }, payload: { view: { type: 'html', version: 1 }, ...(noSource ? {} : { source_ref: { entity_id: 'sales' } }), def_ref: { entity_id: 'def-ext' }, title: 'Review' } },
  ])
  expect(r.status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.getByTestId('aiws-canvas-block-blk-ext').click()
  return ws
}

test('startup timeout rejects the original mount promise', async ({ page }) => {
  await page.goto('/?scenario=normal')
  const result = await page.evaluate(async () => {
    const path = '/src/app/aiworkspace/ui/extensions/htmlRuntime.ts'
    const { HtmlRuntime } = await import(path) as typeof import('../../src/app/aiworkspace/ui/extensions/htmlRuntime')
    const rt = new HtmlRuntime({ html: '<p>no ready</p>' }, { context: () => ({}), read: async () => null, query: async () => null, submit: async () => null, upload: async () => ({ object_id: '', media_type: '', size: 0 }), snapshot: async () => undefined, notify: () => undefined })
    let settled = 'pending', crash = ''
    rt.onCrash = message => { crash = message }
    rt.mount(null).then(() => { settled = 'resolved' }, () => { settled = 'rejected' })
    await new Promise(resolve => setTimeout(resolve, 8400))
    rt.dispose()
    return { settled, crash }
  })
  expect(result.settled).toBe('rejected')
})

test('snapshot remains readable to collaborators and after package import', async ({ page, api }) => {
  const ws = await htmlSetup(page, api, { config_schema: { type: 'object', additionalProperties: false } })
  await page.getByTestId('aiws-near-run').click()
  await expect(page.frameLocator('.aiws-html-frame').locator('#ready')).toHaveText('ready')
  await page.evaluate(async () => {
    const win = (document.querySelector('iframe.aiws-html-frame') as HTMLIFrameElement).contentWindow as unknown as { aiws: { snapshot: (url: string) => Promise<void> } }
    await win.aiws.snapshot('data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><text>中文快照</text></svg>'))
  })
  const read = await api.read(ALICE, ws.workspace_id, 'blk-ext')
  const id = read.content.payload.config.snapshot.object_id
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  const response = await fetch(`${api.base}/asset/${ws.workspace_id}/${id}`, { headers: { authorization: `Bearer ${BOB}` } })
  expect(await response.text()).toContain('中文快照')
  const ex = await api.rpc(ALICE, 'doc.export', { workspace_id: ws.workspace_id, mode: 'share', self_contained: true })
  const pkg = await (await fetch(`${api.base}/export/${ws.workspace_id}/${ex.export_id}`, { headers: { authorization: `Bearer ${ALICE}` } })).arrayBuffer()
  expect(ex.manifest.objects.some((object: { id: string }) => object.id === id)).toBe(true)
  const begin = await api.rpc(ALICE, 'ws.begin_import', {})
  await fetch(`${api.base}/upload/${begin.upload_id}`, { method: 'PUT', headers: { authorization: `Bearer ${ALICE}` }, body: Buffer.from(pkg) })
  const imported = await api.rpc(ALICE, 'ws.import', { upload_id: begin.upload_id, semantics: 'new' })
  expect(imported.ok).toBe(true)
  const importedStatus = (await fetch(`${api.base}/asset/${imported.workspace_id}/${id}`, { headers: { authorization: `Bearer ${ALICE}` } })).status
  expect.soft(response.status).toBe(200)
  expect(importedStatus).toBe(200)
  await page.getByTestId('aiws-html-active-blk-ext').getByRole('button', { name: '停止', exact: true }).click()
  const snapshot = page.getByTestId('aiws-html-static-blk-ext').locator('img')
  await expect.poll(() => snapshot.evaluate((image: HTMLImageElement) => image.naturalWidth)).toBe(40)
})

for (const scenario of [
  { title: 'source type', extra: { accepts: ['buckyos.record'] }, reason: 'type_not_accepted' },
  { title: 'empty accepts', extra: { accepts: [] }, reason: 'type_not_accepted' },
  { title: 'required source', extra: { allow_no_source: false }, noSource: true, reason: 'source_required' },
  { title: 'API version', extra: { html: { html: '<div/>', js: 'aiws.ready()', api_version: 99 } }, reason: 'unsupported_api' },
  { title: 'config schema', extra: { config_schema: { type: 'object', properties: { count: { type: 'number' } }, required: ['count'] } }, reason: 'invalid_config' },
] satisfies { title: string; extra: Partial<BlockDefPayload>; noSource?: boolean; reason: string }[]) {
  test(`document definition enforces ${scenario.title} before running code`, async ({ page, api }) => {
    await htmlSetup(page, api, scenario.extra, 'noSource' in scenario && scenario.noSource)
    await expect(page.getByTestId('aiws-block-fallback-blk-ext')).toHaveAttribute('data-reason', scenario.reason)
    await expect(page.getByTestId('aiws-near-run')).toHaveCount(0)
    await page.keyboard.press('Enter')
    await expect(page.locator('.aiws-html-frame')).toHaveCount(0)
    await expect(page.getByTestId('aiws-workspace')).toBeVisible()
  })
}

test('HTML open-definition action receives def_ref', async ({ page, api }) => {
  await htmlSetup(page, api)
  await expect(page.getByTestId('aiws-near-open-def')).toBeVisible()
})

test('HTML retry mounts a new runtime', async ({ page, api }) => {
  await htmlSetup(page, api)
  await page.getByTestId('aiws-near-run').click()
  await expect(page.frameLocator('.aiws-html-frame').locator('#ready')).toHaveText('ready')
  await page.evaluate(() => {
    ((document.querySelector('iframe.aiws-html-frame') as HTMLIFrameElement).contentWindow as Window & { eval: (code: string) => void }).eval('setTimeout(function(){ throw new Error("review crash") }, 0)')
  })
  await expect(page.getByTestId('aiws-html-failed-blk-ext')).toBeVisible()
  await page.getByRole('button', { name: '重新运行', exact: true }).click()
  await expect(page.locator('.aiws-html-frame')).toHaveCount(1)
  await expect(page.frameLocator('.aiws-html-frame').locator('#ready')).toHaveText('ready')
})

test('unregistering a renderer switches mounted blocks to fallback', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `unregister ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-decl-blk-kpi')).toBeVisible()
  await page.evaluate(async () => {
    const path = '/src/app/aiworkspace/ui/blocks/registry.ts'
    const { blockRegistry } = await import(path) as typeof import('../../src/app/aiworkspace/ui/blocks/registry')
    blockRegistry.unregister('declarative', 1)
  })
  await expect(page.getByTestId('aiws-block-fallback-blk-kpi')).toHaveAttribute('data-reason', 'unknown_renderer')
  await page.evaluate(async () => {
    const registryPath = '/src/app/aiworkspace/ui/blocks/registry.ts'
    const definitionPath = '/src/app/aiworkspace/ui/extensions/declarative.tsx'
    const { blockRegistry } = await import(registryPath) as typeof import('../../src/app/aiworkspace/ui/blocks/registry')
    const { declarativeBlock } = await import(definitionPath) as typeof import('../../src/app/aiworkspace/ui/extensions/declarative')
    blockRegistry.register(declarativeBlock)
  })
  await expect(page.getByTestId('aiws-decl-blk-kpi')).toBeVisible()
  await expect(page.getByTestId('aiws-block-fallback-blk-kpi')).toHaveCount(0)
})

test('a throwing Inspector is isolated to its block', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `inspector ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-decl-blk-kpi')).toBeVisible()
  await page.evaluate(async () => {
    const path = '/src/app/aiworkspace/ui/blocks/registry.ts'
    const { blockRegistry } = await import(path) as typeof import('../../src/app/aiworkspace/ui/blocks/registry')
    const original = blockRegistry.get('declarative', 1)!
    blockRegistry.register({ ...original, Inspector: () => { throw new Error('review inspector crash') } })
  })
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  await page.getByTestId('aiws-near-inspector').click()
  await expect(page.getByTestId('aiws-inspector-error')).toContainText('review inspector crash')
  await expect(page.getByTestId('aiws-workspace')).toBeVisible()
  await page.getByTestId('aiws-canvas-block-blk-sales').click()
  await expect(page.getByTestId('aiws-near-toolbar')).toBeVisible()
})

test('nested richtext blocks respect MAX_EMBED_DEPTH', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `depth ${Date.now()}`)
  const ops = []
  for (let i = 6; i >= 0; i--) {
    const content = i === 6
      ? [{ type: 'paragraph', attrs: { block_id: `text-${i}` }, content: [{ type: 'text', text: 'should be depth-limited' }] }]
      : [{ type: 'object_embed', attrs: { block_id: `embed-${i}`, ref: { entity_id: `deep-c-${i+1}` } } }]
    ops.push({ op: 'entity.create', entity_id: `deep-r-${i}`, type_id: 'buckyos.richtext', parent_id: 'data', order_key: `z${i}a`, payload: { content: { type: 'doc', content } } })
    ops.push({ op: 'entity.create', entity_id: `deep-c-${i}`, type_id: 'buckyos.cell', parent_id: i === 0 ? 'sf-analysis' : 'sf-detail', order_key: `z${i}a`, placement: { x: 40, y: 720, w: 400, h: 200 }, payload: { view: { type: 'richtext' }, source_ref: { entity_id: `deep-r-${i}` } } })
  }
  const committed = await api.commit(ALICE, ws, ops)
  expect(committed.status, JSON.stringify(committed)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await expect(page.getByTestId('aiws-canvas-block-deep-c-0')).toContainText('嵌入层级过深')
  await expect(page.getByTestId('aiws-embed-deep-c-6')).toHaveCount(0)
  const count = await page.locator('[data-testid^="aiws-embed-deep-c-"]').count()
  expect(count).toBeLessThanOrEqual(3)
})

test('actions use the current payload and key revisions and respect view mode', async ({ page, api }) => {
  const ws = await api.demo(ALICE, 'quarterly', `actions ${Date.now()}`)
  const read = await api.read(ALICE, ws.workspace_id, 'blk-kpi')
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'blk-kpi', keys: [{ key: 'config', value: { accent: '#123456' }, expect: { rev: read.content.key_revs.config ?? 0 } }] }])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await page.evaluate(async () => {
    const path = '/src/app/aiworkspace/ui/blocks/registry.ts'
    const { blockRegistry } = await import(path) as typeof import('../../src/app/aiworkspace/ui/blocks/registry')
    blockRegistry.register({ ...blockRegistry.get('declarative', 1)!, actions: [{
      id: 'write-config', label: '修改配置', modes: ['edit', 'view'], needs: ['update'],
      when: (context) => context.payload.def_ref?.entity_id === 'def-kpi' && context.payload.config?.accent === '#123456' && context.keyRevs.config > 0,
      run: async (context, store) => {
        await store.submit({ editId: 'action-config', label: '配置动作', operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: { ...context.payload.config, note: 'from action' }, expect: { rev: context.keyRevs.config } }] }] })
      },
    }] })
  })
  await page.getByTestId('aiws-canvas-block-blk-kpi').click()
  await page.getByTestId('aiws-near-write-config').click()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'blk-kpi')).content.payload.config).toEqual({ accent: '#123456', note: 'from action' })
  await setCanvasMode(page, 'view')
  await expect(page.getByTestId('aiws-near-write-config')).toHaveCount(0)
  await setCanvasMode(page, 'edit')
  await expect(page.getByTestId('aiws-near-write-config')).toBeVisible()
})

test('disposing before ready rejects mount and pending requests', async ({ page }) => {
  await page.goto('/?scenario=normal')
  const result = await page.evaluate(async () => {
    const path = '/src/app/aiworkspace/ui/extensions/htmlRuntime.ts'
    const { HtmlRuntime } = await import(path) as typeof import('../../src/app/aiworkspace/ui/extensions/htmlRuntime')
    const runtime = new HtmlRuntime({ html: '<div/>' }, { context: () => ({}), read: async () => null, query: async () => null, submit: async () => null, upload: async () => ({ object_id: '', media_type: '', size: 0 }), snapshot: async () => undefined, notify: () => undefined })
    const mount = runtime.mount(null)
    const request = runtime.request('analyze', {})
    runtime.dispose()
    return { results: (await Promise.allSettled([mount, request])).map((r) => r.status), frames: document.querySelectorAll('.aiws-html-frame').length }
  })
  expect(result).toEqual({ results: ['rejected', 'rejected'], frames: 0 })
})
