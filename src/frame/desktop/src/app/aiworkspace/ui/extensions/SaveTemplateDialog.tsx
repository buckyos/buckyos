/* Export → Save as template… (扩展控件 P3, Q32): the chosen Surfaces of this Workspace (with what their Blocks reference
 * and the controls they use) become a Workspace template of a control package — into the user's library, or as a .pikg
 * the service packs. Strings of the content may hold `{{params.<k>}}`; the tables chosen under "start empty" lose their
 * rows in every Workspace made from it. Nothing of this Workspace changes. */

import { useState } from 'react'
import { describeError } from '../../api/session'
import type { ExtPackResult, Json, TemplateParam, TemplateSpec } from '../../api/types'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { surfacesOf } from '../canvas/surfaceOps'
import { Modal } from '../shell/dialogs'
import { saveBlob } from '../shell/download'

const ID = /^[a-z0-9][a-z0-9-]{0,63}$/
const PKG = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/

interface ParamRow { key: string; type: TemplateParam['type']; title: string; value: string }

function slug(text: string): string {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48) || 'template'
}

export function SaveTemplateDialog({ initialSurfaceId, onClose }: { initialSurfaceId: string | null; onClose: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  const surfaces = surfacesOf(store)
  const tables = store.outline.all().filter((e) => !e.deleted && e.type_id === 'buckyos.table-source')
  const [chosen, setChosen] = useState<Set<string>>(new Set(initialSurfaceId ? [initialSurfaceId] : surfaces.slice(0, 1).map((s) => s.entity_id)))
  const workspaceTitle = store.session.info().title
  const [title, setTitle] = useState(workspaceTitle)
  const [id, setId] = useState(slug(workspaceTitle))
  const [description, setDescription] = useState('')
  const [params, setParams] = useState<ParamRow[]>([])
  const [reset, setReset] = useState<Set<string>>(new Set())
  const [pkgName, setPkgName] = useState(`${slug(workspaceTitle)}-template`)
  const [version, setVersion] = useState('0.1.0')
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pack, setPack] = useState<ExtPackResult | null>(null)
  const toggle = (set: Set<string>, value: string) => { const next = new Set(set); if (next.has(value)) next.delete(value); else next.add(value); return next }
  const spec = (): TemplateSpec => ({
    surfaces: [...chosen],
    template: {
      id, title: title.trim() || id, ...(description.trim() ? { description: description.trim() } : {}),
      ...(params.length ? { params: Object.fromEntries(params.filter((p) => p.key).map((p) => [p.key, { type: p.type, ...(p.title ? { title: p.title } : {}), ...(p.value !== '' ? { default: valueOf(p) } : {}) }])) } : {}),
      ...(reset.size ? { reset: { tables: [...reset] } } : {}),
    },
  })
  const run = async (work: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try { await work() } catch (e) { setError(describeError(e)) } finally { setBusy(false) }
  }
  const toLibrary = () => run(async () => {
    const r = await store.session.extInstallFrom([], { name: pkgName, version, title: title.trim() || id }, [spec()])
    setDone(t('aiworkspace.ext.template.saved', { name: title.trim() || id, pkg: r.pkg_id, version: r.version }))
  })
  const toPikg = () => run(async () => {
    const r = await store.session.extPack([], { name: pkgName, version, title: title.trim() || id }, [spec()])
    setPack(r)
    setDone(t('aiworkspace.ext.packed', { file: r.key, pkg: r.pkg_id, size: r.size }))
  })
  const download = () => run(async () => { if (pack) saveBlob(await store.session.downloadPack(pack), pack.key) })
  const install = () => run(async () => {
    if (!pack) return
    const r = await store.session.installPack({ bucket_id: pack.bucket_id, key: pack.key })
    setPack(null)
    setDone(t('aiworkspace.ext.exportedLibrary', { pkg: r.pkg_id, version: r.version, action: r.action }))
  })
  const valid = chosen.size > 0 && ID.test(id) && params.every((p) => !p.key || /^[a-z][a-z0-9_]{0,31}$/.test(p.key))
  return (
    <Modal label={t('aiworkspace.ext.template.title')} testId="aiws-save-template" className="aiws-ext-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>{t('aiworkspace.ext.template.title')}</b><span className="aiws-grow" /><button type="button" className="aiws-link" data-testid="aiws-save-template-close" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <div className="aiws-ext-form">
        <p className="aiws-muted">{t('aiworkspace.ext.template.explain')}</p>
        <fieldset className="aiws-ext-package">
          <legend>{t('aiworkspace.ext.template.surfaces')}</legend>
          {surfaces.map((s) => (
            <label key={s.entity_id} className="aiws-inline-form"><input type="checkbox" data-testid={`aiws-template-surface-${s.entity_id}`} checked={chosen.has(s.entity_id)} onChange={() => setChosen((prev) => toggle(prev, s.entity_id))} />{s.title ?? s.name ?? s.entity_id}</label>
          ))}
        </fieldset>
        <label className="aiws-field">{t('aiworkspace.ext.template.name')}<input value={title} data-testid="aiws-template-title" onChange={(e) => setTitle(e.target.value)} /></label>
        <label className="aiws-field">{t('aiworkspace.ext.template.id')}<input value={id} data-testid="aiws-template-id" onChange={(e) => setId(e.target.value.trim())} /></label>
        {id && !ID.test(id) && <div className="aiws-warning">{t('aiworkspace.ext.template.idRule')}</div>}
        <label className="aiws-field">{t('aiworkspace.ext.template.description')}<input value={description} data-testid="aiws-template-description" onChange={(e) => setDescription(e.target.value)} /></label>
        <fieldset className="aiws-ext-package">
          <legend>{t('aiworkspace.ext.template.params')}</legend>
          <div className="aiws-muted">{t('aiworkspace.ext.template.paramsHint', { example: '{{params.team_name}}' })}</div>
          {params.map((p, i) => (
            <div key={i} className="aiws-events-handler" data-testid={`aiws-template-param-${i}`}>
              <input aria-label={t('aiworkspace.ext.template.paramKey')} placeholder="team_name" value={p.key} data-testid={`aiws-template-param-key-${i}`} onChange={(e) => setParams((prev) => prev.map((x, j) => (j === i ? { ...x, key: e.target.value.trim() } : x)))} />
              <select aria-label={t('aiworkspace.ext.template.paramType')} value={p.type} onChange={(e) => setParams((prev) => prev.map((x, j) => (j === i ? { ...x, type: e.target.value as ParamRow['type'] } : x)))}>
                <option value="string">string</option><option value="number">number</option><option value="boolean">boolean</option>
              </select>
              <input aria-label={t('aiworkspace.ext.template.paramTitle')} placeholder={t('aiworkspace.ext.template.paramTitle')} value={p.title} onChange={(e) => setParams((prev) => prev.map((x, j) => (j === i ? { ...x, title: e.target.value } : x)))} />
              <input aria-label={t('aiworkspace.ext.template.paramDefault')} placeholder={t('aiworkspace.ext.template.paramDefault')} value={p.value} data-testid={`aiws-template-param-default-${i}`} onChange={(e) => setParams((prev) => prev.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))} />
              <button type="button" className="aiws-link" onClick={() => setParams((prev) => prev.filter((_, j) => j !== i))}>{t('aiworkspace.ext.events.remove')}</button>
            </div>
          ))}
          <button type="button" data-testid="aiws-template-param-add" disabled={params.length >= 16} onClick={() => setParams((prev) => [...prev, { key: '', type: 'string', title: '', value: '' }])}>{t('aiworkspace.ext.template.addParam')}</button>
        </fieldset>
        {tables.length > 0 && (
          <fieldset className="aiws-ext-package">
            <legend>{t('aiworkspace.ext.template.reset')}</legend>
            {tables.map((tb) => (
              <label key={tb.entity_id} className="aiws-inline-form"><input type="checkbox" data-testid={`aiws-template-reset-${tb.entity_id}`} checked={reset.has(tb.entity_id)} onChange={() => setReset((prev) => toggle(prev, tb.entity_id))} />{tb.title ?? tb.name ?? tb.entity_id}</label>
            ))}
          </fieldset>
        )}
        <label className="aiws-field">{t('aiworkspace.ext.packageName')}<input value={pkgName} data-testid="aiws-template-package" onChange={(e) => setPkgName(e.target.value.trim())} /></label>
        <label className="aiws-field">{t('aiworkspace.ext.packageVersion')}<input value={version} data-testid="aiws-template-version" onChange={(e) => setVersion(e.target.value.trim())} /></label>
        {done && <div role="status" data-testid="aiws-template-done">{done}</div>}
        {error && <div className="aiws-error" role="alert" data-testid="aiws-template-error">{error}</div>}
        <div className="aiws-dialog-actions">
          <button type="button" className="is-primary" data-testid="aiws-template-to-library" disabled={busy || !valid || !/^[a-z0-9][a-z0-9.-]{0,127}$/.test(pkgName)} onClick={() => void toLibrary()}>{t('aiworkspace.ext.template.toLibrary')}</button>
          <button type="button" data-testid="aiws-template-pikg" disabled={busy || !valid || !PKG.test(pkgName)} title={PKG.test(pkgName) ? undefined : t('aiworkspace.ext.pikgNameRule')} onClick={() => void toPikg()}>{t('aiworkspace.ext.exportPikg')}</button>
          {pack && <button type="button" data-testid="aiws-template-pikg-download" disabled={busy} onClick={() => void download()}>{t('aiworkspace.ext.pikgDownload')}</button>}
          {pack && <button type="button" data-testid="aiws-template-pikg-install" disabled={busy} onClick={() => void install()}>{t('aiworkspace.ext.pikgInstall')}</button>}
        </div>
      </div>
    </Modal>
  )
}

function valueOf(p: ParamRow): Json {
  if (p.type === 'number') { const n = Number(p.value); return Number.isFinite(n) ? n : p.value }
  if (p.type === 'boolean') return p.value === 'true' || p.value === '1'
  return p.value
}
