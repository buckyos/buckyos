/* Control sources in the user's home folder (扩展控件 P0, src/frame/aiworkspace/README.md “Extension controls”): the
 * Desktop drives the same build / pull / watch / bundle code as the CLI (buckyos-websdk `aiws_ext`), over the Zone
 * file service the Files app uses and this Workspace's session. Watches outlive the dialog that started them (one per
 * definition), until stopped or the page goes. */

import {
  AiwsSourceError, buildControl, controlSkeleton, createControlSource, ensureFolder, exportBundle, hasControlSource, homeControlDir, homeExportDir, pullControl, sourceStateKey, unbuiltChanges, watchControl, writeHomeFile,
  type AiwsExtRpc, type AiwsSkeletonOptions, type AiwsUploader,
} from 'buckyos/aiws_ext'
import type { NfspClient } from 'buckyos/nfsp'
import { ensureSession, nfspClient } from '../../../filebrowser/data/nfsp/client'
import { ServiceFailure } from '../../api/client'
import type { BuildDiagnostic, ExtBuildResult, Json } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'

export { homeControlDir, homeExportDir, sourceStateKey }

/** The Zone file service with this Desktop's session: the page's own origin, the Desktop's or a Workspace's own host
 * (`aiws-<id>.<zone>`) alike — the gateway gives `/nfs/v1` to the file service whatever the host (Q24). */
export async function homeFiles(): Promise<NfspClient> {
  await ensureSession()
  return nfspClient().raw
}

/** The two service calls the shared drivers make, answered by this Workspace's session. */
export function extRpc(store: WorkspaceStore): AiwsExtRpc {
  return {
    extBuild: (_workspaceId: string, params: object) => store.session.extBuild(params as Record<string, Json>),
    extExport: (_workspaceId: string, entityIds: string[], purpose?: 'package' | 'pull') => store.session.extExport(entityIds, purpose),
  } as unknown as AiwsExtRpc
}

async function assetBytes(store: WorkspaceStore, objId: string): Promise<Uint8Array> {
  return new Uint8Array(await (await store.session.fetchAsset(objId)).arrayBuffer())
}

/** What a build answered, or why it did not happen (the service's diagnostics, or the folder's). */
export type BuildOutcome = { ok: true; result: ExtBuildResult } | { ok: false; error: string; diagnostics: BuildDiagnostic[]; code?: string; subCode?: string }

export function buildFailure(error: unknown): BuildOutcome {
  if (error instanceof AiwsSourceError) return { ok: false, error: error.message, diagnostics: error.diagnostics as BuildDiagnostic[] }
  if (error instanceof ServiceFailure) {
    const diagnostics = (error.data?.diagnostics as BuildDiagnostic[] | undefined) ?? []
    return { ok: false, error: error.message, diagnostics, code: error.code, subCode: error.subCode }
  }
  return { ok: false, error: error instanceof Error ? error.message : String(error), diagnostics: [] }
}

/** Files too big to go inline go to the named store first (Q22): a staged asset of this Workspace, by reference. */
export function assetUploader(store: WorkspaceStore): AiwsUploader {
  return async (name, bytes) => (await store.session.uploadAsset(new Blob([bytes as BlobPart]), name.split('/').pop() ?? name)).object_id
}

/** Build the folder `dir` into this Workspace (`.aiws-build.json` there keeps the base). */
export async function buildFromHome(store: WorkspaceStore, dir: string, opts: { force?: boolean; dryRun?: boolean } = {}): Promise<BuildOutcome> {
  try {
    const result = await buildControl(extRpc(store), await homeFiles(), store.session.workspaceId, dir, { ...opts, upload: assetUploader(store) })
    return { ok: true, result: result as unknown as ExtBuildResult }
  } catch (error) {
    return buildFailure(error)
  }
}

