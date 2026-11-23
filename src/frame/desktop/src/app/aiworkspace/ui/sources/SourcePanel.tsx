/* Publication by path and Sources (内核架构改进 D12, D26; M4). A manager publishes a data entity: the `public`
 * subject may read it and a link (the URL projection of the reference, D21) is shown; whoever reads it can see
 * who registered as a referrer. A sourced entity shows where it comes from and refreshes on demand: a run of
 * `source.refresh@1`, one Commit of origin `source`; its content stays read-only here. */

import { useCallback, useState } from 'react'
import { randomId } from '../../api/ids'
import { unwrap } from '../../api/client'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Publication, Referrer, Result, RunView, TableSourceContent } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { precheckDelete, type DeleteCheck } from '../datamode/ops'
import { ContactsDialog, DeleteData } from '../datamode/dialogs'
import { bindContacts, CONTACT_MODE_LABEL, watchSourceRun, type ContactSyncMode } from './contacts'
import { SyncTaskSection, type SyncRef } from './SyncTaskSection'

/** `meta.data` of a sourced entity (AI Native 余下任务 §7.2, B0-2/B0-3): defined but not synced, or the version its last
 * refresh wrote; the last attempt; whether it is past its freshness. */
interface DataState {
  mode?: 'synced' | 'local' | 'url_query'
  source?: { kind?: string; provider?: string; path?: string; url?: string } | null
  binding?: 'bound' | 'needs_rebind'
  owner_did?: string
  localize_after_refresh?: boolean
  last_success_at?: string
  state?: 'pending' | 'valid'
  version?: { synced_at?: string; business_time?: string; program?: { name?: string; version?: string } | null; records?: number; truncated?: boolean } | null
  attempt?: { at: string; status: 'running' | 'succeeded' | 'failed'; error?: { code?: string; sub_code?: string; detail?: string } | null; schema_changes?: { field_id: string }[] } | null
  expired?: boolean
  def?: { meaning?: string; primary_key?: string[]; freshness?: { max_age_s: number } | null; limit?: number } | null
  /** The Agent's sync task that writes it (§8.2). */
  sync?: SyncRef | null
}

const ATTEMPT_TEXT: Record<'running' | 'succeeded' | 'failed', string> = {
  running: 'aiworkspace.datasource.attempt.running', succeeded: 'aiworkspace.datasource.attempt.succeeded', failed: 'aiworkspace.datasource.attempt.failed',
}

function ageText(seconds: number): string {
  if (seconds % 86400 === 0) return `${seconds / 86400} d`
  if (seconds % 3600 === 0) return `${seconds / 3600} h`
  return `${Math.round(seconds / 60)} min`
}

const PUBLISHABLE = new Set(['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.dir'])
const STATE_LABEL: Record<string, string> = {
  queued: 'aiworkspace.data.source.state.queued', running: 'aiworkspace.data.source.state.running', succeeded: 'aiworkspace.data.source.state.succeeded', failed: 'aiworkspace.data.source.state.failed',
}

