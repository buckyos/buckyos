/* Workspace controls (扩展控件 P0/P1, src/frame/aiworkspace/README.md “Extension controls”): the definitions this
 * Workspace holds, how each is built (its source in the user's home folder → `ext.build`, atomically), the instances
 * that use it, and the ways in and out — a new control from a skeleton, controls from the user's library, a .pikg file
 * into the library, an export to the library, to a bundle folder or as a .pikg the service packs. The home folder is
 * reached from the Desktop and from a Workspace's own host alike (Q24). Phones list only (no building, no adding). */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { Download, FileUp, Hammer, Library, Package, PackagePlus, Plus, Puzzle, RefreshCw, Upload } from 'lucide-react'
import { ServiceFailure } from '../../api/client'
import type { BlockDefPayload, BlockDefRead, BuildDiagnostic, ControlBinding, EntityEnvelope, ExtBuildResult, ExtMigration, ExtPackResult, Json, LibraryPackage } from '../../api/types'
import { describeError, type ReadOk } from '../../api/session'
import { useLoad, useOutlineVersion, useStore, useUserState, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { Modal } from '../shell/dialogs'
import { saveBlob } from '../shell/download'
import { controlDefinitions, HOST_MANIFEST_VERSION, interfaceChanges, type InterfaceChanges } from './controls'
import {
  buildFailure, buildFromHome, createSource, exportToHome, hasSource, homeControlDir, homeExportDir, pullToHome, sourceStateKey, startWatch, stopWatch, subscribeWatch, unbuiltDefinitions, watchOf, type BuildOutcome,
} from './controlSources'
import { requestInsertDefinition } from './extEvents'
import { desktopUIStore } from '../../../../models/DesktopUIDataModel'
import { filesOpenLaunch } from '../../../filebrowser/launch'

const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/

type View = { kind: 'list' } | { kind: 'new' } | { kind: 'library' } | { kind: 'export' } | { kind: 'upgrade'; defId: string; pkgId?: string }

export function ExtensionsDialog({ initialDefId, phone, onClose }: { initialDefId?: string; phone: boolean; onClose: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  useOutlineVersion()
  const defs = controlDefinitions(store.outline.all())
  const [chosen, setChosen] = useState<string | null>(initialDefId ?? defs[0]?.entity_id ?? null)
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const [view, setView] = useState<View>({ kind: 'list' })
  const instances = (id: string) => store.outline.all().filter((e) => (e.type_id === 'buckyos.cell' || e.type_id === 'buckyos.container') && e.def_id === id && !e.deleted).length
  const toggle = (id: string) => setChecked((prev) => { const next = new Set(prev); if (next.has(id)) next.delete(id); else next.add(id); return next })
  const live = store.session.status().kind === 'live'
  const canBuild = !phone && live && store.session.info().capabilities.some((c) => c === 'structure' || c === 'update')
  const current = defs.find((d) => d.entity_id === chosen) ?? null
  // a .pikg file of the user's into the library (Q21): uploaded into its own exports bucket, then `ext.install`
  const pikgInput = useRef<HTMLInputElement>(null)
  const [importing, setImporting] = useState(false)
  const importPikg = async (file: File) => {
    setImporting(true)
    try {
      const r = await store.session.installPikgFile(file)
      store.notify('info', t('aiworkspace.ext.pikgInstalled', { pkg: r.pkg_id, version: r.version, action: r.action }))
      setView({ kind: 'library' })
    } catch (error) {
      store.notify('error', t('aiworkspace.ext.pikgInstallFailed', { error: describeError(error) }))
    } finally {
      setImporting(false)
    }
  }
  return (
    <Modal label={t('aiworkspace.ext.title')} testId="aiws-ext-dialog" className="aiws-ext-dialog" onClose={onClose}>
      <div className="aiws-dialog-head">
        <Puzzle size={18} aria-hidden="true" /><b>{t('aiworkspace.ext.title')}</b><span className="aiws-grow" />
        {canBuild && view.kind === 'list' && <>
          <button type="button" data-testid="aiws-ext-new" onClick={() => setView({ kind: 'new' })}><Plus size={15} aria-hidden="true" />{t('aiworkspace.ext.new')}</button>
          <button type="button" data-testid="aiws-ext-library" onClick={() => setView({ kind: 'library' })}><Library size={15} aria-hidden="true" />{t('aiworkspace.ext.fromLibrary')}</button>
          <button type="button" data-testid="aiws-ext-import-pikg" disabled={importing} onClick={() => pikgInput.current?.click()}><FileUp size={15} aria-hidden="true" />{t('aiworkspace.ext.importPikg')}</button>
          <input ref={pikgInput} type="file" accept=".pikg" hidden data-testid="aiws-ext-import-pikg-file" onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importPikg(file) }} />
          <button type="button" data-testid="aiws-ext-export" disabled={checked.size === 0} title={checked.size === 0 ? t('aiworkspace.ext.exportPick') : undefined} onClick={() => setView({ kind: 'export' })}><Upload size={15} aria-hidden="true" />{t('aiworkspace.ext.export', { count: checked.size })}</button>
        </>}
        {view.kind !== 'list' && <button type="button" className="aiws-link" onClick={() => setView({ kind: 'list' })}>{t('aiworkspace.ext.back')}</button>}
        <button type="button" className="aiws-link" data-testid="aiws-ext-close" onClick={onClose}>{t('aiworkspace.common.close')}</button>
      </div>
      {view.kind === 'new' && <NewControl onDone={(id) => { setChosen(id); setView({ kind: 'list' }) }} />}
      {view.kind === 'library' && <FromLibrary onDone={() => setView({ kind: 'list' })} onUpgrade={(defId, pkgId) => setView({ kind: 'upgrade', defId, pkgId })} />}
      {view.kind === 'upgrade' && <UpgradeControl defId={view.defId} pkgId={view.pkgId} onDone={() => { setChosen(view.defId); setView({ kind: 'list' }) }} />}
      {view.kind === 'export' && <ExportControls ids={[...checked]} onDone={() => setView({ kind: 'list' })} />}
      {view.kind === 'list' && (
        <div className="aiws-ext-body">
          <ul className="aiws-ext-list" data-testid="aiws-ext-list">
            {defs.length === 0 && <li className="aiws-muted">{t('aiworkspace.ext.empty')}</li>}
            {defs.map((d) => (
              <li key={d.entity_id} className={d.entity_id === chosen ? 'is-active' : undefined}>
                {canBuild && <input type="checkbox" aria-label={t('aiworkspace.ext.pick', { name: d.title ?? d.entity_id })} checked={checked.has(d.entity_id)} onChange={() => toggle(d.entity_id)} data-testid={`aiws-ext-check-${d.entity_id}`} />}
                <button type="button" className="aiws-ext-row" data-testid={`aiws-ext-row-${d.entity_id}`} onClick={() => setChosen(d.entity_id)}>
                  <span className="aiws-ext-row-title">{d.title ?? d.def_id ?? d.entity_id}</span>
                  <span className="aiws-muted">{kindLabel(d.def_kind, t)} · v{d.def_version ?? 1} · {d.provenance_pkg ? t('aiworkspace.ext.fromPackage', { pkg: d.provenance_pkg }) : d.source_dir_id ? t('aiworkspace.ext.localBuild') : t('aiworkspace.ext.inline')} · {t('aiworkspace.ext.instances', { count: instances(d.entity_id) })}</span>
                </button>
              </li>
            ))}
          </ul>
          {current ? <DefinitionPanel key={current.entity_id} def={current} canBuild={canBuild} onInsert={onClose} onUpgrade={(pkgId) => setView({ kind: 'upgrade', defId: current.entity_id, pkgId })} /> : <div className="aiws-muted">{t('aiworkspace.ext.chooseOne')}</div>}
        </div>
      )}
    </Modal>
  )
}

