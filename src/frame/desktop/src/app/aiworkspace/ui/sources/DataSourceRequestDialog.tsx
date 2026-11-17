/* Asking for a data source (B4-6; §6 Q7, Q14): what data, from where, how fresh — sent where it can be set up. Only
 * the workspace owner sets up data sources that sync from outside, so a collaborator's request goes to the owner as a
 * MessageHub message (with the workspace, the wish and a link), and the owner handles it or hands it to their Agent;
 * the owner's own request goes to their Agent, in the conversation of the Agent's panel (B2-3). "协作 ▸ 提数据源需求…"
 * opens it, and so does a wish that found external data missing (`openDataSourceRequest` with what it wanted). */

import { useEffect, useState, useSyncExternalStore } from 'react'
import { X } from 'lucide-react'
import { loadConversationBackend } from '../../../../components/conversation/backend'
import { unwrap, type AiwsClient } from '../../api/client'
import { describeError } from '../../api/session'
import type { EntityEnvelope } from '../../api/types'
import { aiwsT } from '../../i18n'
import { workspacePageUrl } from '../../links'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { openJarvis } from '../jarvis/open'
import { objectUrl } from '../jarvis/context'
import { panelAgent, sendToPanelSession } from '../jarvis/session'
import { Modal } from '../shell/dialogs'
import { useShell } from '../shell/shellContext'
import { openDataSourceRequest, sourceRequests, type DataSourceRequest, type DataSourceWanted } from './sourceRequest'

const labelOf = (e: EntityEnvelope) => e.title || e.name || e.entity_id

function freshnessText(value: DataSourceWanted['freshness']): string {
  if (typeof value === 'string') return value
  return value?.max_age_s ? `max_age_s: ${value.max_age_s}` : ''
}

function fieldsText(value: DataSourceWanted['fields']): string {
  return (value ?? []).map((f) => (typeof f === 'string' ? f : f.name || f.field_id || '')).filter(Boolean).join(', ')
}

/** The wish the request is for, and the Block to link to (the wish's own, or the one asked about). */
function wishOf(store: WorkspaceStore, request: DataSourceRequest): { wish: EntityEnvelope | null; blockId: string | null } {
  const block = request.blockId ? store.outline.get(request.blockId) : undefined
  const wish = (request.wishId ? store.outline.get(request.wishId) : undefined) ?? (block?.source_id ? store.outline.get(block.source_id) : undefined)
  const shown = block ?? (wish ? store.outline.all().find((e) => e.source_id === wish.entity_id && e.type_id === 'buckyos.cell') : undefined)
  return { wish: wish ?? null, blockId: shown?.entity_id ?? null }
}

/** The workspace owner's principal, as the caller's workspace list names it. */
async function ownerPrincipal(client: AiwsClient, workspaceId: string): Promise<string | null> {
  const listed = unwrap(await client.wsList())
  return listed.workspaces.find((ws) => ws.workspace_id === workspaceId)?.owner_principal ?? null
}

/** A principal as MessageHub knows people: a Zone account by its `did:bns:` name (as the service names a caller without
 * a DID), anyone else by the DID that is their principal. */
const didOf = (principal: string) => (principal.startsWith('did:') ? principal : `did:bns:${principal}`)

interface Form { meaning: string; origin: string; freshness: string; fields: string }

function requestText(store: WorkspaceStore, form: Form, wish: EntityEnvelope | null, link: string | null): string {
  const ws = store.session.workspaceId
  const lines = [aiwsT('aiworkspace.sourcereq.msg.title', { workspace: store.session.info().title }), '', aiwsT('aiworkspace.sourcereq.msg.meaning', { text: form.meaning.trim() })]
  if (form.origin.trim()) lines.push(aiwsT('aiworkspace.sourcereq.msg.origin', { text: form.origin.trim() }))
  if (form.freshness.trim()) lines.push(aiwsT('aiworkspace.sourcereq.msg.freshness', { text: form.freshness.trim() }))
  if (form.fields.trim()) lines.push(aiwsT('aiworkspace.sourcereq.msg.fields', { text: form.fields.trim() }))
  lines.push('')
  if (wish) lines.push(aiwsT('aiworkspace.sourcereq.msg.wish', { name: labelOf(wish), ref: objectUrl(ws, wish.entity_id) }))
  lines.push(aiwsT('aiworkspace.sourcereq.msg.workspace', { ref: objectUrl(ws) }))
  if (link) lines.push(aiwsT('aiworkspace.sourcereq.msg.link', { url: link }))
  return lines.join('\n')
}

