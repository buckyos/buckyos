/* The Deno host of the Office importer (Office 导入 TODO §7): a .docx / .pptx on disk → the same plans the Desktop
 * makes (editable mode) → commits through any client with `call` and `commit` (the websdk's AiWorkspaceClient) and a
 * picture upload the command provides. Deno has no canvas: cropped pictures stay whole, BMP is uploaded as it is, and
 * nothing is rasterized (charts and EMF pictures stay placeholders / are left out) — all reported; the Desktop's "as it
 * looks" mode is not here (P2).
 *
 *   deno run -A tools/import.ts <file> [--include-hidden] [--keep-deleted]
 * prints what an import would write (pages or Blocks, pictures, the report) without writing anything; the
 * buckyos-agent command `import_office` (src/tools/buckyos-agent/commands/import_office.ts) commits it. */

import { ImportReport, type Json } from '../../import-drawio/convert.ts'
import { keyBetween, keysAfter, randomId } from '../../import-drawio/keys.ts'
import { allocateKeys, buildOperations, type ImportOp, type UploadedAsset } from '../../import-drawio/operations.ts'
import { gridOrigins, stageOf } from '../../import-pdf/layout.ts'
import { uniqueName } from '../../import-table/operations.ts'
import { buildDocOperations } from '../operations.ts'
import { officeKind, OfficeError, openPackage } from '../package.ts'
import type { DocPlan, OfficePagePlan } from '../plan.ts'
import { convertDocx } from '../docx.ts'
import { convertSlide, readDeck } from '../pptx.ts'

export class ImportFileError extends Error {}

export interface OfficeOptions { includeHidden?: boolean; keepDeleted?: boolean; title?: string }

export type OfficeFilePlan =
  | { kind: 'docx'; name: string; bytes: Uint8Array; title: string; doc: DocPlan; report: ImportReport }
  | { kind: 'pptx'; name: string; bytes: Uint8Array; title: string; pages: OfficePagePlan[]; hidden: number; report: ImportReport }

/** Read a file and convert it (editable mode, the Desktop dialog's defaults). */
export async function planOfficeFile(path: string, options: OfficeOptions = {}): Promise<OfficeFilePlan> {
  const name = path.split(/[\\/]/).pop() ?? path
  const bytes = await Deno.readFile(path)
  const kind = officeKind(name, bytes)
  if (kind !== 'docx' && kind !== 'pptx') throw new ImportFileError(kind ? `${kind} files are taken apart only as .docx / .pptx: save it as such, or import it as it looks on the Desktop (LibreOffice)` : 'not a Word or PowerPoint document')
  let pkg
  try { pkg = await openPackage(bytes) } catch (error) {
    throw new ImportFileError(error instanceof OfficeError && error.code === 'encrypted' ? 'the document is encrypted' : error instanceof Error ? error.message : String(error))
  }
  const report = new ImportReport()
  const idPrefix = `${kind}${randomId().slice(0, 6)}_`
  if (pkg.kind === 'docx') {
    const doc = await convertDocx(pkg, report, { fileName: name, idPrefix, keepDeleted: options.keepDeleted })
    return { kind: 'docx', name, bytes, title: (options.title ?? doc.title).slice(0, 120), doc, report }
  }
  const deck = await readDeck(pkg)
  const slides = deck.slides.filter((s) => options.includeHidden || !s.hidden)
  const hidden = deck.slides.length - slides.length
  if (hidden) report.add('pptx_hidden_skipped', 'Hidden slides left out', String(hidden))
  const pages: OfficePagePlan[] = []
  for (const slide of slides) pages.push(await convertSlide(deck, slide, report, { fileName: name, idPrefix, pageTitle: (n) => `Slide ${n}` }))
  return { kind: 'pptx', name, bytes, title: (options.title ?? (deck.title || name.replace(/\.[^.]+$/, ''))).slice(0, 120), pages, hidden, report }
}

/** What an Agent reads before committing (`--dry-run`). */
export function describeOffice(plan: OfficeFilePlan) {
  const report = plan.report.list().map((n) => ({ kind: n.kind, count: n.count, samples: n.samples, message: n.message }))
  if (plan.kind === 'docx') {
    return {
      kind: 'docx', title: plan.title, page: 'flow',
      blocks: plan.doc.segments.map((s) => (s.kind === 'richtext' ? { kind: 'richtext', top_level_nodes: s.content.content?.length ?? 0 } : { kind: 'image', image: s.image, ...(s.title ? { title: s.title } : {}) })),
      counts: plan.doc.counts, images: plan.doc.images.length, report,
    }
  }
  return {
    kind: 'pptx', title: plan.title, page: 'free', slides: plan.pages.length, hidden_left_out: plan.hidden,
    pages: plan.pages.map((p) => ({ slide: p.slide, name: p.name, objects: p.items.length - 2, counts: p.counts })),
    images: plan.pages.reduce((n, p) => n + p.images.length, 0), report,
  }
}