/** What a definition is: declarative, HTML or composite. */
function kindLabel(kind: string | null | undefined, t: (key: string) => string): string {
  return kind === 'html' ? 'HTML' : kind === 'composite' ? t('aiworkspace.ext.kindComposite') : t('aiworkspace.canvas.catalog.declarative')
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
          {d.dependents && d.dependents.length > 0 && <span className="aiws-muted"> {t('aiworkspace.ext.dependentsAffected', { names: d.dependents.map((x) => x.ident).join(', ') })}</span>}
        </li>
      ))}
    </ul>
  )
}

function DefinitionPanel({ def, canBuild, onInsert, onUpgrade }: { def: EntityEnvelope; canBuild: boolean; onInsert: () => void; onUpgrade: (pkgId: string) => void }) {
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
  const users = store.outline.all().filter((e) => (e.type_id === 'buckyos.cell' || e.type_id === 'buckyos.container') && e.def_id === def.entity_id && !e.deleted)
  // the home folder holds changes this Workspace has not built (P0 遗留 L3): said here, never blocking
  const [unbuilt, setUnbuilt] = useState(false)
  useEffect(() => {
    let live = true
    if (canBuild && saved) void unbuiltDefinitions(store, [def.entity_id]).then((ids) => { if (live) setUnbuilt(ids.length > 0) })
    return () => { live = false }
  }, [store, def.entity_id, saved, canBuild, version])
  // a newer version of the package it came from is in the library (P2, Q30): offered as an upgrade
  const provenance = payload?.provenance
  const loadLibrary = useCallback(() => (provenance?.pkg_id && canBuild ? store.session.extListLibrary() : Promise.resolve([] as LibraryPackage[])), [store, provenance?.pkg_id, canBuild])
  const library = useLoad<LibraryPackage[]>(loadLibrary, version)
  const newer = library.data?.find((p) => p.pkg_id === provenance?.pkg_id && p.version !== provenance?.version && !tooNew(p))
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
        <dt>{t('aiworkspace.canvas.catalog.type')}</dt><dd>{kindLabel(def.def_kind, t)} · v{payload?.version ?? def.def_version ?? 1}</dd>
        {payload?.deps && payload.deps.length > 0 && <><dt>{t('aiworkspace.ext.deps')}</dt><dd data-testid="aiws-ext-deps">{payload.deps.map((d) => d.ident).join(', ')}</dd></>}
        {payload?.provenance && <><dt>{t('aiworkspace.ext.source')}</dt><dd>{t('aiworkspace.ext.fromPackage', { pkg: `${payload.provenance.pkg_id ?? ''} ${payload.provenance.version ?? ''}` })}</dd></>}
        <dt>{t('aiworkspace.ext.lastBuild')}</dt><dd data-testid="aiws-ext-built-at">{build ? dateTime(build.built_at) : t('aiworkspace.ext.neverBuilt')}</dd>
      </dl>
      {payload?.description && <p className="aiws-muted">{payload.description}</p>}
      {payload?.bindings && Object.keys(payload.bindings).length > 0 && <div className="aiws-muted">{t('aiworkspace.ext.bindingsList', { names: Object.entries(payload.bindings).map(([n, b]) => `${n}${b.required ? '*' : ''}`).join(', ') })}</div>}
      {unbuilt && <div className="aiws-warning" data-testid="aiws-ext-unbuilt">{t('aiworkspace.ext.unbuilt')}</div>}
      {newer && (
        <div className="aiws-warning" data-testid="aiws-ext-new-version" data-version={newer.version}>
          {t('aiworkspace.ext.upgrade.available', { version: newer.version })}{' '}
          <button type="button" data-testid="aiws-ext-upgrade" onClick={() => onUpgrade(newer.pkg_id)}>{t('aiworkspace.ext.upgrade.action')}</button>
        </div>
      )}
      {canBuild && (
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
  const [kind, setKind] = useState<'declarative' | 'html' | 'composite'>('declarative')
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
        <select value={kind} data-testid="aiws-ext-new-kind" onChange={(e) => setKind(e.target.value as 'declarative' | 'html' | 'composite')}><option value="declarative">{t('aiworkspace.canvas.catalog.declarative')}</option><option value="html">HTML</option><option value="composite">{t('aiworkspace.ext.kindComposite')}</option></select>
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

/** A package newer than this host reads (Q33): listed with a mark, never imported. */
function tooNew(pkg: LibraryPackage): boolean {
  return pkg.needs_newer === true || (pkg.manifest_version ?? 0) > HOST_MANIFEST_VERSION
}

function FromLibrary({ onDone, onUpgrade }: { onDone: () => void; onUpgrade: (defId: string, pkgId: string) => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const load = useCallback(() => store.session.extListLibrary(), [store])
  const library = useLoad<LibraryPackage[]>(load)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // the identity is here from another source (EXT_IDENT_EXISTS): an upgrade, not a second definition (Q1, Q30)
  const [exists, setExists] = useState<{ defId: string; pkgId: string; version: string | null } | null>(null)
  const here = useMemo(() => new Map(controlDefinitions(store.outline.all()).filter((d) => d.ident).map((d) => [d.ident as string, d.entity_id])), [store])
  const present = useMemo(() => new Set(here.keys()), [here])
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
      if (e instanceof ServiceFailure && e.subCode === 'EXT_IDENT_EXISTS' && typeof e.data?.def_entity_id === 'string') {
        const ident = String(e.data.ident ?? '')
        const pkgId = [...picked].map((k) => k.split('\u0000')).find(([, i]) => i === ident)?.[0] ?? [...picked][0]?.split('\u0000')[0] ?? ''
        setExists({ defId: e.data.def_entity_id, pkgId, version: typeof e.data.library_version === 'string' ? e.data.library_version : null })
      } else setError(describeError(e))
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
        <fieldset key={pkg.pkg_id} className="aiws-ext-package" data-testid={`aiws-ext-package-${pkg.pkg_id}`} data-needs-newer={tooNew(pkg) ? 'true' : undefined}>
          <legend>{pkg.title} <span className="aiws-muted">{pkg.pkg_id} {pkg.version}</span></legend>
          {tooNew(pkg) && <div className="aiws-warning" data-testid={`aiws-ext-needs-newer-${pkg.pkg_id}`}>{t('aiworkspace.ext.needsNewer', { version: pkg.manifest_version ?? '?' })}</div>}
          {pkg.controls.map((c) => {
            const blocked = tooNew(pkg) || c.needs_newer === true
            const defId = here.get(c.ident)
            return (
              <label key={c.ident} className="aiws-inline-form">
                <input type="checkbox" data-testid={`aiws-ext-library-pick-${c.name}`} disabled={present.has(c.ident) || blocked} checked={picked.has(key(pkg.pkg_id, c.ident))}
                  onChange={() => setPicked((prev) => { const next = new Set(prev); const k = key(pkg.pkg_id, c.ident); if (next.has(k)) next.delete(k); else next.add(k); return next })} />
                {c.title ?? c.name} <code>{c.ident}</code> {present.has(c.ident) && <span className="aiws-muted">{t('aiworkspace.ext.alreadyHere')}</span>}
                {defId && !blocked && <button type="button" className="aiws-link" data-testid={`aiws-ext-library-upgrade-${c.name}`} onClick={() => onUpgrade(defId, pkg.pkg_id)}>{t('aiworkspace.ext.upgrade.action')}</button>}
              </label>
            )
          })}
          {(pkg.templates ?? []).length > 0 && (
            <div className="aiws-muted" data-testid={`aiws-ext-library-templates-${pkg.pkg_id}`}>{t('aiworkspace.ext.libraryTemplates', { names: (pkg.templates ?? []).map((tpl) => tpl.title || tpl.id).join(', ') })}</div>
          )}
        </fieldset>
      ))}
      {exists && (
        <div className="aiws-warning" role="alert" data-testid="aiws-ext-ident-exists">
          {t('aiworkspace.ext.upgrade.exists', { version: exists.version ?? '?' })}{' '}
          <button type="button" data-testid="aiws-ext-ident-exists-upgrade" onClick={() => onUpgrade(exists.defId, exists.pkgId)}>{t('aiworkspace.ext.upgrade.action')}</button>
        </div>
      )}
      {error && <div className="aiws-error" role="alert">{error}</div>}
      <div className="aiws-dialog-actions"><button type="button" className="is-primary" data-testid="aiws-ext-library-add" disabled={busy || picked.size === 0} onClick={() => void add()}>{t('aiworkspace.ext.addPicked', { count: picked.size })}</button></div>
    </div>
  )
}

