/* Extension controls on a real Zone (notepads/aiworkspace-extension-p0-todo.md M5–M6): the owner creates a control
 * in the Desktop's AI Workspace window — its source goes into the owner's home over the Zone file service, `ext.build`
 * commits it — inserts an instance bound to a table, rebuilds, and makes it
 * available in the library; a second account builds its own control from inline files into its own workspace and
 * libraries it too; both bring their control into one shared workspace from their own libraries, both see the two
 * controls, and the peer instantiates the owner's without having installed its package (D3). The second case uses the
 * CLI alone: a local control folder built (a change rebuilt, a broken one refused), exported, packed, installed into
 * the library, imported elsewhere, and the same package installed through the App Installer (no App deployed). */

import { execFileSync } from 'node:child_process'
import { createHash, randomBytes } from 'node:crypto'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import type { EntityEnvelope, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
type Json = Record<string, unknown>

test.use({ viewport: { width: 1600, height: 1000 } })

const b64 = (text: string) => Buffer.from(text).toString('base64')

async function outline(actor: Actor, ws: Workspace) {
  return (await actor.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id })).entities
}

/** A workspace with a `tasks` table (4 rows) and a free canvas, made through the API. */
async function workspaceWithTasks(actor: Actor): Promise<Workspace> {
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(created.workspace_id)
  const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
  const result = await actor.client.commit(ws, [
    { op: 'entity.create', entity_id: 'tasks', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'a', name: 'tasks', payload: { fields: [{ field_id: 'title', name: 'title', type: 'text' }] } },
    { op: 'table.insert_records', source_id: 'tasks', records: ['a', 'b', 'c', 'd'].map((id) => ({ record_id: id, values: { title: `task ${id}` } })) },
    { op: 'entity.create', entity_id: 'sf-main-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'sf-main', title: 'Main' } },
    { op: 'entity.create', entity_id: 'sf-main', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Main', content_folder_id: 'sf-main-content' } },
  ] as Operation[])
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return ws
}

/** Open a workspace inside the Desktop's AI Workspace window (its origin reaches the user's home over /nfs/v1). */
async function openInDesktop(actor: Actor, ws: Workspace) {
  const page = actor.page
  await page.getByTestId('desktop-app-aiworkspace').click()
  await page.getByTestId('window-aiworkspace').getByRole('button', { name: /^(Maximize|最大化)$/ }).click()
  const back = page.getByTestId('aiws-workspace')
  if (await back.isVisible().catch(() => false)) { await page.getByTestId('aiws-main-menu').click(); await page.getByTestId('aiws-back').click() }
  await page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${ws.workspace_id}"]`).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id, { timeout: 30_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  if (await page.getByTestId('aiws-top-canvas').isVisible().catch(() => false)) await page.getByTestId('aiws-top-canvas').click()
}

async function mainMenu(actor: Actor, ...ids: string[]) {
  await actor.page.getByTestId('aiws-main-menu').click()
  for (const id of ids) await actor.page.getByTestId(id).click()
}

test('[ext] a control from home in the Desktop, an instance, a rebuild, the library, and two accounts sharing their controls', async ({ owner, peer }, info) => {
  test.setTimeout(480_000)
  const tag = randomBytes(4).toString('hex')
  const name = `dvkpi${tag}`
  const page = owner.page

  // 1. the owner, in the Desktop window: a new control, built from the skeleton in the owner's home
  const ws = await workspaceWithTasks(owner)
  await openInDesktop(owner, ws)
  await mainMenu(owner, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  const dialog = page.getByTestId('aiws-ext-dialog')
  await dialog.getByTestId('aiws-ext-new').click()
  await dialog.getByTestId('aiws-ext-new-name').fill(name)
  await dialog.getByTestId('aiws-ext-new-title').fill('DV 任务数')
  await dialog.getByTestId('aiws-ext-new-create').click()
  await expect(dialog.getByTestId('aiws-ext-ident')).toHaveText(`did:bns:${owner.userId}#${name}`, { timeout: 60_000 })
  const defId = (await dialog.getByTestId('aiws-ext-detail').getAttribute('data-def'))!
  const dir = await dialog.getByTestId('aiws-ext-dir').inputValue()
  expect(dir).toBe(`/home/${owner.userId}/AI Workspace/${owner.client.prefix}/controls/${name}`)
  await screenshot(page, info, 'ext-built')

  // an instance bound to the sample's table
  await dialog.getByTestId('aiws-ext-insert').click()
  const catalog = page.getByTestId('aiws-catalog')
  await expect(catalog.getByTestId('aiws-catalog-detail')).toHaveAttribute('data-entry', `def:${defId}`)
  await catalog.getByTestId('aiws-instance-binding-data').selectOption('tasks')
  await catalog.getByTestId('aiws-catalog-insert').click()
  await expect(catalog).toBeHidden()
  const cell = (await outline(owner, ws)).find((e) => e.type_id === 'buckyos.cell' && e.def_id === defId)!
  const rows = (await owner.client.ok<{ rows: unknown[] }>('doc.query', { workspace_id: ws.workspace_id, source_id: 'tasks', limit: 500 })).rows.length
  await expect(page.getByTestId(`aiws-decl-${cell.entity_id}`).getByTestId('aiws-decl-metric')).toContainText(String(rows))

  // a rebuild through the source folder: pulled back, then built unchanged (same objects) — and the build record
  await mainMenu(owner, 'aiws-menu-insert', 'aiws-menu-insert-group-extension', 'aiws-menu-extensions')
  await dialog.getByTestId(`aiws-ext-row-${defId}`).click()
  await dialog.getByTestId('aiws-ext-build').click()
  await expect(dialog.getByTestId('aiws-ext-build-ok')).toBeVisible({ timeout: 60_000 })
  const def = await owner.client.ok<{ derived: { build: Json }; content: { payload: Json } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: defId })
  expect(def.derived.build.builder_version).toBe('aiws-ext-build/1')
  await screenshot(page, info, 'ext-rebuilt')

  // into the owner's library
  await dialog.getByTestId(`aiws-ext-check-${defId}`).check()
  await dialog.getByTestId('aiws-ext-export').click()
  await dialog.getByTestId('aiws-ext-export-name').fill(`dv-owner-${tag}`)
  await dialog.getByTestId('aiws-ext-export-library').click()
  await expect(dialog.getByTestId('aiws-ext-export-done')).toContainText(`dv-owner-${tag}`)
  await dialog.getByTestId('aiws-ext-close').click()

  // 2. the peer: a control of its own, built from inline files into its own workspace, then libraried
  const peerWs = await peer.client.ok<WorkspaceSummary>('ws.create', { title: peer.client.prefix })
  const peerName = `dvgauge${tag}`
  const component = { manifest_version: 1, ident: { author_did: `did:bns:${peer.userId}`, name: peerName }, kind: 'html', title: 'DV gauge', allow_no_source: true, entry: { html: 'view.html' } }
  const built = await peer.client.ok<{ def_entity_id: string }>('ext.build', { workspace_id: peerWs.workspace_id, files: [
    { name: 'component.json', base64: b64(JSON.stringify(component)) }, { name: 'view.html', base64: b64('<b>gauge</b>') }] })
  await peer.client.ok('ext.install', { from: { workspace_id: peerWs.workspace_id, entity_ids: [built.def_entity_id] }, package: { name: `dv-peer-${tag}`, version: '1.0.0' } })

  // 3. one shared workspace: each brings its own control from its own library
  const shared = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  owner.client.owned.add(shared.workspace_id)
  await owner.client.ok('ws.grant', { workspace_id: shared.workspace_id, subject: peer.userId, capabilities: ['read', 'update', 'append', 'structure', 'delete', 'comment'] })
  const ownerLib = await owner.client.ok<{ packages: { pkg_id: string; controls: { ident: string }[] }[] }>('ext.list_library', {})
  expect(ownerLib.packages.map((p) => p.pkg_id)).toContain(`dv-owner-${tag}`)
  await owner.client.ok('ext.import', { workspace_id: shared.workspace_id, items: [{ pkg_id: `dv-owner-${tag}`, ident: `did:bns:${owner.userId}#${name}` }] })
  const imported = await peer.client.ok<{ controls: { action: string; def_entity_id: string }[] }>('ext.import', { workspace_id: shared.workspace_id, items: [{ pkg_id: `dv-peer-${tag}`, ident: `did:bns:${peer.userId}#${peerName}` }] })
  expect(imported.controls[0].action).toBe('created')
  const both = (await outline(peer, shared)).filter((e) => e.type_id === 'buckyos.block-def').map((e) => e.ident).sort()
  expect(both).toEqual([`did:bns:${owner.userId}#${name}`, `did:bns:${peer.userId}#${peerName}`].sort())

  // the peer never installed the owner's package, yet instantiates its control in the shared workspace
  const ownerDef = (await outline(peer, shared)).find((e) => e.ident === `did:bns:${owner.userId}#${name}`)!
  const sinfo = await peer.client.info(shared)
  const ops: Operation[] = [
    { op: 'entity.create', entity_id: 'sf-dv-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'sf-dv', title: 'DV' } },
    { op: 'entity.create', entity_id: 'sf-dv', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'DV', content_folder_id: 'sf-dv-content' } },
    { op: 'entity.create', entity_id: 'cell-dv', type_id: 'buckyos.cell', parent_id: 'sf-dv', order_key: 'a', placement: { x: 40, y: 40, w: 260, h: 140 }, payload: { view: { type: 'declarative' }, def_ref: { entity_id: ownerDef.entity_id } } },
  ] as Operation[]
  const result = await peer.client.commit({ workspace_id: shared.workspace_id, epoch: sinfo.epoch }, ops)
  expect(result.status, JSON.stringify(result)).toBe('accepted')

  // the libraries are each user's own: clean them
  await owner.client.ok('ext.uninstall', { pkg_id: `dv-owner-${tag}` })
  await peer.client.ok('ext.uninstall', { pkg_id: `dv-peer-${tag}` })
  await peer.client.ok('ws.delete', { workspace_id: peerWs.workspace_id })
})

/** The buckyos CLI of the websdk checkout next to this repository, as the signed-in owner. */
function cli(token: string, baseURL: string, cwd: string, ...args: string[]): Json {
  const launcher = resolve(process.env.BUCKYOS_WEBSDK_DIR ?? '../../../../buckyos-websdk', 'cli/launcher.mjs')
  // the Zone comes from the environment: local commands (pikg) refuse an explicit --endpoint
  let out: string
  try {
    out = execFileSync(process.execPath, [launcher, '--non-interactive', '--yes', '--output', 'json', '--timeout', '5m', ...args], {
      cwd, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, BUCKYOS_TOOL_ENDPOINT: baseURL, BUCKYOS_APPCLIENT_SESSION_TOKEN: token, NODE_TLS_REJECT_UNAUTHORIZED: '0', HOME: cwd },
    })
  } catch (error) {
    const failed = error as { stdout?: string; stderr?: string }
    throw new Error(`buckyos ${args.slice(0, 2).join(' ')} failed: ${failed.stdout ?? ''} ${failed.stderr?.slice(-2000) ?? ''}`)
  }
  const envelope = JSON.parse(out) as { ok: boolean; data: Json; error?: Json }
  expect(envelope.ok, out).toBe(true)
  return envelope.data
}

test('[ext] the CLI alone: a local control built, exported, packed, installed and imported; the Installer takes the package too', async ({ owner, baseURL }) => {
  test.setTimeout(480_000)
  const token = (await owner.page.context().cookies()).find((c) => c.name === 'buckyos_session_token')!.value
  const tag = randomBytes(4).toString('hex')
  const work = mkdtempSync(join(tmpdir(), 'aiws-cli-dv-'))
  const name = `clikpi${tag}`
  const wsA = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  const wsB = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  owner.client.owned.add(wsA.workspace_id)
  owner.client.owned.add(wsB.workspace_id)

  // a control skeleton from the PIKG tool, built from the local folder (files inline)
  mkdirSync(join(work, name))
  cli(token, baseURL!, work, 'pikg', 'init', name, '--owner', `did:bns:${owner.userId}`, '--kind', 'aiws-controls', '--source', './aiws')
  const dir = join(work, name, 'aiws', 'controls', name)
  const built = cli(token, baseURL!, work, 'aiws', 'build', dir, '--workspace', wsA.workspace_id)
  expect(built.ident).toBe(`did:bns:${owner.userId}#${name}`)
  // a change: rebuilt over the recorded base; a broken one: refused with diagnostics, nothing committed
  writeFileSync(join(dir, 'declarative.json'), JSON.stringify({ layout: 'stack', items: [{ kind: 'text', text: 'from the CLI' }] }))
  expect(cli(token, baseURL!, work, 'aiws', 'build', dir, '--workspace', wsA.workspace_id).version).toBe(2)
  writeFileSync(join(dir, 'component.json'), '{ "manifest_version": 1, oops }')
  let refused = ''
  try { cli(token, baseURL!, work, 'aiws', 'build', dir, '--workspace', wsA.workspace_id) } catch (error) { refused = String(error) }
  expect(refused).toContain('BUILD_FAILED')

  // export → pack → install (dev) → import into another workspace
  const exported = cli(token, baseURL!, work, 'aiws', 'ext-export', String(built.def_entity_id), '--workspace', wsA.workspace_id, '--out', join(work, 'pack'), '--name', `cli-${tag}`, '--version', '0.1.0', '--install')
  expect((exported.install as Json).action).toBe('installed')
  const pkg = (exported.install as Json).pkg_id as string
  const list = cli(token, baseURL!, work, 'aiws', 'library', 'list')
  expect(JSON.stringify(list)).toContain(`did:bns:${owner.userId}#${name}`)
  const imported = cli(token, baseURL!, work, 'aiws', 'ext-import', `${pkg}#did:bns:${owner.userId}#${name}`, '--workspace', wsB.workspace_id)
  expect(((imported.controls as Json[])[0]).action).toBe('created')
  const defs = (await outline(owner, wsB)).filter((e) => e.type_id === 'buckyos.block-def')
  expect(defs.map((d) => d.provenance_pkg)).toEqual([pkg])

  // the same package through the App Installer: answered at once, in the library, no App deployed
  const pikg = String(exported.pikg)
  cli(token, baseURL!, work, 'aiws', 'library', 'remove', pkg)
  const plan = join(work, 'plan.json')
  cli(token, baseURL!, work, 'app', 'fetch', pikg, '--plan', plan, '--policy', 'local-developer')
  const installed = cli(token, baseURL!, work, 'app', 'install', pikg, '--plan', plan, '--policy', 'local-developer')
  expect(JSON.stringify(installed)).toContain('"pkg_id"')
  expect(JSON.stringify(cli(token, baseURL!, work, 'aiws', 'library', 'list'))).toContain(pkg)
  expect(JSON.stringify(cli(token, baseURL!, work, 'app', 'list'))).not.toContain(`cli-${tag}`)
  cli(token, baseURL!, work, 'aiws', 'library', 'remove', pkg)
})

// ---- P1 (notepads/aiworkspace-extension-p1-todo.md M6): composite controls on a Zone

const HELLO = resolve('../../../doc/workspace/hello-controls/controls')

/** A copy of a hello-controls control in `work`, its `check.svg` also in a sub-folder (Q23: names are relative paths). */
function helloCopy(work: string, name: string): string {
  const dir = join(work, name)
  execFileSync('cp', ['-r', join(HELLO, name), dir])
  if (name === 'hello-card') {
    mkdirSync(join(dir, 'icons'))
    execFileSync('cp', [join(dir, 'check.svg'), join(dir, 'icons', 'check.svg')])
    const component = JSON.parse(execFileSync('cat', [join(dir, 'component.json')], { encoding: 'utf8' })) as Json
    component.assets = ['check.svg', 'icons/check.svg']
    writeFileSync(join(dir, 'component.json'), JSON.stringify(component, null, 2))
  }
  return dir
}

test('[ext p1] composites: the CLI builds hello-card + hello-board, packs them with their dependency, imports the board elsewhere; the Desktop draws an instance; the service packs a .pikg', async ({ owner, baseURL }, info) => {
  test.setTimeout(480_000)
  const token = (await owner.page.context().cookies()).find((c) => c.name === 'buckyos_session_token')!.value
  const tag = randomBytes(4).toString('hex')
  const work = mkdtempSync(join(tmpdir(), 'aiws-p1-dv-'))
  const wsA = await owner.client.ok<WorkspaceSummary>('ws.create', { title: owner.client.prefix })
  owner.client.owned.add(wsA.workspace_id)
  const wsB = await workspaceWithTasks(owner)

  // the card (with a sub-folder) and the board that builds on it, from local folders
  const card = cli(token, baseURL!, work, 'aiws', 'build', helloCopy(work, 'hello-card'), '--workspace', wsA.workspace_id)
  expect(card.ident).toBe('did:bns:alice#hello-card')
  const board = cli(token, baseURL!, work, 'aiws', 'build', helloCopy(work, 'hello-board'), '--workspace', wsA.workspace_id)
  expect(board.ident).toBe('did:bns:alice#hello-board')
  const boardRead = await owner.client.ok<{ content: { payload: Json } }>('doc.read', { workspace_id: wsA.workspace_id, entity_id: board.def_entity_id })
  expect(boardRead.content.payload.deps).toEqual([{ ident: 'did:bns:alice#hello-card', entity_id: card.def_entity_id }])
  const snapshot = await owner.client.ok<{ controls: { files: { name: string }[] }[] }>('ext.export', { workspace_id: wsA.workspace_id, entity_ids: [card.def_entity_id], purpose: 'pull' })
  expect(snapshot.controls[0].files.map((f) => f.name)).toContain('icons/check.svg')

  // exported from the board alone: the package carries the card too; imported elsewhere, the card comes first
  const pkgName = `hello-p1-${tag}`
  const exported = cli(token, baseURL!, work, 'aiws', 'ext-export', String(board.def_entity_id), '--workspace', wsA.workspace_id, '--out', join(work, 'pack'), '--name', pkgName, '--version', '0.1.0', '--install')
  expect((exported.controls as Json[]).map((c) => c.ident).sort()).toEqual(['did:bns:alice#hello-board', 'did:bns:alice#hello-card'])
  const pkg = (exported.install as Json).pkg_id as string
  const imported = cli(token, baseURL!, work, 'aiws', 'ext-import', `${pkg}#did:bns:alice#hello-board`, '--workspace', wsB.workspace_id)
  expect((imported.controls as Json[]).map((c) => `${c.ident}:${c.action}`)).toEqual(['did:bns:alice#hello-card:created', 'did:bns:alice#hello-board:created'])
  const defsB = (await outline(owner, wsB)).filter((e) => e.type_id === 'buckyos.block-def')
  const boardB = defsB.find((d) => d.ident === 'did:bns:alice#hello-board')!
  const cardB = defsB.find((d) => d.ident === 'did:bns:alice#hello-card')!

  // an instance with two cards, drawn by the Desktop's composite host
  const infoB = await owner.client.info(wsB)
  const placed = await owner.client.commit({ workspace_id: wsB.workspace_id, epoch: infoB.epoch }, [
    { op: 'entity.create', entity_id: 'board-1', type_id: 'buckyos.container', parent_id: 'sf-main', order_key: 'm', placement: { x: 60, y: 60, w: 'auto', h: 'auto' },
      payload: { kind: 'composite', def_ref: { entity_id: boardB.entity_id }, config: { title: 'DV board', gap: 12 } } },
    ...['c1', 'c2'].map((c, i) => ({ op: 'entity.create', entity_id: c, type_id: 'buckyos.cell', parent_id: 'board-1', order_key: ['a', 'b'][i], placement: { x: 'auto', y: 'auto', w: 240, h: 120 },
      payload: { view: { type: 'html' }, def_ref: { entity_id: cardB.entity_id }, slot: 'cards' } })),
  ] as Operation[])
  expect(placed.status, JSON.stringify(placed)).toBe('accepted')
  await openInDesktop(owner, wsB)
  await expect(owner.page.getByTestId('aiws-composite-title-board-1')).toHaveText('DV board', { timeout: 30_000 })
  const [a, b] = [await owner.page.getByTestId('aiws-canvas-block-c1').boundingBox(), await owner.page.getByTestId('aiws-canvas-block-c2').boundingBox()]
  expect(b!.y).toBeGreaterThan(a!.y + a!.height)
  await screenshot(owner.page, info, 'p1-composite-instance')

  // the service packs the board (and the card) as a .pikg into the owner's exports bucket; installed from there
  const packed = await owner.client.ok<Json>('ext.pack', { workspace_id: wsA.workspace_id, entity_ids: [board.def_entity_id], package: { name: `svc-${tag}`, version: '0.1.0' } })
  expect(packed.pkg_id).toBe(`svc-${tag}.${owner.userId}.bns.did`)
  const installed = await owner.client.ok<Json>('ext.install', { pikg: { bucket_id: packed.bucket_id, key: packed.key }, mode: 'dev' })
  expect((installed.controls as Json[]).length).toBe(2)
  // its tar is the CLI's: the same bundle in the same deterministic archive (the gzip layers differ, README)
  const cliTar = execFileSync('gzip', ['-dc', join(work, 'pack', 'dapp_dist', 'aiws.tar.gz')])
  expect(`sha256:${createHashHex(cliTar)}`).toBe(packed.tar_digest)

  await owner.client.ok('ext.uninstall', { pkg_id: pkg })
  await owner.client.ok('ext.uninstall', { pkg_id: packed.pkg_id })
})

function createHashHex(bytes: Buffer): string {
  return createHash('sha256').update(bytes).digest('hex')
}