/** What committing needs: the service client and a picture upload (bytes → a staged Workspace asset). */
export interface OfficeClient {
  call(method: string, params: Record<string, unknown>): Promise<Record<string, unknown>>
  commit(workspaceId: string, request: Record<string, unknown>): Promise<Record<string, unknown>>
  upload(bytes: Uint8Array, name: string, mediaType: string): Promise<{ object_id: string }>
}

export interface OfficeCommitted { surface_id: string; content_folder_id: string; path_id: string | null; blocks?: string[]; pages?: string[]; images: number; missing_images: number }

const MARGIN = 80
const ASSET_LIMIT = 32 * 1024 * 1024

/** Commit a plan as a new page (docx: a flow page; pptx: a free canvas with a presentation path), the original file
 * kept unless `keepOriginal` is false. Stops at the first refused commit. */
export async function commitOffice(client: OfficeClient, workspaceId: string, plan: OfficeFilePlan, options: { keepOriginal?: boolean; createPath?: boolean } = {}, progress?: (text: string) => void): Promise<{ committed: OfficeCommitted | null; error?: { code: string; detail: string; step: string } }> {
  const info = await client.call('ws.get_info', { workspace_id: workspaceId })
  const epoch = String(info.epoch)
  const outline = (await client.call('doc.outline', { workspace_id: workspaceId })).entities as { entity_id: string; parent_id?: string; order_key?: string; name?: string; deleted?: boolean }[]
  const lastKey = (parent: string) => outline.filter((e) => e.parent_id === parent && !e.deleted).map((e) => e.order_key ?? '').sort().at(-1) ?? null
  // names are unique among siblings: the same file imported again gets " (2)"
  const name = uniqueName(plan.title, new Set(outline.filter((e) => !e.deleted && (e.parent_id === 'surfaces' || e.parent_id === 'canvas-content')).map((e) => e.name ?? '').filter(Boolean)))
  const surfaceId = randomId('sf')
  const folderId = `${surfaceId}-content`
  const flow = plan.kind === 'docx'
  // pictures: each content once
  const images = flow ? plan.doc.images : plan.pages.flatMap((p) => p.images)
  const assets = new Map<string, UploadedAsset | null>()
  const byName = new Map<string, Promise<UploadedAsset | null>>()
  for (const [k, image] of images.entries()) {
    if (!image.bytes || assets.has(image.key)) continue
    progress?.(`picture ${k + 1} of ${images.length}`)
    const hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', image.bytes as BufferSource)), (b) => b.toString(16).padStart(2, '0')).join('')
    let pending = byName.get(hash)
    if (!pending) {
      if (image.mediaType === 'image/bmp') plan.report.add('office_bmp_kept', 'BMP pictures stored as they are (no canvas here to convert them)', image.name)
      pending = client.upload(image.bytes, image.name, image.mediaType).then((r) => ({ object_id: r.object_id, file_name: image.name }), () => { plan.report.add('image_missing', 'Pictures that could not be stored left out', image.name); return null })
      byName.set(hash, pending)
    }
    assets.set(image.key, await pending)
  }
  let original: { object_id: string; file_name: string } | null = null
  if (options.keepOriginal !== false && plan.bytes.length <= ASSET_LIMIT) {
    try { original = { object_id: (await client.upload(plan.bytes, plan.name, plan.kind === 'docx' ? 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' : 'application/vnd.openxmlformats-officedocument.presentationml.presentation')).object_id, file_name: plan.name } } catch {
      plan.report.add('office_original_missing', 'The original file could not be stored', plan.name)
    }
  }
  const surfaceOps: ImportOp[] = [
    { op: 'entity.create', entity_id: folderId, type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: keyBetween(lastKey('canvas-content'), null), name, payload: { kind: 'folder', title: plan.title, system: 'surface_content', surface_id: surfaceId } },
    { op: 'entity.create', entity_id: surfaceId, type_id: 'buckyos.container', parent_id: 'surfaces', order_key: keyBetween(lastKey('surfaces'), null), name, payload: { kind: 'surface', layout: { mode: flow ? 'flow' : 'free' }, title: plan.title, content_folder_id: folderId } },
  ]
  // one Commit for the whole import (the client sends a large one in parts): it is there as a whole, or not at all
  const submit = async (operations: ImportOp[]) => {
    progress?.(`commit: ${operations.length} operations`)
    const result = await client.commit(workspaceId, { protocol_version: '1.0', epoch, idempotency_key: `import-office/${surfaceId}`, session_id: 'import-office', operations })
    if (result.status === 'accepted') return null
    const error = (result.errors as { code?: string; detail?: string }[] | undefined)?.[0]
    return { code: String(result.code ?? error?.code ?? result.status), detail: String(result.detail ?? error?.detail ?? JSON.stringify(result).slice(0, 400)), step: 'commit' }
  }
  if (flow) {
    const built = buildDocOperations(plan.doc, { surfaceId, contentFolderId: folderId, afterKey: null, contentAfterKey: null, assets, original })
    const error = await submit([...surfaceOps, ...built.ops])
    if (error) return { committed: null, error }
    return { committed: { surface_id: surfaceId, content_folder_id: folderId, path_id: null, blocks: built.blocks, images: [...assets.values()].filter(Boolean).length, missing_images: built.missing } }
  }
  // slides: the canvas with the original in its folder, the pages in a grid, the path
  let contentAfter: string | null = null
  if (original) {
    contentAfter = keysAfter(null, 1)[0]
    surfaceOps.push({ op: 'entity.create', entity_id: randomId('d'), type_id: 'buckyos.asset-ref', parent_id: folderId, order_key: contentAfter, payload: { object_id: original.object_id, file_name: original.file_name.slice(0, 255) } })
  }
  const sizes = plan.pages.map((p) => p.pageSize ?? { w: p.bounds.w, h: p.bounds.h })
  const origins = gridOrigins(sizes)
  const all: ImportOp[] = [...surfaceOps]
  const frames: string[] = []
  const groups: string[] = []
  const keys = allocateKeys(plan.pages, assets, null, contentAfter)
  for (const [i, page] of plan.pages.entries()) {
    const built = buildOperations(page, { parentId: surfaceId, origin: { x: origins[i].x + MARGIN, y: origins[i].y + MARGIN }, afterKey: null, contentFolderId: folderId, contentAfterKey: contentAfter, assets, keys: keys[i] })
    const groupId = built.ids.get(page.id)!
    all.push(...built.objects, ...built.lines)
    const frame = built.ids.get(`${page.id}~frame`)
    if (frame) frames.push(frame)
    groups.push(groupId)
  }
  let pathId: string | null = null
  if (options.createPath !== false && frames.length) {
    pathId = randomId('p')
    const steps = frames.map((f) => ({ id: randomId('st'), target: { kind: 'frame', entity_id: f }, transition: 'cut' }))
    all.push({ op: 'entity.create', entity_id: pathId, type_id: 'buckyos.show-path', parent_id: 'shows', order_key: keyBetween(lastKey('shows'), null), payload: { title: plan.title, purpose: 'presentation', stage: stageOf(sizes), background: '#000000', steps } as unknown as Record<string, Json> })
  }
  const error = await submit(all)
  if (error) return { committed: null, error }
  return { committed: { surface_id: surfaceId, content_folder_id: folderId, path_id: pathId, pages: groups, images: [...assets.values()].filter(Boolean).length, missing_images: 0 } }
}

/** `--name value` / `--flag` arguments. */
export function parseOfficeArgs(argv: string[]): { file: string | null; flags: Map<string, string | true> } {
  const flags = new Map<string, string | true>()
  let file: string | null = null
  const booleans = new Set(['include-hidden', 'keep-deleted', 'no-original', 'no-path', 'dry-run'])
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (!a.startsWith('--')) { file ??= a; continue }
    const name = a.slice(2)
    if (booleans.has(name)) { flags.set(name, true); continue }
    const value = argv[++i]
    if (value === undefined) throw new ImportFileError(`--${name} needs a value`)
    flags.set(name, value)
  }
  return { file, flags }
}

if (import.meta.main) {
  const args = parseOfficeArgs(Deno.args)
  if (!args.file) { console.error('usage: deno run -A tools/import.ts <file> [--include-hidden] [--keep-deleted]'); Deno.exit(2) }
  const plan = await planOfficeFile(args.file, { includeHidden: args.flags.get('include-hidden') === true, keepDeleted: args.flags.get('keep-deleted') === true })
  console.log(JSON.stringify(describeOffice(plan), null, 2))
}