function ExportControls({ ids, onDone }: { ids: string[]; onDone: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const first = store.outline.get(ids[0])
  const [name, setName] = useState((first?.def_id ?? 'controls').replace(/[._]/g, '-'))
  const [version, setVersion] = useState('0.1.0')
  const [busy, setBusy] = useState(false)
  const [done, setDone] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pack, setPack] = useState<ExtPackResult | null>(null)
  const principal = store.session.principal ?? 'me'
  // what is exported is the Workspace's snapshot: home folders with changes not built yet are named (P0 遗留 L3)
  const [unbuilt, setUnbuilt] = useState<string[]>([])
  const idsKey = ids.join(',')
  useEffect(() => {
    let live = true
    void unbuiltDefinitions(store, idsKey.split(',')).then((found) => { if (live) setUnbuilt(found) })
    return () => { live = false }
  }, [store, idsKey])
  const run = async (work: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try { await work() } catch (e) { setError(describeError(e)) } finally { setBusy(false) }
  }
  const act = (how: 'library' | 'bundle' | 'pikg') => run(async () => {
    if (how === 'library') {
      const r = await store.session.extInstallFrom(ids, { name, version })
      setDone(t('aiworkspace.ext.exportedLibrary', { pkg: r.pkg_id, version: r.version, action: r.action }))
    } else if (how === 'pikg') {
      // the service packs it (Q20) into the user's own exports bucket: download it, or put it straight into the library
      const r = await store.session.extPack(ids, { name, version, title: first?.title ?? name })
      setPack(r)
      setDone(t('aiworkspace.ext.packed', { file: r.key, pkg: r.pkg_id, size: r.size }))
    } else {
      const dir = homeExportDir(principal, name)
      const r = await exportToHome(store, ids, `${dir}/aiws`)
      setDone(t('aiworkspace.ext.exportedBundle', { path: `${dir}/aiws`, count: r.controls.length, command: `buckyos aiws ext-export ${ids.join(',')} --workspace ${store.session.workspaceId} --out ./${name} --pikg` }))
    }
  })
  const download = () => run(async () => { if (pack) saveBlob(await store.session.downloadPack(pack), pack.key) })
  const install = () => run(async () => {
    if (!pack) return
    const r = await store.session.installPack({ bucket_id: pack.bucket_id, key: pack.key })
    setPack(null)
    setDone(t('aiworkspace.ext.exportedLibrary', { pkg: r.pkg_id, version: r.version, action: r.action }))
  })
  const validName = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(name)
  return (
    <div className="aiws-ext-form" data-testid="aiws-ext-export-form">
      <p className="aiws-muted">{t('aiworkspace.ext.exportExplain', { count: ids.length })}</p>
      {unbuilt.length > 0 && <div className="aiws-warning" data-testid="aiws-ext-export-unbuilt">{t('aiworkspace.ext.unbuiltExport', { names: unbuilt.map((id) => store.outline.get(id)?.title ?? id).join(', ') })}</div>}
      <label className="aiws-field">{t('aiworkspace.ext.packageName')}<input value={name} data-testid="aiws-ext-export-name" onChange={(e) => setName(e.target.value.trim())} /></label>
      <label className="aiws-field">{t('aiworkspace.ext.packageVersion')}<input value={version} data-testid="aiws-ext-export-version" onChange={(e) => setVersion(e.target.value.trim())} /></label>
      {done && <div role="status" data-testid="aiws-ext-export-done">{done}</div>}
      {error && <div className="aiws-error" role="alert">{error}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" className="is-primary" data-testid="aiws-ext-export-library" disabled={busy || !/^[a-z0-9][a-z0-9.-]{0,127}$/.test(name)} onClick={() => void act('library')}>{t('aiworkspace.ext.exportToLibrary')}</button>
        <button type="button" data-testid="aiws-ext-export-bundle" disabled={busy || !name} onClick={() => void act('bundle')}>{t('aiworkspace.ext.exportToHome')}</button>
        <button type="button" data-testid="aiws-ext-export-pikg" disabled={busy || !validName} title={validName ? undefined : t('aiworkspace.ext.pikgNameRule')} onClick={() => void act('pikg')}><Package size={15} aria-hidden="true" />{t('aiworkspace.ext.exportPikg')}</button>
        {pack && <button type="button" data-testid="aiws-ext-pikg-download" disabled={busy} onClick={() => void download()}><Download size={15} aria-hidden="true" />{t('aiworkspace.ext.pikgDownload')}</button>}
        {pack && <button type="button" data-testid="aiws-ext-pikg-install" disabled={busy} onClick={() => void install()}>{t('aiworkspace.ext.pikgInstall')}</button>}
        {done && <button type="button" onClick={onDone}>{t('aiworkspace.ext.back')}</button>}
      </div>
    </div>
  )
}

type UpgradeCheck =
  | { kind: 'ok'; result: ExtBuildResult }
  | { kind: 'local'; files: { added: string[]; removed: string[]; changed: string[] } }
  | { kind: 'failed'; error: string; diagnostics: BuildDiagnostic[] }

const fromBase64 = (b64: string) => new TextDecoder().decode(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)))