/** Publish / unpublish, the link, and who reads it. */
export function PublicationSection({ entity }: { entity: EntityEnvelope }) {
  const { t, dateTime } = useAiwsText()
  const store = useStore()
  const version = useVersion('any')
  const canManage = store.session.info().capabilities.includes('manage')
  const load = useCallback(async () => {
    const publications = await store.session.listPublications()
    const mine = publications.find((p) => p.entity_id === entity.entity_id && p.current !== false) ?? null
    const referrers = mine ? await store.session.listReferrers(entity.entity_id) : []
    return { mine, referrers }
  }, [store, entity.entity_id])
  const read = useLoad<{ mine: Publication | null; referrers: Referrer[] }>(load, version)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  if (!PUBLISHABLE.has(entity.type_id) || entity.source) return null
  const act = (work: () => Promise<unknown>) => {
    setBusy(true)
    setMessage(null)
    work().then(() => read.reload()).catch((error: unknown) => setMessage(describeError(error))).finally(() => setBusy(false))
  }
  const mine = read.data?.mine ?? null
  const link = mine?.url ?? (mine ? `${window.location.origin}/kapi/aiworkspace/pub/${store.session.info().workspace_id}/${mine.name}` : null)
  return (
    <div className="aiws-wish-section" data-testid="aiws-publication" data-published={mine ? 'true' : 'false'}>
      <div className="aiws-muted">{t('aiworkspace.data.publish.title')}</div>
      {mine ? (
        <>
          <div className="aiws-inline-form">
            <code data-testid="aiws-publish-link">{link}</code>
            <button type="button" onClick={() => { void navigator.clipboard?.writeText(link ?? '') }}>{t('aiworkspace.data.publish.copy')}</button>
            {canManage && <button type="button" data-testid="aiws-unpublish" disabled={busy} onClick={() => act(() => store.session.unpublish(entity.entity_id))}>{t('aiworkspace.data.publish.unpublish')}</button>}
          </div>
          <div className="aiws-muted">{t('aiworkspace.data.publish.since', { at: dateTime(mine.published_at), by: mine.published_by })}</div>
          <div className="aiws-muted">{t('aiworkspace.data.publish.referrers', { count: read.data?.referrers.length ?? 0 })}</div>
          <ul className="aiws-relation-list" data-testid="aiws-referrers">
            {(read.data?.referrers ?? []).map((r, i) => (
              <li key={i} data-testid="aiws-referrer">{r.referrer.zone ? `${r.referrer.zone} · ` : ''}{r.referrer.workspace_id}{r.referrer.principal ? ` · ${r.referrer.principal}` : ''} <span className="aiws-muted">{dateTime(r.registered_at)}</span></li>
            ))}
          </ul>
        </>
      ) : (
        <div className="aiws-inline-form">
          <span className="aiws-muted">{t('aiworkspace.data.publish.hint')}</span>
          {canManage && <button type="button" data-testid="aiws-publish" disabled={busy} onClick={() => act(() => store.session.publish(entity.entity_id))}>{t('aiworkspace.data.publish.publish')}</button>}
        </div>
      )}
      {message && <div className="aiws-error" role="alert">{message}</div>}
    </div>
  )
}

