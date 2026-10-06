/* Permission management (phase two §6.3): presets as capability sets, the actual grants grouped by
 * preset, canvas permissions (one row shown, two grants written: the Surface and its content
 * folder), subject choice from the zone's user list. Managers edit; others only see what applies to
 * them. Operations run online through `ws.*`, are never queued and never undoable. */

import { useCallback, useState } from 'react'
import { describeError } from '../../api/session'
import type { Capability, Grant, GrantList, Subject } from '../../api/types'
import { useLoad, useOutlineVersion, useStore } from '../../state/hooks'
import { entityLabel } from './dataOps'
import { PRESETS, presetOf } from './presets'

export function PermissionsPanel() {
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
  const act = async (label: string, work: () => Promise<void>) => {
    setMessage(null)
    try { await work(); setTick((n) => n + 1) } catch (error) { setMessage(`${label}失败：${describeError(error)}`) }
  }
  const grant = () => act('授权', async () => {
    const target = subject.trim()
    if (!target) throw new Error('请选择或输入主体')
    if (scope) {
      // a canvas permission: the Surface subtree and its content folder (§6.3)
      const surface = store.outline.get(scope)
      await store.session.grant(target, caps, scope)
      if (surface?.content_folder_id) await store.session.grant(target, caps, surface.content_folder_id)
    } else await store.session.grant(target, caps)
    setMessage(`已授予 ${target}：${caps.join('、')}${scope ? '（画布）' : ''}。界面显示的是后台返回的实际能力。`)
  })
  const revoke = (g: Grant) => act('撤回', async () => {
    await store.session.revoke(g.subject, g.scope_entity_id ?? undefined)
    const surface = g.scope_entity_id ? store.outline.get(g.scope_entity_id) : undefined
    if (surface?.kind === 'surface' && surface.content_folder_id) await store.session.revoke(g.subject, surface.content_folder_id)
    setMessage(`已撤回 ${g.subject} 在${g.scope_entity_id ? '该范围' : '工作区'}的授权；祖先或工作区授权可能仍使其有效。`)
  })
  // group rows: workspace-level by preset; canvas permissions as one row per subject+surface (content folder rows folded in)
  const rows = grants.data?.grants ?? []
  const folderOf = new Map(surfaces.map((s) => [s.content_folder_id ?? '', s.entity_id]))
  const shown = rows.filter((g) => !(g.scope_entity_id && folderOf.has(g.scope_entity_id) && rows.some((o) => o.subject === g.subject && o.scope_entity_id === folderOf.get(g.scope_entity_id ?? ''))))
  return (
    <div className="aiws-permissions" data-testid="aiws-permissions">
      <div className="aiws-panel-title">权限管理 <span className="aiws-muted">{complete ? '全部授权' : '只显示对我生效的授权'}</span></div>
      {!online && <div className="aiws-warning" data-testid="aiws-permissions-offline">后台不可达：显示的是最近已知状态，权限管理需要联网。</div>}
      {grants.error && <div className="aiws-error" role="alert">{grants.error}</div>}
      <div className="aiws-muted">分享以整个工作区为单位：授予工作区级能力（至少 read）。画布权限在此基础上对某张画布额外授权。</div>
      <table className="aiws-grants">
        <thead><tr><th>主体</th><th>范围</th><th>预设</th><th>实际能力</th><th /></tr></thead>
        <tbody>
          {shown.map((g) => {
            const surface = g.scope_entity_id ? store.outline.get(g.scope_entity_id) : undefined
            const scopeLabel = !g.scope_entity_id ? '工作区' : surface?.kind === 'surface' ? `画布：${entityLabel(surface)}` : `子树：${surface ? entityLabel(surface) : g.scope_entity_id}`
            return (
              <tr key={`${g.subject}:${g.scope_entity_id ?? ''}`} data-testid="aiws-grant" data-subject={g.subject} data-scope={g.scope_entity_id ?? ''}>
                <td>{g.subject === '*' ? '任意已认证主体（*）' : g.subject}{g.subject === grants.data?.owner ? <span className="aiws-chip">所有者</span> : null}</td>
                <td>{scopeLabel}</td>
                <td data-testid="aiws-grant-preset">{presetOf(g.capabilities)}</td>
                <td data-testid="aiws-grant-caps">{g.capabilities.join('、')}</td>
                <td>{complete && !(g.subject === grants.data?.owner && !g.scope_entity_id) && <button type="button" className="aiws-link" data-testid="aiws-revoke" onClick={() => { void revoke(g) }}>撤回</button>}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
      {complete && (
        <form className="aiws-inline-form aiws-grant-form" data-testid="aiws-grant-form" onSubmit={(event) => { event.preventDefault(); void grant() }}>
          <input aria-label="主体" list="aiws-subjects" placeholder="用户或 Agent 标识" value={subject} onChange={(event) => setSubject(event.target.value)} data-testid="aiws-grant-subject" />
          <datalist id="aiws-subjects">{(subjects.data ?? []).map((s) => <option key={s.subject} value={s.subject}>{s.kind}</option>)}</datalist>
          <select aria-label="预设权限组" value={preset} data-testid="aiws-grant-preset-select" onChange={(event) => setPreset(event.target.value)}>
            {PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}（{p.caps.join('、')}）</option>)}
            <option value="custom">自定义…</option>
          </select>
          {preset === 'custom' && (['read', 'append', 'update', 'delete', 'structure', 'comment', 'export', 'manage'] as Capability[]).map((c) => (
            <label key={c}><input type="checkbox" checked={custom.includes(c)} onChange={(event) => setCustom(event.target.checked ? [...custom, c] : custom.filter((x) => x !== c))} /> {c}</label>
          ))}
          <select aria-label="范围" value={scope} data-testid="aiws-grant-scope" onChange={(event) => setScope(event.target.value)}>
            <option value="">整个工作区</option>
            {surfaces.map((s) => <option key={s.entity_id} value={s.entity_id}>画布：{entityLabel(s)}</option>)}
          </select>
          <button type="submit" data-testid="aiws-grant-submit" disabled={!online}>授权</button>
        </form>
      )}
      {message && <div className="aiws-warning" role="status" data-testid="aiws-permissions-message">{message}</div>}
      {(subjects.data ?? []).length === 0 && complete && <div className="aiws-muted">没有可选的主体列表（后台未提供）：直接输入经认证的主体标识。</div>}
    </div>
  )
}
