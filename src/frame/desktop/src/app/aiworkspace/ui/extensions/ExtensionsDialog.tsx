/* Workspace controls (扩展控件 P0, src/frame/aiworkspace/README.md “Extension controls”): the definitions this
 * Workspace holds, how each is built (its source in the user's home folder → `ext.build`, atomically), the instances
 * that use it, and the ways in and out — a new control from a skeleton, controls from the user's library, an export
 * to the library or to a bundle folder. Phones list only (no building, no adding). */

import { useCallback, useMemo, useState, useSyncExternalStore } from 'react'
import { Hammer, Library, PackagePlus, Plus, Puzzle, RefreshCw, Upload } from 'lucide-react'
import type { BlockDefRead, BuildDiagnostic, EntityEnvelope, LibraryPackage } from '../../api/types'
import { describeError, type ReadOk } from '../../api/session'
import { onWorkspaceHost } from '../../links'
import { useLoad, useOutlineVersion, useStore, useUserState, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { Modal } from '../shell/dialogs'
import { controlDefinitions } from './controls'
import {
  buildFromHome, createSource, exportToHome, hasSource, homeControlDir, homeExportDir, pullToHome, sourceStateKey, startWatch, stopWatch, subscribeWatch, watchOf, type BuildOutcome,
} from './controlSources'
import { requestInsertDefinition } from './extEvents'
import { desktopUIStore } from '../../../../models/DesktopUIDataModel'
import { filesOpenLaunch } from '../../../filebrowser/launch'

const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/

type View = { kind: 'list' } | { kind: 'new' } | { kind: 'library' } | { kind: 'export' }

export function ExtensionsDialog({ initialDefId, phone, onClose }: { initialDefId?: string; phone: boolean; onClose: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  const defs = controlDefinitions(store.outline.all())
  const [chosen, setChosen] = useState<string | null>(initialDefId ?? defs[0]?.entity_id ?? null)
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const [view, setView] = useState<View>({ kind: 'list' })
  const instances = (id: string) => store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.def_id === id && !e.deleted).length
  const toggle = (id: string) => setChecked((prev) => { const next = new Set(prev); if (next.has(id)) next.delete(id); else next.add(id); return next })
  const live = store.session.status().kind === 'live'
  const canBuild = !phone && live && store.session.info().capabilities.some((c) => c === 'structure' || c === 'update')
  // the home folder is reached through the Zone file service of the Desktop's origin, not from a workspace's own host
  const homeReachable = !onWorkspaceHost()
  const current = defs.find((d) => d.entity_id === chosen) ?? null
  return (
    <Modal label={t('aiworkspace.ext.title')} testId="aiws-ext-dialog" className="aiws-ext-dialog" onClose={onClose}>
      <div className="aiws-dialog-head">
        <Puzzle size={18} aria-hidden="true" /><b>{t('aiworkspace.ext.title')}</b><span className="aiws-grow" />
        {canBuild && view.kind === 'list' && <>
          <button type="button" data-testid="aiws-ext-new" disabled={!homeReachable} title={homeReachable ? undefined : t('aiworkspace.ext.desktopOnly')} onClick={() => setView({ kind: 'new' })}><Plus size={15} aria-hidden="true" />{t('aiworkspace.ext.new')}</button>
          <button type="button" data-testid="aiws-ext-library" onClick={() => setView({ kind: 'library' })}><Library size={15} aria-hidden="true" />{t('aiworkspace.ext.fromLibrary')}</button>
          <button type="button" data-testid="aiws-ext-export" disabled={checked.size === 0} title={checked.size === 0 ? t('aiworkspace.ext.exportPick') : undefined} onClick={() => setView({ kind: 'export' })}><Upload size={15} aria-hidden="true" />{t('aiworkspace.ext.export', { count: checked.size })}</button>
        </>}
        {view.kind !== 'list' && <button type="button" className="aiws-link" onClick={() => setView({ kind: 'list' })}>{t('aiworkspace.ext.back')}</button>}
        <button type="button" className="aiws-link" data-testid="aiws-ext-close" onClick={onClose}>{t('aiworkspace.common.close')}</button>
      </div>
      {view.kind === 'new' && <NewControl onDone={(id) => { setChosen(id); setView({ kind: 'list' }) }} />}
      {view.kind === 'library' && <FromLibrary onDone={() => setView({ kind: 'list' })} />}
      {view.kind === 'export' && <ExportControls ids={[...checked]} homeReachable={homeReachable} onDone={() => setView({ kind: 'list' })} />}
      {canBuild && !homeReachable && view.kind === 'list' && <div className="aiws-muted" data-testid="aiws-ext-desktop-only">{t('aiworkspace.ext.desktopOnly')}</div>}
      {view.kind === 'list' && (
        <div className="aiws-ext-body">
          <ul className="aiws-ext-list" data-testid="aiws-ext-list">
            {defs.length === 0 && <li className="aiws-muted">{t('aiworkspace.ext.empty')}</li>}
            {defs.map((d) => (
              <li key={d.entity_id} className={d.entity_id === chosen ? 'is-active' : undefined}>
                {canBuild && <input type="checkbox" aria-label={t('aiworkspace.ext.pick', { name: d.title ?? d.entity_id })} checked={checked.has(d.entity_id)} onChange={() => toggle(d.entity_id)} data-testid={`aiws-ext-check-${d.entity_id}`} />}
                <button type="button" className="aiws-ext-row" data-testid={`aiws-ext-row-${d.entity_id}`} onClick={() => setChosen(d.entity_id)}>
                  <span className="aiws-ext-row-title">{d.title ?? d.def_id ?? d.entity_id}</span>
                  <span className="aiws-muted">{d.def_kind === 'html' ? 'HTML' : t('aiworkspace.canvas.catalog.declarative')} · v{d.def_version ?? 1} · {d.provenance_pkg ? t('aiworkspace.ext.fromPackage', { pkg: d.provenance_pkg }) : d.source_dir_id ? t('aiworkspace.ext.localBuild') : t('aiworkspace.ext.inline')} · {t('aiworkspace.ext.instances', { count: instances(d.entity_id) })}</span>
                </button>
              </li>
            ))}
          </ul>
          {current ? <DefinitionPanel key={current.entity_id} def={current} canBuild={canBuild} homeReachable={homeReachable} onInsert={onClose} /> : <div className="aiws-muted">{t('aiworkspace.ext.chooseOne')}</div>}
        </div>
      )}
    </Modal>
  )
}

function Diagnostics({ outcome }: { outcome: BuildOutcome | null }) {
  const { t } = useAiwsText()
  if (!outcome) return null
  if (outcome.ok) {
    const r = outcome.result
    return <div role="status" data-testid="aiws-ext-build-ok">{r.unchanged ? t('aiworkspace.ext.buildUnchanged') : t('aiworkspace.ext.buildOk', { version: r.version ?? '?' })}{r.diagnostics.length > 0 && <DiagList items={r.diagnostics} />}</div>
  }
  return (
    <div className="aiws-error" role="alert" data-testid="aiws-ext-build-failed">
      <div>{outcome.subCode === 'BUILD_BASE_STALE' || outcome.subCode === 'BUILD_BASE_REQUIRED' ? t('aiworkspace.ext.buildStale') : t('aiworkspace.ext.buildFailed', { error: outcome.error })}</div>
      {outcome.diagnostics.length > 0 && <DiagList items={outcome.diagnostics} />}
    </div>
  )
}

function DiagList({ items }: { items: BuildDiagnostic[] }) {
  const { t } = useAiwsText()
  return (
    <ul className="aiws-ext-diagnostics" data-testid="aiws-ext-diagnostics">
      {items.map((d, i) => (
        <li key={i} data-severity={d.severity}>
          <code>{d.file || '—'}{d.line ? `:${d.line}${d.col ? `:${d.col}` : ''}` : ''}</code> {d.message}
          {d.instances && d.instances.length > 0 && <span className="aiws-muted"> {t('aiworkspace.ext.affected', { count: d.instances.length })}</span>}
        </li>
      ))}
    </ul>
  )
}

function DefinitionPanel({ def, canBuild, homeReachable, onInsert }: { def: EntityEnvelope; canBuild: boolean; homeReachable: boolean; onInsert: () => void }) {
  const store = useStore()
  const { t, dateTime } = useAiwsText()
  const version = useVersion(`e:${def.entity_id}`)
  const load = useCallback(() => store.session.read<BlockDefRead>(def.entity_id), [store, def.entity_id])
  const read = useLoad<ReadOk<BlockDefRead>>(load, version)
  const payload = read.data?.content.payload
  const build = (read.data?.derived as { build?: { built_at: string; input_digest: string } } | null | undefined)?.build
  const saved = useUserState<string>(sourceStateKey(def.entity_id))
  const workspaceTitle = store.session.info().title
  const principal = store.session.principal ?? 'me'
  // what the person typed, else the folder they used last (user state), else the suggested one
  const [edited, setDir] = useState<string | null>(null)
  const dir = edited ?? saved ?? homeControlDir(principal, workspaceTitle, def.def_id ?? def.entity_id)
  const [busy, setBusy] = useState<string | null>(null)
  const [outcome, setOutcome] = useState<BuildOutcome | null>(null)
  const watching = useSyncExternalStore(useCallback((l: () => void) => subscribeWatch(def.entity_id, l), [def.entity_id]), () => watchOf(def.entity_id))
  const users = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.def_id === def.entity_id && !e.deleted)
  const remember = (path: string) => { if (path !== saved) store.userState.set(sourceStateKey(def.entity_id), path) }
  const run = async (label: string, work: () => Promise<void>) => {
    setBusy(label)
    try { await work() } catch (error) { setOutcome({ ok: false, error: describeError(error), diagnostics: [] }) } finally { setBusy(null) }
  }
  const editSource = () => run('edit', async () => {
    if (!await hasSource(dir)) await pullToHome(store, def.entity_id, dir)
    remember(dir)
    desktopUIStore.openAppWindow('files', { launch: filesOpenLaunch(dir) })
  })
  const doBuild = () => run('build', async () => {
    remember(dir)
    setOutcome(await buildFromHome(store, dir))
  })
  const pull = () => run('pull', async () => { await pullToHome(store, def.entity_id, dir); remember(dir); store.notify('info', t('aiworkspace.ext.pulled', { path: dir })) })
  const remove = () => run('remove', async () => {
    const ops = [{ op: 'entity.delete', entity_id: def.entity_id, expect: { rev: def.life_rev } }, ...(def.source_dir_id ? [{ op: 'entity.delete', entity_id: def.source_dir_id, expect: { rev: store.outline.get(def.source_dir_id)?.life_rev ?? 0 } }] : [])]
    await store.submit({ editId: `ext-remove:${def.entity_id}`, label: t('aiworkspace.ext.removeLabel', { name: def.title ?? def.entity_id }), operations: ops as never })
  })
  return (
    <div className="aiws-ext-detail" data-testid="aiws-ext-detail" data-def={def.entity_id}>
      <h3>{def.title ?? def.def_id}</h3>
      <dl className="aiws-catalog-meta">
        <dt>{t('aiworkspace.ext.ident')}</dt><dd><code data-testid="aiws-ext-ident">{def.ident ?? t('aiworkspace.ext.noIdent')}</code></dd>
        <dt>{t('aiworkspace.canvas.catalog.type')}</dt><dd>{def.def_kind === 'html' ? 'HTML' : t('aiworkspace.canvas.catalog.declarative')} · v{payload?.version ?? def.def_version ?? 1}</dd>
        {payload?.provenance && <><dt>{t('aiworkspace.ext.source')}</dt><dd>{t('aiworkspace.ext.fromPackage', { pkg: `${payload.provenance.pkg_id ?? ''} ${payload.provenance.version ?? ''}` })}</dd></>}
        <dt>{t('aiworkspace.ext.lastBuild')}</dt><dd data-testid="aiws-ext-built-at">{build ? dateTime(build.built_at) : t('aiworkspace.ext.neverBuilt')}</dd>
      </dl>
      {payload?.description && <p className="aiws-muted">{payload.description}</p>}
      {payload?.bindings && Object.keys(payload.bindings).length > 0 && <div className="aiws-muted">{t('aiworkspace.ext.bindingsList', { names: Object.entries(payload.bindings).map(([n, b]) => `${n}${b.required ? '*' : ''}`).join(', ') })}</div>}
      {canBuild && homeReachable && (
        <>
          <label className="aiws-field">{t('aiworkspace.ext.sourceFolder')}
            <input value={dir} data-testid="aiws-ext-dir" onChange={(event) => setDir(event.target.value)} />
            <span className="aiws-muted">{t('aiworkspace.ext.sourceFolderHint')}</span>
          </label>
          <div className="aiws-dialog-actions">
            <button type="button" data-testid="aiws-ext-edit" disabled={busy !== null} onClick={() => void editSource()}>{t('aiworkspace.ext.editSource')}</button>
            <button type="button" className="is-primary" data-testid="aiws-ext-build" disabled={busy !== null} onClick={() => void doBuild()}><Hammer size={15} aria-hidden="true" />{busy === 'build' ? t('aiworkspace.ext.building') : t('aiworkspace.ext.build')}</button>
            <button type="button" data-testid="aiws-ext-pull" disabled={busy !== null} onClick={() => void pull()}><RefreshCw size={15} aria-hidden="true" />{t('aiworkspace.ext.pull')}</button>
            <label className="aiws-inline-form"><input type="checkbox" data-testid="aiws-ext-watch" checked={watching !== null} onChange={(event) => { if (event.target.checked) { remember(dir); startWatch(store, def.entity_id, dir, setOutcome) } else stopWatch(def.entity_id) }} />{t('aiworkspace.ext.watch')}</label>
          </div>
        </>
      )}
      <Diagnostics outcome={outcome ?? watching?.last ?? null} />
      <div className="aiws-ext-users">
        <b>{t('aiworkspace.ext.instances', { count: users.length })}</b>
        <ul>{users.map((u) => <li key={u.entity_id}><code>{u.entity_id}</code> {u.title ?? ''}</li>)}</ul>
      </div>
      <div className="aiws-dialog-actions">
        {canBuild && <button type="button" data-testid="aiws-ext-insert" onClick={() => { if (requestInsertDefinition(def.entity_id)) onInsert(); else store.notify('info', t('aiworkspace.ext.openCanvasFirst')) }}>{t('aiworkspace.ext.insertInstance')}</button>}
        {canBuild && <button type="button" className="aiws-danger" data-testid="aiws-ext-remove" disabled={users.length > 0 || busy !== null} title={users.length > 0 ? t('aiworkspace.ext.removeBlocked') : undefined} onClick={() => void remove()}>{t('aiworkspace.ext.remove')}</button>}
      </div>
    </div>
  )
}

