/* Permission management (phase two §6.3): presets as capability sets, the actual grants grouped by
 * preset, canvas permissions (one row shown, two grants written: the Surface and its content
 * folder), subject choice from the zone's user list. Managers edit; others only see what applies to
 * them. Operations run online through `ws.*`, are never queued and never undoable. */

import { useCallback, useState } from 'react'
import { describeError } from '../../api/session'
import type { Capability, Grant, GrantList, Subject, WishAiccMode } from '../../api/types'
import { useLoad, useOutlineVersion, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { entityLabel } from './dataOps'
import { PRESETS, presetOf } from './presets'

export function PermissionsPanel() {
  const { t, list } = useAiwsText()
  const store = useStore()
  useOutlineVersion()
  const [tick, setTick] = useState(0)
  const load = useCallback(() => store.session.listGrants(), [store])
  const grants = useLoad<GrantList>(load, tick)
  const loadSubjects = useCallback(() => store.session.listSubjects().catch(() => [] as Subject[]), [store])
  const subjects = useLoad<Subject[]>(loadSubjects)
  const [subject, setSubject] = useState('')
  const [preset, setPreset] = useState('reader')
  const [custom, setCustom] = useState<Capability[]>(['read'])
  const [scope, setScope] = useState<string>('')
  const [message, setMessage] = useState<string | null>(null)
  const online = store.session.status().kind === 'live'
  const surfaces = store.outline.childrenOf('surfaces').filter((e) => e.kind === 'surface')
  const complete = grants.data?.complete ?? false
  const caps = preset === 'custom' ? custom : (PRESETS.find((p) => p.id === preset)?.caps ?? ['read'])
  const act = async (failed: (error: string) => string, work: () => Promise<void>) => {
    setMessage(null)
    try { await work(); setTick((n) => n + 1) } catch (error) { setMessage(failed(describeError(error))) }
  }
  const grant = () => act((error) => t('aiworkspace.data.perm.grantFailed', { error }), async () => {
    const target = subject.trim()
    if (!target) throw new Error(t('aiworkspace.data.perm.subjectRequired'))
    if (scope) {
      // a canvas permission: the Surface subtree and its content folder (§6.3)
      const surface = store.outline.get(scope)
      await store.session.grant(target, caps, scope)
      if (surface?.content_folder_id) await store.session.grant(target, caps, surface.content_folder_id)
    } else await store.session.grant(target, caps)
    setMessage(t(scope ? 'aiworkspace.data.perm.grantedCanvas' : 'aiworkspace.data.perm.granted', { subject: target, caps: list(caps) }))
  })
  const revoke = (g: Grant) => act((error) => t('aiworkspace.data.perm.revokeFailed', { error }), async () => {
    await store.session.revoke(g.subject, g.scope_entity_id ?? undefined)
    const surface = g.scope_entity_id ? store.outline.get(g.scope_entity_id) : undefined
    if (surface?.kind === 'surface' && surface.content_folder_id) await store.session.revoke(g.subject, surface.content_folder_id)
    setMessage(t(g.scope_entity_id ? 'aiworkspace.data.perm.revokedScope' : 'aiworkspace.data.perm.revokedWorkspace', { subject: g.subject }))
  })
  // group rows: workspace-level by preset; canvas permissions as one row per subject+surface (content folder rows folded in)
  const rows = grants.data?.grants ?? []
  const folderOf = new Map(surfaces.map((s) => [s.content_folder_id ?? '', s.entity_id]))
  const shown = rows.filter((g) => !(g.scope_entity_id && folderOf.has(g.scope_entity_id) && rows.some((o) => o.subject === g.subject && o.scope_entity_id === folderOf.get(g.scope_entity_id ?? ''))))
  return (
    <div className="aiws-permissions" data-testid="aiws-permissions">
      <div className="aiws-panel-title">{t('aiworkspace.data.perm.title')} <span className="aiws-muted">{complete ? t('aiworkspace.data.perm.complete') : t('aiworkspace.data.perm.mineOnly')}</span></div>
      {!online && <div className="aiws-warning" data-testid="aiws-permissions-offline">{t('aiworkspace.data.perm.offline')}</div>}
      {grants.error && <div className="aiws-error" role="alert">{grants.error}</div>}
      <div className="aiws-muted">{t('aiworkspace.data.perm.intro')}</div>
      <table className="aiws-grants">
        <thead><tr><th>{t('aiworkspace.data.perm.subject')}</th><th>{t('aiworkspace.data.perm.scope')}</th><th>{t('aiworkspace.data.perm.preset')}</th><th>{t('aiworkspace.data.perm.actual')}</th><th /></tr></thead>
        <tbody>
          {shown.map((g) => {
            const surface = g.scope_entity_id ? store.outline.get(g.scope_entity_id) : undefined
            const scopeLabel = !g.scope_entity_id ? t('aiworkspace.data.perm.scopeWorkspace') : surface?.kind === 'surface' ? t('aiworkspace.data.perm.scopeCanvas', { name: entityLabel(surface) })
              : t('aiworkspace.data.perm.scopeSubtree', { name: surface ? entityLabel(surface) : g.scope_entity_id })
            return (
              <tr key={`${g.subject}:${g.scope_entity_id ?? ''}`} data-testid="aiws-grant" data-subject={g.subject} data-scope={g.scope_entity_id ?? ''}>
                <td>{g.subject === '*' ? t('aiworkspace.data.perm.anySignedIn') : g.subject === 'public' ? t('aiworkspace.data.perm.public') : g.subject}{g.subject === grants.data?.owner ? <span className="aiws-chip">{t('aiworkspace.data.perm.owner')}</span> : null}</td>
                <td>{scopeLabel}</td>
                <td data-testid="aiws-grant-preset">{presetOf(g.capabilities)}</td>
                <td data-testid="aiws-grant-caps">{list(g.capabilities)}</td>
                <td>{complete && !(g.subject === grants.data?.owner && !g.scope_entity_id) && <button type="button" className="aiws-link" data-testid="aiws-revoke" onClick={() => { void revoke(g) }}>{t('aiworkspace.data.perm.revoke')}</button>}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
      {complete && (
        <form className="aiws-inline-form aiws-grant-form" data-testid="aiws-grant-form" onSubmit={(event) => { event.preventDefault(); void grant() }}>
          <input aria-label={t('aiworkspace.data.perm.subject')} list="aiws-subjects" placeholder={t('aiworkspace.data.perm.subjectPlaceholder')} value={subject} onChange={(event) => setSubject(event.target.value)} data-testid="aiws-grant-subject" />
          <datalist id="aiws-subjects">
            <option value="public">{t('aiworkspace.data.perm.publicOption')}</option>
            <option value="*">{t('aiworkspace.data.perm.anySignedInOption')}</option>
            {(subjects.data ?? []).map((s) => <option key={s.subject} value={s.subject}>{s.kind}</option>)}
          </datalist>
          <select aria-label={t('aiworkspace.data.perm.presetGroup')} value={preset} data-testid="aiws-grant-preset-select" onChange={(event) => setPreset(event.target.value)}>
            {PRESETS.map((p) => <option key={p.id} value={p.id}>{t('aiworkspace.data.perm.presetOption', { label: p.label, caps: list(p.caps) })}</option>)}
            <option value="custom">{t('aiworkspace.data.perm.customOption')}</option>
          </select>
          {preset === 'custom' && (['read', 'append', 'update', 'delete', 'structure', 'comment', 'export', 'manage'] as Capability[]).map((c) => (
            <label key={c}><input type="checkbox" checked={custom.includes(c)} onChange={(event) => setCustom(event.target.checked ? [...custom, c] : custom.filter((x) => x !== c))} /> {c}</label>
          ))}
          <select aria-label={t('aiworkspace.data.perm.scope')} value={scope} data-testid="aiws-grant-scope" onChange={(event) => setScope(event.target.value)}>
            <option value="">{t('aiworkspace.data.perm.wholeWorkspace')}</option>
            {surfaces.map((s) => <option key={s.entity_id} value={s.entity_id}>{t('aiworkspace.data.perm.scopeCanvas', { name: entityLabel(s) })}</option>)}
          </select>
          <button type="submit" data-testid="aiws-grant-submit" disabled={!online}>{t('aiworkspace.data.perm.grant')}</button>
        </form>
      )}
      {message && <div className="aiws-warning" role="status" data-testid="aiws-permissions-message">{message}</div>}
      {(subjects.data ?? []).length === 0 && complete && <div className="aiws-muted">{t('aiworkspace.data.perm.noSubjects')}</div>}
      <WishAiccSetting online={online} />
    </div>
  )
}

/** Whose AICC the wish blocks use (AI Center Quota PRD §3.3): a manager chooses, everyone sees it. */
function WishAiccSetting({ online }: { online: boolean }) {
  const { t } = useAiwsText()
  const store = useStore()
  const [mode, setMode] = useState<WishAiccMode>(store.session.info().wish_aicc_mode ?? 'workspace_owner')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const canManage = store.session.info().capabilities.includes('manage')
  const change = (next: WishAiccMode) => {
    setBusy(true)
    setFailure(null)
    store.session.setWishAiccMode(next)
      .then(setMode)
      .catch((error: unknown) => setFailure(t('aiworkspace.data.perm.wishAiccFailed', { error: describeError(error) })))
      .finally(() => setBusy(false))
  }
  return (
    <div className="aiws-inline-form" data-testid="aiws-wish-aicc-setting">
      <label>{t('aiworkspace.data.perm.wishAicc')}
        <select value={mode} disabled={!canManage || !online || busy} data-testid="aiws-wish-aicc-mode" onChange={(event) => change(event.target.value as WishAiccMode)}>
          <option value="workspace_owner">{t('aiworkspace.data.perm.wishAiccOwner')}</option>
          <option value="current_user">{t('aiworkspace.data.perm.wishAiccStarter')}</option>
        </select>
      </label>
      <div className="aiws-muted">{t('aiworkspace.data.perm.wishAiccHint')}</div>
      {failure && <div className="aiws-error" role="alert">{failure}</div>}
    </div>
  )
}