/** Where a sourced entity comes from, what state its data is in, and its refresh. */
export function SourceSection({ entity }: { entity: EntityEnvelope }) {
  const { t, dateTime } = useAiwsText()
  const store = useStore()
  const [rebind, setRebind] = useState(false)
  const [cancelCheck, setCancelCheck] = useState<DeleteCheck | null>(null)
  const [run, setRun] = useState<RunView | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const version = useVersion(`e:${entity.entity_id}`)
  const online = store.session.status().kind === 'live'
  const loadData = useCallback(async () => {
    if (!online) return null
    const meta = unwrap(await store.session.call<Result<{ data?: DataState }>>('doc.read', { workspace_id: store.session.workspaceId, entity_id: entity.entity_id, as: 'meta' }))
    return meta.data ?? null
  }, [store, entity.entity_id, online])
  const data = useLoad<DataState | null>(loadData, `${version}:${run?.state ?? ''}`).data ?? null
  const source = entity.source
  const owner = store.session.info().owner === true
  const mayBind = owner && entity.capabilities.includes('update')
  const rebindControl = mayBind && (
    <>
      <button type="button" disabled={!online || busy} data-testid="aiws-contacts-rebind" onClick={() => setRebind(true)}>{t('aiworkspace.datasource.contacts.rebind')}</button>
      {rebind && <ContactsDialog existing={entity} onClose={() => setRebind(false)} />}
    </>
  )
  if (!source) return data?.mode === 'local' ? (
    <div className="aiws-wish-section">
      <div>{t('aiworkspace.datasource.contacts.mode')} · {t(CONTACT_MODE_LABEL.once)}</div>
      {data.attempt?.status === 'failed' && <div className="aiws-error" role="alert">{t(ATTEMPT_TEXT.failed, { at: dateTime(data.attempt.at), error: data.attempt.error?.detail ?? data.attempt.error?.code ?? '' })}</div>}
      {rebindControl}
    </div>
  ) : null
  const contacts = source.kind === 'builtin' && (source.provider ?? data?.source?.provider) === 'contacts'
  const canRefresh = entity.capabilities.some((c) => c === 'run' || c === 'update' || c === 'append' || c === 'structure' || c === 'delete')
    && online && (!contacts || data?.binding === 'bound')
  const origin = contacts ? t('aiworkspace.datasource.contacts.title') : source.kind === 'zone' ? `${t('aiworkspace.datasource.zone')} · ${data?.source?.path ?? ''}` : source.kind === 'url' ? source.url ?? '' : `${source.zone ? `${source.zone} · ` : ''}${source.workspace_id ?? ''} / ${source.entity_id ?? ''}`
  const mode = data?.localize_after_refresh ? 'once' : source.refresh ? 'auto' : 'manual'
  const v = data?.version ?? null
  const syncedAt = data?.last_success_at ?? v?.synced_at
  const attempt = data?.attempt ?? null
  const def = data?.def ?? null
  const changed = attempt?.schema_changes?.map((c) => c.field_id) ?? []
  const refresh = async () => {
    setRefreshing(true)
    setMessage(null)
    try {
      const current = await store.session.procStart('source.refresh@1', { entity_id: entity.entity_id })
      setRun(current)
      await watchSourceRun(store, current.run_id, setRun)
    } catch (error) {
      setMessage(describeError(error))
    } finally {
      setRefreshing(false)
    }
  }
  const changeMode = async (next: ContactSyncMode) => {
    if (busy || !mayBind || !online || next === mode) return
    setBusy(true)
    setMessage(null)
    try {
      if (next === 'once') {
        const read = await store.session.read<TableSourceContent>(entity.entity_id)
        if (read.content_rev !== entity.content_rev) throw new Error(t('aiworkspace.datasource.contacts.changed'))
        const outcome = await store.submit({
          editId: `source:${entity.entity_id}`, label: t(CONTACT_MODE_LABEL.once), operations: [
            { op: 'entity.unset_keys', entity_id: entity.entity_id, keys: [{ key: 'source', expect: { rev: read.content.key_revs.source } }] },
          ],
        })
        if (outcome.status !== 'accepted') throw new Error('code' in outcome ? outcome.code : outcome.status)
      } else {
        const reply = await bindContacts(store, entity.entity_id, next, randomId(), { expect_content_rev: entity.content_rev })
        if (reply.run_id) await watchSourceRun(store, reply.run_id, setRun)
      }
      await store.outline.reload()
      store.versions.bump(['outline', `e:${entity.entity_id}`])
    } catch (failure) {
      setMessage(describeError(failure))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-wish-section" data-testid="aiws-source" data-kind={source.kind} data-mode={source.mode} data-state={data?.state ?? ''} data-expired={data?.expired ? 'true' : 'false'}>
      <div className="aiws-muted" style={{ overflowWrap: 'anywhere' }}>{t(source.mode === 'fixed' ? 'aiworkspace.data.source.fixed' : 'aiworkspace.data.source.live')} · <code>{origin}</code></div>
      {contacts && <>
        {data?.owner_did && <div data-testid="aiws-contacts-owner">{t('aiworkspace.datasource.contacts.account', { owner: data.owner_did })}</div>}
        {data?.binding === 'needs_rebind' && <div className="aiws-warning" role="status" data-testid="aiws-contacts-unbound">
          {t('aiworkspace.datasource.contacts.unbound')} {rebindControl}
          {mayBind && data.state === 'valid' && <button type="button" disabled={!online || busy} onClick={() => void changeMode('once')}>{t(CONTACT_MODE_LABEL.once)}</button>}
        </div>}
        <label className="aiws-dm-field">{t('aiworkspace.datasource.contacts.mode')}
          <select value={mode} data-testid="aiws-contacts-current-mode" disabled={!mayBind || !online || busy || data?.binding !== 'bound'} onChange={(event) => void changeMode(event.target.value as ContactSyncMode)}>
            <option value="auto">{t(CONTACT_MODE_LABEL.auto)}</option>
            <option value="manual">{t(CONTACT_MODE_LABEL.manual)}</option>
            <option value="once" disabled={data?.state !== 'valid'}>{t(CONTACT_MODE_LABEL.once)}</option>
          </select>
        </label>
      </>}
      {def?.meaning && <div data-testid="aiws-source-meaning">{t('aiworkspace.datasource.meaning', { text: def.meaning })}</div>}
      {def && (def.primary_key?.length || def.limit) && (
        <div className="aiws-muted">{[def.primary_key?.length ? t('aiworkspace.datasource.key', { fields: def.primary_key.join(', ') }) : '', def.limit ? t('aiworkspace.datasource.limit', { count: def.limit }) : ''].filter(Boolean).join(' · ')}</div>
      )}
      {data?.state === 'pending' && <div className="aiws-warning" role="status" data-testid="aiws-source-pending">{t('aiworkspace.datasource.pending')}</div>}
      {v && (
        <div className="aiws-muted" data-testid="aiws-source-version">
          {[v.business_time ? t('aiworkspace.datasource.businessTime', { at: dateTime(v.business_time) }) : '',
            syncedAt ? t('aiworkspace.datasource.syncedAt', { at: dateTime(syncedAt) }) : '',
            v.program?.name ? t('aiworkspace.datasource.program', { name: v.program.name, version: v.program.version ?? '' }) : '',
            typeof v.records === 'number' ? t('aiworkspace.datasource.records', { count: v.records }) + (v.truncated ? ` (${t('aiworkspace.datasource.truncated')})` : '') : ''].filter(Boolean).join(' · ')}
        </div>
      )}
      {data?.expired && def?.freshness && <div className="aiws-warning" role="status" data-testid="aiws-source-expired">{t('aiworkspace.datasource.expired', { age: ageText(def.freshness.max_age_s) })}</div>}
      {attempt && (
        <div className={attempt.status === 'failed' ? 'aiws-error' : 'aiws-muted'} data-testid="aiws-source-attempt" data-status={attempt.status}>
          {t(ATTEMPT_TEXT[attempt.status], { at: dateTime(attempt.at), error: attempt.error?.detail ?? attempt.error?.sub_code ?? attempt.error?.code ?? '' })}
        </div>
      )}
      {changed.length > 0 && <div className="aiws-warning" role="status">{t('aiworkspace.datasource.schemaChanged', { fields: changed.join(', ') })}</div>}
      {data?.sync && <SyncTaskSection sync={data.sync} owner={store.session.info().owner === true} version={`${version}:${run?.state ?? ''}`} />}
      <div className="aiws-muted">{t('aiworkspace.data.source.readOnly')}{source.refresh ? ` · ${t('aiworkspace.data.source.schedule', { minutes: Math.round(source.refresh.interval_s / 60) })}` : ''}</div>
      {!store.session.info().owner && source.kind !== 'session' && <div className="aiws-muted">{t('aiworkspace.datasource.ownerOnly')}</div>}
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-source-refresh" disabled={!canRefresh || busy || refreshing} onClick={() => { void refresh() }}>{t(contacts || data?.sync ? 'aiworkspace.datasource.sync.now' : 'aiworkspace.data.source.refresh')}</button>
        {contacts && owner && entity.capabilities.includes('delete') && data?.state === 'pending' && data.binding === 'bound' && attempt?.status === 'failed' && (
          <button type="button" data-testid="aiws-contacts-cancel" disabled={!online || busy} onClick={() => {
            void precheckDelete(store, entity).then(setCancelCheck).catch((cause: unknown) => setMessage(describeError(cause)))
          }}>{t('aiworkspace.common.cancel')}</button>
        )}
        {cancelCheck && <DeleteData request={{ kind: 'delete', entityId: entity.entity_id, check: cancelCheck }} onClose={() => setCancelCheck(null)} />}
        {run && <span className="aiws-muted" data-testid="aiws-source-run" data-state={run.state}>{STATE_LABEL[run.state] ? t(STATE_LABEL[run.state]) : run.state}</span>}
      </div>
      {message && <div className="aiws-error" role="alert" data-testid="aiws-source-error">{message}</div>}
    </div>
  )
}