function NewControl({ onDone }: { onDone: (defEntityId: string) => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const principal = store.session.principal ?? 'me'
  const [name, setName] = useState('')
  const [title, setTitle] = useState('')
  const [kind, setKind] = useState<'declarative' | 'html'>('declarative')
  const [author, setAuthor] = useState(`did:bns:${principal}`)
  const [busy, setBusy] = useState(false)
  const [outcome, setOutcome] = useState<BuildOutcome | null>(null)
  const dir = homeControlDir(principal, store.session.info().title, name || '…')
  const create = async () => {
    setBusy(true)
    try {
      await createSource(dir, { authorDid: author, name, title: title || name, kind })
      const built = await buildFromHome(store, dir)
      setOutcome(built)
      if (built.ok && built.result.def_entity_id) {
        store.userState.set(sourceStateKey(built.result.def_entity_id), dir)
        onDone(built.result.def_entity_id)
      }
    } catch (error) {
      setOutcome({ ok: false, error: describeError(error), diagnostics: [] })
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-ext-form" data-testid="aiws-ext-new-form">
      <p className="aiws-muted">{t('aiworkspace.ext.newExplain')}</p>
      <label className="aiws-field">{t('aiworkspace.ext.name')}<input value={name} data-testid="aiws-ext-new-name" placeholder="kpi-card" onChange={(e) => setName(e.target.value.trim())} /></label>
      {name && !NAME.test(name) && <div className="aiws-warning">{t('aiworkspace.ext.nameRule')}</div>}
      <label className="aiws-field">{t('aiworkspace.ext.displayTitle')}<input value={title} data-testid="aiws-ext-new-title" onChange={(e) => setTitle(e.target.value)} /></label>
      <label className="aiws-field">{t('aiworkspace.ext.kind')}
        <select value={kind} data-testid="aiws-ext-new-kind" onChange={(e) => setKind(e.target.value as 'declarative' | 'html')}><option value="declarative">{t('aiworkspace.canvas.catalog.declarative')}</option><option value="html">HTML</option></select>
      </label>
      <label className="aiws-field">{t('aiworkspace.ext.author')}<input value={author} data-testid="aiws-ext-new-author" onChange={(e) => setAuthor(e.target.value.trim())} /></label>
      <div className="aiws-muted">{t('aiworkspace.ext.newFolder', { path: dir })}</div>
      <Diagnostics outcome={outcome} />
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-ext-new-create" disabled={busy || !NAME.test(name) || !author.startsWith('did:')} onClick={() => void create()}><PackagePlus size={15} aria-hidden="true" />{t('aiworkspace.ext.createAndBuild')}</button>
      </div>
    </div>
  )
}

function FromLibrary({ onDone }: { onDone: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const load = useCallback(() => store.session.extListLibrary(), [store])
  const library = useLoad<LibraryPackage[]>(load)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const present = useMemo(() => new Set(controlDefinitions(store.outline.all()).map((d) => d.ident).filter(Boolean)), [store])
  const key = (pkg: string, ident: string) => `${pkg}\u0000${ident}`
  const add = async () => {
    setBusy(true)
    setError(null)
    try {
      const items = [...picked].map((k) => { const [pkg_id, ident] = k.split('\u0000'); return { pkg_id, ident } })
      const r = await store.session.extImport(items)
      store.notify('info', t('aiworkspace.ext.imported', { count: r.controls.filter((c) => c.action === 'created').length }))
      onDone()
    } catch (e) {
      setError(describeError(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-ext-form" data-testid="aiws-ext-library-form">
      <p className="aiws-muted">{t('aiworkspace.ext.libraryExplain')}</p>
      {library.error && <div className="aiws-error">{library.error}</div>}
      {library.data?.length === 0 && <div className="aiws-muted" data-testid="aiws-ext-library-empty">{t('aiworkspace.ext.libraryEmpty')}</div>}
      {library.data?.map((pkg) => (
        <fieldset key={pkg.pkg_id} className="aiws-ext-package">
          <legend>{pkg.title} <span className="aiws-muted">{pkg.pkg_id} {pkg.version}</span></legend>
          {pkg.controls.map((c) => (
            <label key={c.ident} className="aiws-inline-form">
              <input type="checkbox" data-testid={`aiws-ext-library-pick-${c.name}`} disabled={present.has(c.ident)} checked={picked.has(key(pkg.pkg_id, c.ident))}
                onChange={() => setPicked((prev) => { const next = new Set(prev); const k = key(pkg.pkg_id, c.ident); if (next.has(k)) next.delete(k); else next.add(k); return next })} />
              {c.title ?? c.name} <code>{c.ident}</code> {present.has(c.ident) && <span className="aiws-muted">{t('aiworkspace.ext.alreadyHere')}</span>}
            </label>
          ))}
        </fieldset>
      ))}
      {error && <div className="aiws-error" role="alert">{error}</div>}
      <div className="aiws-dialog-actions"><button type="button" className="is-primary" data-testid="aiws-ext-library-add" disabled={busy || picked.size === 0} onClick={() => void add()}>{t('aiworkspace.ext.addPicked', { count: picked.size })}</button></div>
    </div>
  )
}

function ExportControls({ ids, homeReachable, onDone }: { ids: string[]; homeReachable: boolean; onDone: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const first = store.outline.get(ids[0])
  const [name, setName] = useState((first?.def_id ?? 'controls').replace(/[._]/g, '-'))
  const [version, setVersion] = useState('0.1.0')
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const principal = store.session.principal ?? 'me'
  const act = async (how: 'library' | 'bundle') => {
    setBusy(true)
    setError(null)
    try {
      if (how === 'library') {
        const r = await store.session.extInstallFrom(ids, { name, version })
        setDone(t('aiworkspace.ext.exportedLibrary', { pkg: r.pkg_id, version: r.version, action: r.action }))
      } else {
        const dir = homeExportDir(principal, name)
        const r = await exportToHome(store, ids, `${dir}/aiws`)
        setDone(t('aiworkspace.ext.exportedBundle', { path: `${dir}/aiws`, count: r.controls.length, command: `buckyos aiws ext-export ${ids.join(',')} --workspace ${store.session.workspaceId} --out ./${name} --pikg` }))
      }
    } catch (e) {
      setError(describeError(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="aiws-ext-form" data-testid="aiws-ext-export-form">
      <p className="aiws-muted">{t('aiworkspace.ext.exportExplain', { count: ids.length })}</p>
      <label className="aiws-field">{t('aiworkspace.ext.packageName')}<input value={name} data-testid="aiws-ext-export-name" onChange={(e) => setName(e.target.value.trim())} /></label>
      <label className="aiws-field">{t('aiworkspace.ext.packageVersion')}<input value={version} data-testid="aiws-ext-export-version" onChange={(e) => setVersion(e.target.value.trim())} /></label>
      {done && <div role="status" data-testid="aiws-ext-export-done">{done}</div>}
      {error && <div className="aiws-error" role="alert">{error}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-ext-export-library" disabled={busy || !/^[a-z0-9][a-z0-9.-]{0,127}$/.test(name)} onClick={() => void act('library')}>{t('aiworkspace.ext.exportToLibrary')}</button>
        {homeReachable && <button type="button" data-testid="aiws-ext-export-bundle" disabled={busy || !name} onClick={() => void act('bundle')}>{t('aiworkspace.ext.exportToHome')}</button>}
        {done && <button type="button" onClick={onDone}>{t('aiworkspace.ext.back')}</button>}
      </div>
    </div>
  )
}
