/* Publication by path and Sources (内核架构改进 D12, D26; M4). A manager publishes a data entity: the `public`
 * subject may read it and a link (the URL projection of the reference, D21) is shown; whoever reads it can see
 * who registered as a referrer. A sourced entity shows where it comes from and refreshes on demand: a run of
 * `source.refresh@1`, one Commit of origin `source`; its content stays read-only here. */

import { useCallback, useState } from 'react'
import { unwrap } from '../../api/client'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Publication, Referrer, Result, RunView } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { SyncTaskSection, type SyncRef } from './SyncTaskSection'

/** `meta.data` of a sourced entity (AI Native 余下任务 §7.2, B0-2/B0-3): defined but not synced, or the version its last
 * refresh wrote; the last attempt; whether it is past its freshness. */
interface DataState {
  source?: { kind?: string; path?: string; url?: string } | null
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
const ACTIVE = new Set(['queued', 'running', 'planning', 'validating', 'applying'])
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
  const [run, setRun] = useState<RunView | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const version = useVersion(`e:${entity.entity_id}`)
  const online = store.session.status().kind !== 'offline'
  const loadData = useCallback(async () => {
    if (!online) return null
    const meta = unwrap(await store.session.call<Result<{ data?: DataState }>>('doc.read', { workspace_id: store.session.workspaceId, entity_id: entity.entity_id, as: 'meta' }))
    return meta.data ?? null
  }, [store, entity.entity_id, online])
  const data = useLoad<DataState | null>(loadData, `${version}:${run?.state ?? ''}`).data ?? null
  const source = entity.source
  if (!source) return null
  // triggering a sync is anyone's with `run` (B4-1); changing the source is the owner's
  const canRefresh = entity.capabilities.some((c) => c === 'run' || c === 'update') && online
  const zone = (source.kind as string) === 'zone'
  const origin = zone ? `${t('aiworkspace.datasource.zone')} · ${data?.source?.path ?? ''}` : source.kind === 'url' ? source.url ?? '' : `${source.zone ? `${source.zone} · ` : ''}${source.workspace_id ?? ''} / ${source.entity_id ?? ''}`
  const v = data?.version ?? null
  const attempt = data?.attempt ?? null
  const def = data?.def ?? null
  const changed = attempt?.schema_changes?.map((c) => c.field_id) ?? []
  const refresh = async () => {
    setBusy(true)
    setMessage(null)
    try {
      let current = await store.session.procStart('source.refresh@1', { entity_id: entity.entity_id })
      setRun(current)
      while (ACTIVE.has(current.state)) {
        await new Promise((resolve) => window.setTimeout(resolve, 300))
        current = await store.session.procGet(current.run_id)
        setRun(current)
      }
      if (current.state === 'failed') setMessage(t('aiworkspace.data.source.failed', { error: (current.result as { error?: { detail?: string; code?: string } } | null)?.error?.detail ?? (current.result as { error?: { code?: string } } | null)?.error?.code ?? current.state }))
    } catch (error) {
      setMessage(describeError(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-wish-section" data-testid="aiws-source" data-kind={source.kind} data-mode={source.mode} data-state={data?.state ?? ''} data-expired={data?.expired ? 'true' : 'false'}>
      <div className="aiws-muted" style={{ overflowWrap: 'anywhere' }}>{t(source.mode === 'fixed' ? 'aiworkspace.data.source.fixed' : 'aiworkspace.data.source.live')} · <code>{origin}</code></div>
      {def?.meaning && <div data-testid="aiws-source-meaning">{t('aiworkspace.datasource.meaning', { text: def.meaning })}</div>}
      {def && (def.primary_key?.length || def.limit) && (
        <div className="aiws-muted">{[def.primary_key?.length ? t('aiworkspace.datasource.key', { fields: def.primary_key.join(', ') }) : '', def.limit ? t('aiworkspace.datasource.limit', { count: def.limit }) : ''].filter(Boolean).join(' · ')}</div>
      )}
      {data?.state === 'pending' && <div className="aiws-warning" role="status" data-testid="aiws-source-pending">{t('aiworkspace.datasource.pending')}</div>}
      {v && (
        <div className="aiws-muted" data-testid="aiws-source-version">
          {[v.business_time ? t('aiworkspace.datasource.businessTime', { at: dateTime(v.business_time) }) : '',
            v.synced_at ? t('aiworkspace.datasource.syncedAt', { at: dateTime(v.synced_at) }) : '',
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
        <button type="button" data-testid="aiws-source-refresh" disabled={!canRefresh || busy} onClick={() => { void refresh() }}>{t(data?.sync ? 'aiworkspace.datasource.sync.now' : 'aiworkspace.data.source.refresh')}</button>
        {run && <span className="aiws-muted" data-testid="aiws-source-run" data-state={run.state}>{STATE_LABEL[run.state] ? t(STATE_LABEL[run.state]) : run.state}</span>}
      </div>
      {message && <div className="aiws-error" role="alert" data-testid="aiws-source-error">{message}</div>}
    </div>
  )
}
