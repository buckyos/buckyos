/* "画布 ▸ 导出可打印 PDF…" (打印 TODO §3.1, T4): what the entry checks before the dialog opens. A flow canvas goes
 * straight to the dialog; a free canvas needs a presentation path with a step on it (guides do not count, Q6) — none,
 * and the person is told so, with path editing when they may edit paths here. The shell's host (PrintHost) opens the
 * dialog. "工作区 ▸ 打印字体…" (T6, Q17) asks the same host for the fonts dialog. */

import type { EntityEnvelope, KeyedContent, ShowPathPayload } from '../../api/types'
import { aiwsT } from '../../i18n'
import { pathsOf, playable, resolveSteps } from '../../presentation/model'
import type { WorkspaceStore } from '../../state/store'

export interface PrintPath { id: string; title: string; payload: ShowPathPayload }

/** What the dialog prints: the canvas, its layout, and for a free canvas the paths that go through it. */
export interface PrintTarget { surfaceId: string; mode: 'flow' | 'free'; paths: PrintPath[] }

type Request = { kind: 'print'; target: PrintTarget } | { kind: 'fonts' }

const listeners = new Set<(request: Request) => void>()

export function onPrintRequest(listener: (request: Request) => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

function emit(request: Request) {
  for (const listener of [...listeners]) listener(request)
}

export function requestPrintFonts() {
  emit({ kind: 'fonts' })
}

/** The presentation paths with a step that plays on `surfaceId`, in tree order. */
export async function pathsThrough(store: WorkspaceStore, surfaceId: string): Promise<PrintPath[]> {
  const paths = pathsOf(store.outline, 'presentation')
  const payloads = await Promise.all(paths.map((p) => store.session.read<KeyedContent<ShowPathPayload>>(p.entity_id).then((r) => r.content.payload, () => null)))
  return paths.flatMap((p, i) => {
    const payload = payloads[i]
    if (!payload || !playable(resolveSteps(store.outline, payload)).some((s) => s.surfaceId === surfaceId)) return []
    return [{ id: p.entity_id, title: payload.title || p.title || p.name || p.entity_id, payload }]
  })
}

/** The entry's run: the dialog, or for a free canvas without a path the reason (and path editing, `editPaths`). */
export async function beginPrint(store: WorkspaceStore, surface: EntityEnvelope, editPaths: (() => void) | null): Promise<void> {
  if (surface.layout?.mode !== 'free') {
    emit({ kind: 'print', target: { surfaceId: surface.entity_id, mode: 'flow', paths: [] } })
    return
  }
  const paths = await pathsThrough(store, surface.entity_id)
  if (paths.length > 0) {
    emit({ kind: 'print', target: { surfaceId: surface.entity_id, mode: 'free', paths } })
    return
  }
  store.notify('info', aiwsT('aiworkspace.print.noPath'), editPaths ? { label: aiwsT('aiworkspace.print.editPaths'), run: editPaths } : undefined)
}
