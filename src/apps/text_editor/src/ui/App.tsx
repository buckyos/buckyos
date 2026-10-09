import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { EditorView } from '@codemirror/view'
import { openSearchPanel, closeSearchPanel, gotoLine } from '@codemirror/search'
import { contentRefString, parentSource } from 'buckyos/content'
import { DocumentEditor } from '../editor/setup.ts'
import { languages } from '../editor/languages.ts'
import { extractOutline, type OutlineItem } from '../editor/outline/index.ts'
import { Workspace, dirty, type EditorDocument, type Group } from '../model/workspace.ts'
import { translate } from '../i18n/index.ts'
import type { Entry } from 'buckyos/nfsp'

export const controllers = new Map<string, DocumentEditor>()
export let activeView: EditorView | undefined
function Editor({ workspace: w, doc, group, onCursor }: { workspace: Workspace; doc: EditorDocument; group: Group; onCursor: () => void }) {
  const parent = useRef<HTMLDivElement>(null)
  useEffect(() => {
    let controller = controllers.get(doc.id)
    if (!controller) { controller = new DocumentEditor(doc, w); controllers.set(doc.id, controller) }
    const view = controller.attach(parent.current!, v => { if (v.hasFocus) { activeView = v; onCursor() } })
    const focus = () => { activeView = view; w.activeGroup = group.id; onCursor(); w.emit() }
    view.contentDOM.addEventListener('focus', focus)
    view.focus()
    return () => { view.contentDOM.removeEventListener('focus', focus); controller.detach(view); if (activeView === view) activeView = undefined }
  }, [doc.id, group.id, w])
  useEffect(() => { controllers.get(doc.id)?.configure() })
  return <div className="editor" ref={parent} aria-label={doc.name} />
}
export function App({ workspace: w }: { workspace: Workspace }) {
  useSyncExternalStore(w.subscribe, w.getSnapshot)
  const [menu, setMenu] = useState(false)
  const [cursor, setCursor] = useState(0)
  const [outline, setOutline] = useState<OutlineItem[]>([])
  const t = (key: string) => translate(w.locale, key)
  const doc = w.active
  const refreshCursor = () => setCursor(v => v + 1)
  useEffect(() => {
    document.documentElement.dataset.theme = w.theme
    document.documentElement.lang = w.locale.startsWith('zh') ? 'zh-CN' : 'en'
    for (const [id] of controllers) if (!w.documents.has(id)) controllers.delete(id)
    const timer = setTimeout(() => setOutline(doc && !doc.largeFile && controllers.has(doc.id) ? extractOutline(controllers.get(doc.id)!.state) : []), 250)
    return () => clearTimeout(timer)
  }, [w.revision, cursor, doc?.id, w.theme, w.locale])
  const openLocation = () => w.chooseLocation(path => w.open({ requestId: crypto.randomUUID(), source: { kind: 'cyfs-path', path } }))
  const saveAs = () => w.chooseLocation(path => w.save(w.active, { path }).then(() => {}))
  const action = (fn: () => void) => { fn(); setMenu(false) }
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      if (w.choice && event.key !== 'Escape') return
      if ((event.target as HTMLElement)?.closest('dialog,input,select') && !(event.target as HTMLElement)?.closest('.cm-editor')) return
      const mod = /Mac/.test(navigator.platform) ? event.metaKey : event.ctrlKey
      const alt = event.altKey && (!/Mac/.test(navigator.platform) || event.ctrlKey)
      const key = event.key.toLowerCase()
      let run: (() => void) | undefined
      if (mod && key === 's') run = event.shiftKey ? saveAs : () => w.run(w.save())
      if (mod && key === 'p') run = () => { w.panel = 'quick'; w.emit() }
      if (mod && key === 'b') run = () => { w.sidebar = !w.sidebar; w.emit() }
      if (mod && key === '\\') run = () => w.split('columns-2')
      if (mod && ['=', '+', '-', '0'].includes(key)) run = () => w.run(w.updateSettings({ ...w.settings, fontSize: key === '0' ? 14 : Math.max(8, Math.min(40, w.settings.fontSize + (key === '-' ? -1 : 1))) }))
      if (alt && key === 'n') run = () => { w.newDocument() }
      if (alt && key === 'w' && w.active) run = () => w.run(w.close(w.active!))
      if (alt && event.shiftKey && ['Digit1', 'Digit2', 'Digit8'].includes(event.code)) run = () => w.split(event.code === 'Digit1' ? 'single' : event.code === 'Digit2' ? 'columns-2' : 'rows-2')
      else if (alt && /^[1-9]$/.test(key)) run = () => { w.group.active = w.group.tabs[Number(key) - 1]?.doc ?? w.group.active; w.emit() }
      if (alt && ['[', ']'].includes(key)) run = () => { const i = w.group.tabs.findIndex(t => t.doc === w.group.active); w.group.active = w.group.tabs[(i + (key === '[' ? -1 : 1) + w.group.tabs.length) % w.group.tabs.length]?.doc; w.emit() }
      if (event.key === 'Escape') run = () => { if (w.choice) w.choice.resolve('cancel'); else if (w.panel) { w.panel = undefined; w.emit() }; setMenu(false) }
      if (run) { event.preventDefault(); run() }
    }
    window.addEventListener('keydown', listener)
    return () => window.removeEventListener('keydown', listener)
  })
  useEffect(() => {
    if (!w.choice && !w.panel) return
    const previous = document.activeElement as HTMLElement | null
    const container = document.querySelector<HTMLElement>(w.choice ? '[role="alertdialog"]' : '[role="dialog"]')
    const focusable = () => [...(container?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),[tabindex="0"]') ?? [])]
    focusable()[0]?.focus()
    const trap = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return
      const nodes = focusable(); if (!nodes.length) return
      const at = nodes.indexOf(document.activeElement as HTMLElement)
      if (at < 0 || !event.shiftKey && at === nodes.length - 1 || event.shiftKey && at === 0) {
        event.preventDefault(); (event.shiftKey ? nodes.at(-1) : nodes[0])?.focus()
      }
    }
    document.addEventListener('keydown', trap, true)
    return () => { document.removeEventListener('keydown', trap, true); previous?.focus() }
  }, [w.choice, w.panel])
  const state = activeView?.state
  const position = state?.selection.main.head ?? 0
  const line = state?.doc.lineAt(position)
  const currentHeading = outline.filter(h => h.from <= position).at(-1)
  const moveDivider = (event: React.PointerEvent, kind: 'width' | 'ratio') => {
    const element = event.currentTarget as HTMLElement; element.setPointerCapture(event.pointerId)
    const rect = element.parentElement!.getBoundingClientRect()
    const move = (e: PointerEvent) => { if (kind === 'width') w.sidebarWidth = Math.max(150, Math.min(450, e.clientX - rect.left)); else w.outlineRatio = Math.max(15, Math.min(85, (e.clientY - rect.top) / rect.height * 100)); w.emit() }
    element.addEventListener('pointermove', move)
    element.addEventListener('pointerup', () => element.removeEventListener('pointermove', move), { once: true })
  }
  return <main className="workspace">
    {w.sessionLost && !w.sessionLostMuted && <div role="alert" className="session-banner"><span>{t('sessionLost')}</span><button className="primary" onClick={() => { if (w.onLogin) w.run(w.onLogin()) }}>{t('login')}</button><button onClick={() => { w.sessionLostMuted = true; w.emit() }}>{t('later')}</button></div>}
    <div className="body">
      {w.sidebar && <aside style={{ width: w.sidebarWidth }}>
        <details open className="file-section" style={{ flexBasis: `${w.outlineRatio}%` }}><summary>{t('files')} <button onClick={openLocation} aria-label={t('open')}>＋</button></summary>
          <nav role="listbox" aria-label={t('files')}>{w.items.map(item => { const open = [...w.documents.values()].find(d => d.source && contentRefString(d.source) === contentRefString(item.source)); return <button key={contentRefString(item.source)} role="option" aria-selected={doc === open} onClick={() => item.text ? w.run(w.open({ requestId: crypto.randomUUID(), source: item.source, session: w.session }, true)) : w.onExternalOpen?.(item.source, 'default')} onDoubleClick={() => { if (item.text) w.run(w.open({ requestId: crypto.randomUUID(), source: item.source, session: w.session })) }} title={contentRefString(item.source)}>{open && dirty(open) ? '● ' : ''}{item.title}{!item.text ? ' ↗' : ''}</button> })}</nav>
        </details>
        <div role="separator" aria-orientation="horizontal" tabIndex={0} className="divider horizontal" onPointerDown={e => moveDivider(e, 'ratio')} onKeyDown={e => { if (e.key.startsWith('Arrow')) { w.outlineRatio = Math.max(15, Math.min(85, w.outlineRatio + (e.key === 'ArrowUp' ? -5 : 5))); w.emit() } }} />
        <details open className="outline-section"><summary>{t('outline')}</summary><nav aria-label={t('outline')}>{outline.length ? outline.map(h => <button key={h.from} aria-current={h === currentHeading ? 'location' : undefined} style={{ paddingLeft: 12 + (h.level - 1) * 13 }} onClick={() => { const view = activeView; if (view) { view.dispatch({ selection: { anchor: h.from }, effects: EditorView.scrollIntoView(h.from, { y: 'center' }) }); view.focus() } }}>{h.title}</button>) : <p className="muted">{t('noOutline')}</p>}</nav></details>
      </aside>}
      {w.sidebar && <div role="separator" aria-orientation="vertical" tabIndex={0} className="divider vertical" onPointerDown={e => moveDivider(e, 'width')} onKeyDown={e => { if (e.key.startsWith('Arrow')) { w.sidebarWidth = Math.max(150, Math.min(450, w.sidebarWidth + (e.key === 'ArrowLeft' ? -10 : 10))); w.emit() } }} />}
      <section className={`groups ${w.layout}`}>
        {w.groups.map(group => <section key={group.id} className={`group ${group.id === w.activeGroup ? 'active-group' : ''}`} onFocusCapture={() => { if (w.activeGroup !== group.id) { w.activeGroup = group.id; w.emit() } }}>
          <div className="tabbar"><div className="tabs" role="tablist" aria-label={t('files')} onDragOver={e => e.preventDefault()} onDrop={e => { e.preventDefault(); try { const [id, from] = JSON.parse(e.dataTransfer.getData('text/plain')); w.moveTab(id, from, group.id) } catch {} }}>
            {group.tabs.map(tab => { const d = w.documents.get(tab.doc)!; const duplicate = [...w.documents.values()].some(other => other.id !== d.id && other.name === d.name); return <div className={`tab ${tab.transient ? 'transient' : ''}`} key={tab.doc} draggable onDragStart={e => e.dataTransfer.setData('text/plain', JSON.stringify([tab.doc, group.id]))} onDrop={e => { e.stopPropagation(); e.preventDefault(); try { const [id, from] = JSON.parse(e.dataTransfer.getData('text/plain')); w.moveTab(id, from, group.id, tab.doc) } catch {} }}>
              <button role="tab" aria-selected={group.active === tab.doc} title={d.source ? contentRefString(d.source) : d.name} onClick={() => { w.activeGroup = group.id; group.active = tab.doc; w.emit() }} onDoubleClick={() => { tab.transient = false; w.emit() }} onAuxClick={e => { if (e.button === 1) w.run(w.close(d, group)) }}>{d.readOnly ? '🔒 ' : ''}{d.name}{duplicate && d.source?.kind === 'cyfs-path' ? ` — ${parentSource(d.source)?.kind === 'cyfs-path' ? d.source.path.split('/').at(-2) : ''}` : ''}{dirty(d) ? ' ●' : ''}</button><button className="close-tab" aria-label={`${t('close')} ${d.name}`} onClick={() => w.run(w.close(d, group))}>×</button>
            </div> })}
          </div><button aria-label={t('new')} title={t('new')} onClick={() => { w.activeGroup = group.id; w.newDocument() }}>＋</button><button aria-label={t('menu')} title={t('menu')} onClick={() => { w.activeGroup = group.id; setMenu(!menu) }}>☰</button></div>
          {group.active && w.documents.has(group.active) ? <Editor workspace={w} doc={w.documents.get(group.active)!} group={group} onCursor={refreshCursor} /> : <div className="welcome"><div className="wordmark">Aa<span>_</span></div><h1>{t('title')}</h1><p className="muted">{t('empty')}</p><div className="welcome-actions"><button onClick={() => w.newDocument()}>{t('new')} <kbd>Alt N</kbd></button><button onClick={openLocation}>{t('open')} <kbd>↗</kbd></button><button onClick={() => { w.panel = 'buffers'; w.run(w.refreshRecords()) }}>{t('buffers')} <span>{w.abandoned.length}</span></button><button onClick={() => { w.panel = 'recovery'; w.run(w.refreshRecords()) }}>{t('recovery')}</button></div>{w.recent.length > 0 && <><h2>{t('recent')}</h2>{w.recent.map(source => <button className="recent" key={contentRefString(source)} onClick={() => w.run(w.open({ requestId: crypto.randomUUID(), source }))}>{contentRefString(source)}</button>)}</>}</div>}
        </section>)}
      </section>
    </div>
    {menu && <div className="menu" role="menu">{[
      ['new', () => w.newDocument()], ['open', openLocation], ['save', () => w.run(w.save())], ['saveAs', saveAs], ['close', () => doc && w.run(w.close(doc))],
      ['single', () => w.split('single')], ['columns', () => w.split('columns-2')], ['rows', () => w.split('rows-2')], ['quick', () => { w.panel = 'quick'; w.emit() }],
      ['find', () => activeView && openSearchPanel(activeView)], ['goto', () => activeView && gotoLine(activeView)],
      ['buffers', () => { w.panel = 'buffers'; w.run(w.refreshRecords()) }], ['recovery', () => { w.panel = 'recovery'; w.run(w.refreshRecords()) }], ['settings', () => { w.panel = 'settings'; w.emit() }],
      ['files', () => { w.sidebar = !w.sidebar; w.emit() }], ['reload', () => doc && w.run(w.reload(doc))], ['preview', () => doc?.source && w.onExternalOpen?.(doc.source, 'preview')],
    ].map(([key, fn]) => <button role="menuitem" key={String(key)} onClick={() => action(fn as () => void)}>{t(String(key))}</button>)}</div>}
    <footer><button aria-label={t('files')} onClick={() => { w.sidebar = !w.sidebar; w.emit() }}>▤</button><span>{t('line')} {line?.number ?? 1}, {t('column')} {position - (line?.from ?? 0) + 1}{state && !state.selection.main.empty ? ` · ${state.selection.main.to - state.selection.main.from} ${t('selected')}` : ''}</span>
      {doc && <><select aria-label={t('encoding')} value={doc.encoding} onChange={e => { const encoding = e.target.value; if (['utf-8', 'utf-16le', 'utf-16be'].includes(encoding)) w.convertEncoding(doc, encoding); else w.run(w.reload(doc, encoding)) }}>{['utf-8', 'utf-16le', 'utf-16be', 'gb18030', 'big5', 'shift_jis', 'euc-kr', 'windows-1252'].map(c => <option key={c}>{c}</option>)}</select>
      <select aria-label="EOL" disabled={doc.readOnly || doc.mutating} value={doc.eol} onChange={e => { doc.eol = e.target.value as '\n'; w.changed(doc) }}><option value={'\n'}>LF</option><option value={'\r\n'}>CRLF</option><option value={'\r'}>CR</option></select>
      <select aria-label={t('language')} value={doc.language ?? ''} onChange={e => { doc.language = e.target.value || undefined; w.emit() }}><option value="">{t('auto')}</option><option value="plain">{t('plain')}</option>{languages.map(l => <option key={l.name}>{l.name}</option>)}</select>
      {doc.readOnly && doc.writable && <button onClick={() => w.convertEncoding(doc, 'utf-8')}>{t('utf8')}</button>}<span>{w.settings.insertSpaces ? '␠' : '⇥'}: {w.settings.tabSize}</span>{doc.largeFile && <span>{t('large')}</span>}{doc.mixedEol && <span title={t('mixed')}>⚠ {t('mixed')}</span>}
      <button className="disk-state" onClick={() => doc.readOnly ? saveAs() : w.run(w.save(doc))}>{t(doc.disk !== 'in-sync' ? doc.disk : doc.readOnly ? 'readonly' : dirty(doc) ? doc.syncedVersion === doc.version ? 'synced' : 'unsynced' : 'in-sync')}</button></>}
    </footer>
    {w.message && <div role="status" aria-live="polite" className="notice"><span>{t(w.message)}</span>{w.failedSource && ['binary', 'too-large'].includes(w.message) && <button onClick={() => w.onExternalOpen?.(w.failedSource!, 'preview')}>{t('preview')}</button>}<button aria-label={t('cancel')} onClick={() => w.notify('')}>×</button></div>}
    {w.panel && <Panel workspace={w} />}
    {w.choice && <div className="overlay"><div role="alertdialog" aria-modal="true" aria-label={t(w.choice.title)} className="dialog"><h2>{t(w.choice.title)}</h2><div className="choices">{w.choice.options.map(option => <button key={option} autoFocus={option === w.choice!.options[0]} onClick={() => w.choice?.resolve(option)}>{t(option)}</button>)}</div></div></div>}
  </main>
}
function Panel({ workspace: w }: { workspace: Workspace }) {
  const t = (key: string) => translate(w.locale, key)
  const [query, setQuery] = useState('')
  const [path, setPath] = useState(() => { const parent = w.active?.source ? parentSource(w.active.source) : undefined; return parent?.kind === 'cyfs-path' ? parent.path : w.buffers.root.split('/.local')[0] })
  const [name, setName] = useState(w.active?.name ?? '')
  const [entries, setEntries] = useState<Entry[]>([])
  useEffect(() => { if (w.panel === 'location') w.run(w.store.list(path).then(setEntries)) }, [path, w.panel, w])
  const close = () => { w.panel = undefined; w.emit() }
  const fuzzy = (value: string) => { let i = 0; const q = query.toLowerCase(); for (const c of value.toLowerCase()) if (c === q[i]) i++; return i === q.length }
  return <div className="overlay" onMouseDown={e => { if (e.target === e.currentTarget) close() }}><section className="dialog panel" role="dialog" aria-modal="true" aria-label={t(w.panel!)}><header><h2>{t(w.panel!)}</h2><button onClick={close} aria-label={t('cancel')}>×</button></header>
    {w.panel === 'quick' && <><input autoFocus placeholder={t('quick')} value={query} onChange={e => setQuery(e.target.value)} />{w.items.filter(i => fuzzy(i.title)).map(i => <button className="record" key={contentRefString(i.source)} onClick={() => { w.run(w.open({ requestId: crypto.randomUUID(), source: i.source, session: w.session })); close() }}>{i.title}</button>)}</>}
    {w.panel === 'location' && <><label>{t('path')}<input value={path} onChange={e => setPath(e.target.value)} /></label><button onClick={() => { const parent = path.slice(0, path.lastIndexOf('/')); if (parent.length >= 7) setPath(parent) }}>{t('parent')}</button><div className="directory">{entries.map(e => <button key={e.name} onClick={() => e.target.kind === 'dir' ? setPath(`${path}/${e.name}`) : setName(e.name)}>{e.target.kind === 'dir' ? '▸ ' : ''}{e.name}</button>)}</div><label>{t('filename')}<input autoFocus value={name} onChange={e => setName(e.target.value)} /></label><button className="primary" disabled={!name || /[\/\\]/.test(name) || name === '.' || name === '..'} onClick={() => { const action = w.locationAction; close(); if (action) w.run(action(`${path.replace(/\/$/, '')}/${name}`)) }}>{t('choose')}</button></>}
    {w.panel === 'settings' && <><div className="settings-grid">{(['fontSize', 'tabSize'] as const).map(key => <label key={key}>{t(key)}<input type="number" min={key === 'fontSize' ? 8 : 1} max={key === 'fontSize' ? 40 : 8} value={w.settings[key]} onChange={e => w.run(w.updateSettings({ ...w.settings, [key]: Math.max(key === 'fontSize' ? 8 : 1, Math.min(key === 'fontSize' ? 40 : 8, Number(e.target.value))) }))} /></label>)}<label>{t('wordWrap')}<select value={w.settings.wordWrap} onChange={e => w.run(w.updateSettings({ ...w.settings, wordWrap: e.target.value as 'auto' }))}>{['auto', 'on', 'off'].map(v => <option key={v} value={v}>{t(v)}</option>)}</select></label>{(['insertSpaces', 'lineNumbers', 'renderWhitespace', 'autoReloadClean', 'restoreSession', 'closeKeepsChanges', 'trimTrailingWhitespaceOnSave', 'ensureFinalNewline'] as const).map(key => <label key={key}><input type="checkbox" checked={w.settings[key]} onChange={e => w.run(w.updateSettings({ ...w.settings, [key]: e.target.checked }))} />{t(key === 'renderWhitespace' ? 'whitespace' : key)}</label>)}{(['discarded', 'takenOver'] as const).map(key => <label key={key}>{t(`${key}Days`)}<input type="number" min="0" value={w.settings.recoveryRetentionDays[key]} onChange={e => w.run(w.updateSettings({ ...w.settings, recoveryRetentionDays: { ...w.settings.recoveryRetentionDays, [key]: Math.max(0, Number(e.target.value)) } }))} /></label>)}</div></>}
    {w.panel === 'buffers' && <>{!w.abandoned.length && <p>{t('emptyBuffers')}</p>}{w.abandoned.map(b => <div className="record" key={b.path}><strong>{b.header.displayName}</strong><small>{b.header.source ? contentRefString(b.header.source) : ''}<br />{b.header.origin.device} · {new Date(b.header.updatedAt).toLocaleString(w.locale)}</small><div><button disabled={b.active} title={b.active ? t('buffer-in-use') : undefined} onClick={() => w.run(w.restoreBuffer(b))}>{t('restore')}</button><button onClick={() => { if (b.header.source) w.run(w.open({ requestId: crypto.randomUUID(), source: b.header.source }, false, false)); close() }}>{t('disk')}</button><button disabled={b.active} onClick={() => w.run(w.discardBuffer(b))}>{t('deleteBuffer')}</button></div></div>)}</>}
    {w.panel === 'recovery' && <><p className="muted">{(w.recoveries.reduce((n, r) => n + r.size, 0) / 1024).toFixed(1)} KiB</p><button onClick={() => w.run((async () => { if (await w.ask('confirmDelete', ['clear', 'cancel']) === 'clear') { for (const r of w.recoveries) await w.store.remove(r.path, true); await w.refreshRecords() } })())}>{t('clear')}</button>{!w.recoveries.length && <p>{t('emptyRecovery')}</p>}{w.recoveries.map(r => <div className="record" key={r.path}><strong>{r.entry.displayName}</strong><small>{r.entry.source ? contentRefString(r.entry.source) : ''}<br />{r.entry.reason} · {new Date(r.entry.createdAt).toLocaleString(w.locale)}</small><div><button onClick={() => w.run(w.openRecovery(r))}>{t('open')}</button><button onClick={() => w.run(w.openRecovery(r, true))}>{t('restore')}</button><button onClick={() => w.run((async () => { await w.openRecovery(r); w.chooseLocation(path => w.save(w.active, { path }).then(() => {})) })())}>{t('saveAs')}</button><button onClick={() => w.run((async () => { if (await w.ask('confirmDelete', ['remove', 'cancel']) === 'remove') { await w.store.remove(r.path, true); await w.refreshRecords() } })())}>{t('remove')}</button></div></div>)}</>}
  </section></div>
}
export function handleBack(w: Workspace): boolean {
  if (activeView && closeSearchPanel(activeView)) return true
  if (w.panel) { w.panel = undefined; w.emit(); return true }
  if (w.sidebar && matchMedia('(max-width: 719px)').matches) { w.sidebar = false; w.emit(); return true }
  return false
}
