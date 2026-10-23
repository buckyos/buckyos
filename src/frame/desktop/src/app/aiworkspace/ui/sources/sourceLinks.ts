/* Inserting a link as data (内核架构改进 D26, T4-5): a publication link of another Workspace — on its own host
 * (`aiws-<id>.<zonehost>/<workspace>/<name>`) or the service path (`/kapi/aiworkspace/pub/<id>/<name>`) — becomes
 * a `source{ kind: workspace }`; any other http(s) URL (CSV, JSON, Markdown) becomes a `source{ kind: url }`.
 * The entity is created empty and refreshed at once (a run of `source.refresh@1`). */

import { randomId } from '../../api/ids'
import type { Json, Operation, RunView } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { SYSTEM_IDS } from '../../api/types'

export interface ParsedLink {
  kind: 'workspace' | 'url'
  url: string
  workspace_id?: string
  name?: string
  zone?: string
  /** Where the publication's JSON is read from before inserting (type and title). */
  describe?: string
}

const ACTIVE = new Set(['queued', 'running', 'planning', 'validating', 'applying'])

/** `selfHost`: this Zone's host (the Desktop's), so a link of this Zone carries no `zone`. */
export function parseLink(text: string, selfHost: string): ParsedLink {
  const raw = text.trim()
  const url = (() => { try { return new URL(raw, window.location.origin) } catch { return null } })()
  if (!url || !/^https?:$/.test(url.protocol)) throw new Error(aiwsT('aiworkspace.data.tree.linkInvalid'))
  const byPath = /\/kapi\/aiworkspace\/pub\/(ws_[a-z0-9]+)\/([^/?#]+)/.exec(url.pathname)
  if (byPath) {
    const foreign = url.host !== selfHost && url.host !== window.location.host
    return { kind: 'workspace', url: raw, workspace_id: byPath[1], name: decodeURIComponent(byPath[2]), zone: foreign ? url.host : undefined, describe: foreign ? `${url.origin}${url.pathname}?format=json` : `/kapi/aiworkspace/pub/${byPath[1]}/${byPath[2]}?format=json` }
  }
  const byHost = /^aiws-([a-z0-9]+)\.(.+)$/.exec(url.host)
  const segments = url.pathname.split('/').filter(Boolean)
  if (byHost && segments.length === 2) {
    const workspaceId = `ws_${byHost[1]}`
    const zone = byHost[2] === selfHost ? undefined : byHost[2]
    const name = decodeURIComponent(segments[1])
    return { kind: 'workspace', url: raw, workspace_id: workspaceId, name, zone, describe: zone ? `${url.origin}/-/${segments[1]}?format=json` : `/kapi/aiworkspace/pub/${workspaceId}/${segments[1]}?format=json` }
  }
  return { kind: 'url', url: raw }
}

/** Create the sourced entity under `parentId` and refresh it; resolves with the entity id and the run's end state. */
export async function insertLinkedSource(store: WorkspaceStore, parentId: string, text: string, title: string): Promise<{ entityId: string; run: RunView }> {
  const info = store.session.info()
  const selfHost = info.public_base ? new URL(info.public_base.replace('{workspace_id}', 'x')).host.replace(/^aiws-x\./, '') : window.location.host
  const link = parseLink(text, selfHost)
  let typeId = 'buckyos.table-source'
  let name = title.trim()
  let source: Record<string, Json>
  if (link.kind === 'workspace') {
    const response = await fetch(link.describe!, { headers: { accept: 'application/json' } })
    if (!response.ok) throw new Error(aiwsT('aiworkspace.data.tree.linkUnavailable', { status: String(response.status) }))
    const published = await response.json() as { type_id?: string; title?: string | null; name?: string; entity_id?: string }
    typeId = published.type_id ?? typeId
    if (typeId !== 'buckyos.table-source' && typeId !== 'buckyos.richtext') throw new Error(aiwsT('aiworkspace.data.tree.linkUnsupported', { type: typeId.replace('buckyos.', '') }))
    name ||= String(published.title ?? published.name ?? link.name ?? '')
    source = { kind: 'workspace', workspace_id: link.workspace_id ?? '', entity_id: published.entity_id ?? link.name ?? '', mode: 'live', ...(link.zone ? { zone: link.zone, url: link.describe!.split('?')[0] } : {}), name: name || link.name || '' }
  } else {
    const path = link.url.split('?')[0].toLowerCase()
    typeId = /\.(md|markdown|txt)$/.test(path) ? 'buckyos.richtext' : 'buckyos.table-source'
    name ||= decodeURIComponent(path.split('/').pop() ?? '') || link.url
    source = { kind: 'url', url: link.url, mode: 'live' }
  }
  const key = store.core.order_key_between(store.outline.childrenOf(parentId).filter((e) => !SYSTEM_IDS.has(e.entity_id)).at(-1)?.order_key ?? undefined, undefined)
  const entityId = randomId(typeId === 'buckyos.richtext' ? 't' : 's')
  const payload: Record<string, Json> = typeId === 'buckyos.richtext'
    ? { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: randomId('b') }, content: [] }] }, source }
    : { source }
  const op: Operation = { op: 'entity.create', entity_id: entityId, type_id: typeId, parent_id: parentId, order_key: key, ...(name ? { name } : {}), payload }
  const outcome = await store.submit({ editId: `new-data:${entityId}`, label: aiwsT('aiworkspace.data.tree.newLink'), operations: [op] })
  if (outcome.status === 'conflict' || outcome.status === 'rejected') throw new Error(aiwsT('aiworkspace.data.tree.linkNotCreated', { code: outcome.code }))
  let run = await store.session.procStart('source.refresh@1', { entity_id: entityId })
  while (ACTIVE.has(run.state)) {
    await new Promise((resolve) => window.setTimeout(resolve, 300))
    run = await store.session.procGet(run.run_id)
  }
  return { entityId, run }
}