function RequestDialog({ request, onClose }: { request: DataSourceRequest; onClose: () => void }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const info = store.session.info()
  const toAgent = info.owner === true
  const { wish, blockId } = wishOf(store, request)
  const wanted = request.wanted ?? {}
  const [form, setForm] = useState<Form>({ meaning: wanted.meaning ?? '', origin: wanted.origin ?? '', freshness: freshnessText(wanted.freshness), fields: fieldsText(wanted.fields) })
  const [owner, setOwner] = useState<string | null | undefined>(undefined)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (toAgent) return
    let alive = true
    ownerPrincipal(shell.client, store.session.workspaceId).then((found) => { if (alive) setOwner(found) }, () => { if (alive) setOwner(null) })
    return () => { alive = false }
  }, [store, shell.client, toAgent])

  const surfaceOf = (id: string) => store.outline.ancestors(id).map((a) => store.outline.get(a)).find((e) => e?.kind === 'surface')
  const surface = (blockId ? surfaceOf(blockId) : undefined) ?? shell.activeSurface
  const link = workspacePageUrl(info.public_base, info.title, surface ? { id: surface.entity_id, title: labelOf(surface) } : null, blockId)

  const send = async () => {
    setBusy(true)
    setError(null)
    try {
      const text = requestText(store, form, wish, link)
      if (toAgent) {
        const agent = await panelAgent()
        if (!agent) throw Error(t('aiworkspace.jarvis.none'))
        await sendToPanelSession(agent, text)
        onClose()
        store.notify('info', t('aiworkspace.sourcereq.sentAgent'))
        openJarvis()
        return
      }
      if (!owner) throw Error(t('aiworkspace.sourcereq.noOwner'))
      const backend = await loadConversationBackend()
      await backend.ensureReady()
      backend.addPerson(didOf(owner), owner)
      const session = await backend.ensureDefaultSession(didOf(owner))
      if (!session) throw Error(t('aiworkspace.sourcereq.noOwner'))
      await backend.send(session.id, { text })
      onClose()
      store.notify('info', t('aiworkspace.sourcereq.sentOwner', { owner }))
    } catch (failure) {
      setError(t('aiworkspace.sourcereq.failed', { error: describeError(failure) }))
      setBusy(false)
    }
  }
  const field = (key: keyof Form, label: string, placeholder: string, rows: number, required = false) => (
    <label className="aiws-field">{label}
      <textarea rows={rows} value={form[key]} placeholder={placeholder} required={required} maxLength={2000} data-testid={`aiws-source-request-${key}`} onChange={(event) => setForm({ ...form, [key]: event.target.value })} />
    </label>
  )
  const recipient = toAgent || Boolean(owner)
  return (
    <Modal label={t('aiworkspace.sourcereq.title')} testId="aiws-source-request" className="aiws-source-request" onClose={onClose}>
      <div className="aiws-dialog-head"><b>{t('aiworkspace.sourcereq.title')}</b><span className="aiws-grow" /><button type="button" className="aiws-tool aiws-tool-small aiws-dialog-close" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} onClick={onClose}><X size={16} aria-hidden="true" /></button></div>
      <div className="aiws-muted" data-testid="aiws-source-request-to" data-to={toAgent ? 'agent' : 'owner'}>
        {toAgent ? t('aiworkspace.sourcereq.toAgent') : owner === null ? t('aiworkspace.sourcereq.noOwner') : t('aiworkspace.sourcereq.toOwner', { owner: owner ?? '…' })}
      </div>
      {wish && <div data-testid="aiws-source-request-wish">{t('aiworkspace.sourcereq.forWish', { name: labelOf(wish) })}</div>}
      <form className="aiws-form" onSubmit={(event) => { event.preventDefault(); if (!busy && form.meaning.trim() && recipient) void send() }}>
        {field('meaning', t('aiworkspace.sourcereq.meaning'), t('aiworkspace.sourcereq.meaningPlaceholder'), 3, true)}
        {field('origin', t('aiworkspace.sourcereq.origin'), t('aiworkspace.sourcereq.originPlaceholder'), 2)}
        {field('freshness', t('aiworkspace.sourcereq.freshness'), t('aiworkspace.sourcereq.freshnessPlaceholder'), 1)}
        {field('fields', t('aiworkspace.sourcereq.fields'), t('aiworkspace.sourcereq.fieldsPlaceholder'), 1)}
        {error && <div className="aiws-error" role="alert" data-testid="aiws-source-request-error">{error}</div>}
        <div className="aiws-dialog-actions">
          <button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
          <button type="submit" className="is-primary" disabled={busy || !form.meaning.trim() || !recipient} data-testid="aiws-source-request-send">{busy ? t('aiworkspace.sourcereq.sending') : t('aiworkspace.sourcereq.send')}</button>
        </div>
      </form>
    </Modal>
  )
}

/** Mounted once by the workspace shell (FrameworkHost). */
export function DataSourceRequestHost() {
  const request = useSyncExternalStore(sourceRequests.emitter.subscribe, () => sourceRequests.current)
  useEffect(() => () => openDataSourceRequest(null), [])
  return request ? <RequestDialog request={request} onClose={() => openDataSourceRequest(null)} /> : null
}