/** The upgrade of a definition to the version of its package in the library (P2, Q30): what changed here since the
 * import (never overwritten silently), the instances it touches, how its interface changes, and a minimal migration —
 * renamed or dropped properties and bindings, values for new required ones, the slot each removed slot moves to. A dry
 * run checks it first; the upgrade is one Commit. */
function UpgradeControl({ defId, pkgId, onDone }: { defId: string; pkgId?: string; onDone: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const def = store.outline.get(defId)
  const ident = def?.ident ?? ''
  const read = useLoad<ReadOk<BlockDefRead>>(useCallback(() => store.session.read<BlockDefRead>(defId), [store, defId]))
  const current = read.data?.content.payload
  const pkg = pkgId ?? current?.provenance?.pkg_id ?? def?.provenance_pkg ?? ''
  const library = useLoad<LibraryPackage[]>(useCallback(() => store.session.extListLibrary(), [store]))
  const target = library.data?.find((p) => p.pkg_id === pkg)
  // the newer version's interface, from its files in the library
  const loadNext = useCallback(async () => {
    if (!pkg || !ident) return null
    const controls = await store.session.extLibraryBundle(pkg, [ident])
    const files = controls.find((c) => c.ident === ident)?.files ?? []
    const text = (name: string) => { const f = files.find((x) => x.name === name); return f ? fromBase64(f.base64) : null }
    const component = JSON.parse(text('component.json') ?? '{}') as { config_schema?: Json; bindings?: Record<string, ControlBinding>; events?: Record<string, unknown>; entry?: unknown; kind?: string }
    const entry = typeof component.entry === 'string' ? component.entry : 'composition.json'
    const composition = component.kind === 'composite' ? JSON.parse(text(entry) ?? '{}') as { slots?: Record<string, unknown> } : null
    return { component, slots: composition ? Object.keys(composition.slots ?? {}) : null }
  }, [store, pkg, ident])
  const next = useLoad(loadNext)
  const changes: InterfaceChanges | null = current && next.data ? interfaceChanges(current as BlockDefPayload, next.data.component, next.data.slots) : null
  const [rename, setRename] = useState<Record<string, string>>({})
  const [fill, setFill] = useState<Record<string, string>>({})
  const [bindRename, setBindRename] = useState<Record<string, string>>({})
  const [slotMove, setSlotMove] = useState<Record<string, string>>({})
  const [force, setForce] = useState(false)
  const [busy, setBusy] = useState(false)
  const [check, setCheck] = useState<UpgradeCheck | null>(null)
  const [unbuilt, setUnbuilt] = useState(false)
  useEffect(() => {
    let live = true
    void unbuiltDefinitions(store, [defId]).then((ids) => { if (live) setUnbuilt(ids.length > 0) })
    return () => { live = false }
  }, [store, defId])
  const instances = store.outline.all().filter((e) => (e.type_id === 'buckyos.cell' || e.type_id === 'buckyos.container') && e.def_id === defId && !e.deleted)
  const migration = (): Record<string, ExtMigration> | undefined => {
    if (!changes) return undefined
    const m: ExtMigration = {}
    const renamed = Object.fromEntries(changes.removedConfig.filter((k) => rename[k]).map((k) => [k, rename[k]]))
    const dropped = changes.removedConfig.filter((k) => !rename[k])
    const filled = Object.fromEntries(changes.requiredConfig.filter((k) => fill[k] !== undefined && fill[k] !== '').map((k) => [k, parseValue(fill[k])]))
    if (Object.keys(renamed).length || dropped.length || Object.keys(filled).length) m.config = { ...(Object.keys(renamed).length ? { rename: renamed } : {}), ...(dropped.length ? { drop: dropped } : {}), ...(Object.keys(filled).length ? { fill: filled } : {}) }
    const bRenamed = Object.fromEntries(changes.removedBindings.filter((k) => bindRename[k]).map((k) => [k, bindRename[k]]))
    const bDropped = changes.removedBindings.filter((k) => !bindRename[k])
    if (Object.keys(bRenamed).length || bDropped.length) m.bindings = { ...(Object.keys(bRenamed).length ? { rename: bRenamed } : {}), ...(bDropped.length ? { drop: bDropped } : {}) }
    const moved = Object.fromEntries(changes.removedSlots.filter((k) => slotMove[k]).map((k) => [k, slotMove[k]]))
    if (Object.keys(moved).length) m.slots = { move: moved }
    if (changes.removedEvents.length) m.events = { drop: changes.removedEvents }
    return Object.keys(m).length > 0 && ident ? { [ident]: m } : undefined
  }
  const run = async (dryRun: boolean) => {
    setBusy(true)
    try {
      const result = await store.session.extUpgrade([{ def_entity_id: defId, ...(pkg ? { pkg_id: pkg } : {}) }], { migration: migration(), force: force || undefined, dry_run: dryRun || undefined })
      setCheck({ kind: 'ok', result })
      if (!dryRun) {
        store.notify('info', result.status === 'unchanged' ? t('aiworkspace.ext.upgrade.unchanged') : t('aiworkspace.ext.upgrade.done', { name: def?.title ?? ident, from: result.from ?? '?', to: result.to ?? target?.version ?? '?' }))
        onDone()
      }
    } catch (error) {
      if (error instanceof ServiceFailure && error.subCode === 'EXT_LOCAL_CHANGES') {
        const files = (error.data?.files ?? {}) as { added?: string[]; removed?: string[]; changed?: string[] }
        setCheck({ kind: 'local', files: { added: files.added ?? [], removed: files.removed ?? [], changed: files.changed ?? [] } })
      } else {
        const failed = buildFailure(error)
        setCheck({ kind: 'failed', error: failed.ok ? '' : failed.error, diagnostics: failed.ok ? [] : failed.diagnostics })
      }
    } finally {
      setBusy(false)
    }
  }
  const firstCheck = useRef(false)
  useEffect(() => {
    if (firstCheck.current || !current || !next.data) return
    firstCheck.current = true
    void run(true)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the first dry run, once both versions are known
  }, [current, next.data])
  const newConfig = changes?.newConfig ?? []
  const newBindings = changes?.newBindings ?? []
  const newSlots = changes?.newSlots ?? []
  const nothingChanges = changes && !changes.removedConfig.length && !changes.requiredConfig.length && !changes.removedBindings.length && !changes.removedSlots.length && !changes.removedEvents.length
  return (
    <div className="aiws-ext-form" data-testid="aiws-ext-upgrade-form" data-def={defId}>
      <h3>{t('aiworkspace.ext.upgrade.title', { name: def?.title ?? ident })}</h3>
      <div data-testid="aiws-ext-upgrade-versions">{t('aiworkspace.ext.upgrade.versions', { pkg, from: current?.provenance?.version ?? t('aiworkspace.ext.localBuild'), to: target?.version ?? '?' })}</div>
      {(library.error || next.error || read.error) && <div className="aiws-error">{library.error ?? next.error ?? read.error}</div>}
      {unbuilt && <div className="aiws-warning" data-testid="aiws-ext-upgrade-unbuilt">{t('aiworkspace.ext.upgrade.unbuilt')}</div>}
      <div className="aiws-muted" data-testid="aiws-ext-upgrade-instances">{t('aiworkspace.ext.upgrade.instances', { count: instances.length })}</div>
      {changes && nothingChanges && <div className="aiws-muted" data-testid="aiws-ext-upgrade-compatible">{t('aiworkspace.ext.upgrade.compatible')}</div>}
      {changes && !nothingChanges && (
        <fieldset className="aiws-ext-package" data-testid="aiws-ext-upgrade-migration">
          <legend>{t('aiworkspace.ext.upgrade.migration')}</legend>
          {changes.removedConfig.map((k) => (
            <label key={`c-${k}`} className="aiws-field">{t('aiworkspace.ext.upgrade.propertyGone', { name: k })}
              <select value={rename[k] ?? ''} data-testid={`aiws-ext-upgrade-rename-${k}`} onChange={(e) => setRename((p) => ({ ...p, [k]: e.target.value }))}>
                <option value="">{t('aiworkspace.ext.upgrade.drop')}</option>
                {newConfig.map((n) => <option key={n} value={n}>{t('aiworkspace.ext.upgrade.renameTo', { name: n })}</option>)}
              </select>
            </label>
          ))}
          {changes.requiredConfig.map((k) => (
            <label key={`f-${k}`} className="aiws-field">{t('aiworkspace.ext.upgrade.propertyRequired', { name: k })}
              <input value={fill[k] ?? ''} data-testid={`aiws-ext-upgrade-fill-${k}`} onChange={(e) => setFill((p) => ({ ...p, [k]: e.target.value }))} />
            </label>
          ))}
          {changes.removedBindings.map((k) => (
            <label key={`b-${k}`} className="aiws-field">{t('aiworkspace.ext.upgrade.bindingGone', { name: k })}
              <select value={bindRename[k] ?? ''} data-testid={`aiws-ext-upgrade-binding-${k}`} onChange={(e) => setBindRename((p) => ({ ...p, [k]: e.target.value }))}>
                <option value="">{t('aiworkspace.ext.upgrade.drop')}</option>
                {newBindings.map((n) => <option key={n} value={n}>{t('aiworkspace.ext.upgrade.renameTo', { name: n })}</option>)}
              </select>
            </label>
          ))}
          {changes.removedSlots.map((k) => (
            <label key={`s-${k}`} className="aiws-field">{t('aiworkspace.ext.upgrade.slotGone', { name: k })}
              <select value={slotMove[k] ?? ''} data-testid={`aiws-ext-upgrade-slot-${k}`} onChange={(e) => setSlotMove((p) => ({ ...p, [k]: e.target.value }))}>
                <option value="">{t('aiworkspace.ext.upgrade.chooseSlot')}</option>
                {newSlots.map((n) => <option key={n} value={n}>{n}</option>)}
              </select>
            </label>
          ))}
          {changes.removedEvents.length > 0 && <div className="aiws-muted">{t('aiworkspace.ext.upgrade.eventsGone', { names: changes.removedEvents.join(', ') })}</div>}
        </fieldset>
      )}
      {check?.kind === 'local' && (
        <div className="aiws-warning" data-testid="aiws-ext-upgrade-local">
          <div>{t('aiworkspace.ext.upgrade.localChanges')}</div>
          <ul>
            {check.files.changed.map((f) => <li key={`c-${f}`}><code>{f}</code> {t('aiworkspace.ext.upgrade.fileChanged')}</li>)}
            {check.files.added.map((f) => <li key={`a-${f}`}><code>{f}</code> {t('aiworkspace.ext.upgrade.fileAdded')}</li>)}
            {check.files.removed.map((f) => <li key={`r-${f}`}><code>{f}</code> {t('aiworkspace.ext.upgrade.fileRemoved')}</li>)}
          </ul>
          <label className="aiws-inline-form"><input type="checkbox" data-testid="aiws-ext-upgrade-force" checked={force} onChange={(e) => setForce(e.target.checked)} />{t('aiworkspace.ext.upgrade.force')}</label>
        </div>
      )}
      {check?.kind === 'failed' && (
        <div className="aiws-error" role="alert" data-testid="aiws-ext-upgrade-failed">{check.error}{check.diagnostics.length > 0 && <DiagList items={check.diagnostics} />}</div>
      )}
      {check?.kind === 'ok' && check.result.status === 'checked' && (
        <div role="status" data-testid="aiws-ext-upgrade-checked">{t('aiworkspace.ext.upgrade.checked', { count: (check.result.plan ?? []).length })}</div>
      )}
      <div className="aiws-dialog-actions">
        <button type="button" data-testid="aiws-ext-upgrade-check" disabled={busy || !current} onClick={() => void run(true)}>{t('aiworkspace.ext.upgrade.check')}</button>
        <button type="button" className="is-primary" data-testid="aiws-ext-upgrade-run" disabled={busy || !current || (check?.kind === 'local' && !force)} onClick={() => void run(false)}>{t('aiworkspace.ext.upgrade.run', { version: target?.version ?? '?' })}</button>
      </div>
    </div>
  )
}

/** A value typed into the migration form: JSON when it parses (numbers, booleans), else the text. */
function parseValue(text: string): Json {
  try { return JSON.parse(text) as Json } catch { return text }
}
