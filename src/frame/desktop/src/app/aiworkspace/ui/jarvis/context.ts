/* What the Agent's panel hands the Agent with a message (C2-1; PRD §8.3): the workspace, where the person is (the
 * canvas and a running guide's step, or the object open in the data mode) and what is selected — each as its object
 * (`obj://aiworkspace/<ws>/entities/<id>`, server/src/obj_provider.rs) and path, for the Agent to read itself: the
 * conversation is never the source of truth. It travels as plain text in the message (the Agent's model reads a
 * message's text, not its machine part); the keys are fixed and the manual aiworkspace_run reads them. */

import { pathOf } from '../../api/paths'
import type { EntityEnvelope } from '../../api/types'
import { aiwsT } from '../../i18n'
import { workspacePageUrl } from '../../links'
import type { WorkspaceStore } from '../../state/store'
import { dataTabsOf, PERMISSIONS_TAB } from '../datamode/tabs'
import type { ShellApi } from '../shell/shellContext'

export const CONTEXT_HEADER = '[aiworkspace context]'
const MAX_SELECTED = 12

export interface PanelContext {
  /** The same place and selection give the same key: what was sent once is not sent again. */
  key: string
  /** The chip above the conversation. */
  summary: string
  /** The block the message carries. */
  text: string
}

export function objectUrl(workspaceId: string, entityId?: string): string {
  return entityId ? `obj://aiworkspace/${workspaceId}/entities/${entityId}` : `obj://aiworkspace/${workspaceId}`
}

const labelOf = (e: EntityEnvelope) => e.title || e.name || e.entity_id

/** `<role>: <name> (<what>) <object> <path>`; a Block also names the data it shows (a wish's Block: the wish). */
function refLine(store: WorkspaceStore, role: string, e: EntityEnvelope): string {
  const ws = store.session.workspaceId
  const what = e.type_id === 'buckyos.cell' ? `${e.view_type ?? 'block'} block` : e.kind ?? e.type_id
  const source = e.source_id ? store.outline.get(e.source_id) : undefined
  const shows = e.source_id ? `; shows ${source ? `${labelOf(source)} (${source.type_id}) ` : ''}${objectUrl(ws, e.source_id)}` : ''
  return `${role}: ${labelOf(e)} (${what}) ${objectUrl(ws, e.entity_id)} ${pathOf(store.outline, e.entity_id)}${shows}`
}

/** The context as it is now. */
export function panelContext(store: WorkspaceStore, shell: Pick<ShellApi, 'topMode' | 'activeSurface' | 'guide'>): PanelContext {
  const info = store.session.info()
  const ws = store.session.workspaceId
  const lines = [CONTEXT_HEADER, `workspace: ${info.title} ${objectUrl(ws)}`]
  const chip: string[] = []
  const key: string[] = [ws, shell.topMode]
  let page: string | null
  if (shell.topMode === 'sources') {
    const active = dataTabsOf(store).snapshot().active
    const open = active && active !== PERMISSIONS_TAB ? store.outline.get(active) : undefined
    lines.push('view: data mode')
    if (open) { lines.push(refLine(store, 'open', open)); chip.push(labelOf(open)); key.push(open.entity_id) }
    page = workspacePageUrl(info.public_base, info.title)
  } else {
    const surface = shell.activeSurface
    if (surface) { lines.push(refLine(store, 'canvas', surface)); chip.push(labelOf(surface)); key.push(surface.entity_id) }
    const guide = shell.guide ? store.outline.get(shell.guide.pathId) : undefined
    if (shell.guide && guide) {
      lines.push(`guide: ${labelOf(guide)} step ${shell.guide.index + 1} ${objectUrl(ws, guide.entity_id)}`)
      chip.push(`${labelOf(guide)} ${aiwsT('aiworkspace.jarvis.context.step', { n: shell.guide.index + 1 })}`)
      key.push(`${guide.entity_id}#${shell.guide.index}`)
    }
    const focus = store.canvasFocus
    const selected = surface && focus.surfaceId === surface.entity_id
      ? focus.current.map((id) => store.outline.get(id)).filter((e): e is EntityEnvelope => Boolean(e && !e.deleted))
      : []
    for (const e of selected.slice(0, MAX_SELECTED)) lines.push(refLine(store, 'selected', e))
    if (selected.length > MAX_SELECTED) lines.push(`selected: ${selected.length - MAX_SELECTED} more`)
    if (selected.length > 0) chip.push(aiwsT('aiworkspace.jarvis.context.selected', { count: selected.length }))
    key.push(...selected.map((e) => e.entity_id))
    page = workspacePageUrl(info.public_base, info.title, surface ? { id: surface.entity_id, title: labelOf(surface) } : null, selected.length === 1 ? selected[0].entity_id : null)
  }
  if (page) lines.splice(2, 0, `page: ${page}`)
  return { key: key.join('\n'), summary: chip.length > 0 ? chip.join(' · ') : info.title, text: lines.join('\n') }
}