/** Write a definition's snapshot into `dir` (the base of the next build). */
export async function pullToHome(store: WorkspaceStore, defEntityId: string, dir: string): Promise<string[]> {
  const pulled = await pullControl(extRpc(store), await homeFiles(), store.session.workspaceId, defEntityId, dir, (_ws, obj) => assetBytes(store, obj))
  return pulled.files
}

/** Whether `dir` holds a control source (its component.json). */
export async function hasSource(dir: string): Promise<boolean> {
  try {
    return await hasControlSource(await homeFiles(), dir)
  } catch {
    return false
  }
}

/** A new control's folder from the skeleton (one optional `data` binding, a label property; a composite: its slots). */
export async function createSource(dir: string, opts: AiwsSkeletonOptions): Promise<void> {
  await createControlSource(await homeFiles(), dir, controlSkeleton(opts))
}

/** The definitions among `defIds` whose remembered home folder holds changes that were not built (P0 遗留 L3). */
export async function unbuiltDefinitions(store: WorkspaceStore, defIds: string[]): Promise<string[]> {
  const out: string[] = []
  const nfsp = await homeFiles().catch(() => null)
  if (!nfsp) return out
  for (const id of defIds) {
    const dir = store.userState.get<string>(sourceStateKey(id))
    if (typeof dir !== 'string') continue
    try {
      const r = await unbuiltChanges(extRpc(store), nfsp, store.session.workspaceId, id, dir)
      if (r?.changed) out.push(id)
    } catch { /* a folder that cannot be read says nothing */ }
  }
  return out
}

/** The bundle of definitions written into the user's home (`AI Workspace/exports/<name>/`): manifest plus controls. */
export async function exportToHome(store: WorkspaceStore, entityIds: string[], dir: string): Promise<{ files: number; controls: string[] }> {
  const bundle = await exportBundle(extRpc(store), store.session.workspaceId, entityIds, (_ws, obj) => assetBytes(store, obj))
  const nfsp = await homeFiles()
  const folders = new Map<string, Awaited<ReturnType<typeof ensureFolder>>>()
  for (const [path, bytes] of bundle.files) {
    const cut = path.lastIndexOf('/')
    const folder = cut < 0 ? dir : `${dir}/${path.slice(0, cut)}`
    let ref = folders.get(folder)
    if (!ref) { ref = await ensureFolder(nfsp, folder); folders.set(folder, ref) }
    await writeHomeFile(nfsp, ref, path.slice(cut + 1), bytes)
  }
  return { files: bundle.files.size, controls: bundle.manifest.controls.map((c) => c.ident) }
}

// ---- watches: one per definition, kept across dialogs

interface Watch { dir: string; controller: AbortController; last: BuildOutcome | null; listeners: Set<() => void> }
const watches = new Map<string, Watch>()

export function watchOf(defEntityId: string): { dir: string; last: BuildOutcome | null } | null {
  const w = watches.get(defEntityId)
  return w ? { dir: w.dir, last: w.last } : null
}

export function subscribeWatch(defEntityId: string, listener: () => void): () => void {
  const w = watches.get(defEntityId)
  if (!w) return () => undefined
  w.listeners.add(listener)
  return () => w.listeners.delete(listener)
}

/** Rebuild `dir` whenever it changes (debounced; identical objects build nothing). */
export function startWatch(store: WorkspaceStore, defEntityId: string, dir: string, onBuilt?: (outcome: BuildOutcome) => void): void {
  stopWatch(defEntityId)
  const w: Watch = { dir, controller: new AbortController(), last: null, listeners: new Set() }
  watches.set(defEntityId, w)
  void homeFiles().then((nfsp) => watchControl(nfsp, dir, async () => {
    w.last = await buildFromHome(store, dir)
    onBuilt?.(w.last)
    for (const l of w.listeners) l()
  }, { signal: w.controller.signal })).catch(() => undefined)
}

export function stopWatch(defEntityId: string): void {
  const w = watches.get(defEntityId)
  if (!w) return
  w.controller.abort()
  watches.delete(defEntityId)
  for (const l of w.listeners) l()
}
