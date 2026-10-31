/* “定义为控件…” (扩展控件 P1): the selected Blocks become a composite control. The dialog proposes one slot per Block
 * (named after what it is, accepting it: `cell:<ident>` for an instance of a control, its view type, or `container`),
 * a `fixed` layout (each keeps its place; `stack` lines them up), writes component.json + composition.json + AGENT.md
 * into the user's home folder, builds it, and — when asked — replaces the selection with an instance in one Commit:
 * the container, each Block copied into its slot with `tree.clone` (its own data with it), the originals deleted. */

import { useMemo, useState } from 'react'
import { LayoutPanelTop } from 'lucide-react'
import { DEFAULT_PADDING } from '@controls/composite'
import { randomId } from '../../api/ids'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Json, Operation } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import type { Laid } from '../canvas/layout'
import { Modal } from '../shell/dialogs'
import { buildFromHome, createSource, homeControlDir, sourceStateKey, type BuildOutcome } from './controlSources'

const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/
const SLOT = /^[a-z][a-z0-9_]{0,31}$/

interface SlotRow { id: string; slot: string; accepts: string; title: string }

/** What a Block is as a slot: its name and what the slot accepts. */
function slotOf(e: EntityEnvelope, identOf: (defId: string) => string | null | undefined): { base: string; accepts: string } {
  if (e.type_id === 'buckyos.container') return { base: e.kind === 'group' ? 'group' : (e.kind ?? 'container'), accepts: 'container' }
  const ident = e.def_id ? identOf(e.def_id) : null
  if (ident) return { base: ident.slice(ident.indexOf('#') + 1), accepts: `cell:${ident}` }
  return { base: e.view_type ?? 'item', accepts: e.view_type ?? 'richtext' }
}

function slotName(base: string, taken: Set<string>): string {
  const clean = base.toLowerCase().replace(/[^a-z0-9_]+/g, '_').replace(/^[^a-z]+/, '').slice(0, 28) || 'item'
  let name = clean
  for (let n = 2; taken.has(name); n++) name = `${clean}_${n}`
  taken.add(name)
  return name
}

export function DefineControlDialog({ ids, laid, surfaceId, onDone, onClose }: { ids: string[]; laid: Map<string, Laid>; surfaceId: string; onDone: (selected: string[]) => void; onClose: () => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const principal = store.session.principal ?? 'me'
  const identOf = (defId: string) => store.outline.get(defId)?.ident ?? null
  const blocks = ids.map((id) => laid.get(id)).filter((l): l is Laid => Boolean(l))
  const initial = useMemo<SlotRow[]>(() => {
    const taken = new Set<string>()
    return blocks.map((l) => { const s = slotOf(l.entity, identOf); return { id: l.entity.entity_id, slot: slotName(s.base, taken), accepts: s.accepts, title: l.entity.title ?? '' } })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the selection when the dialog opened
  }, [])
  const [rows, setRows] = useState<SlotRow[]>(initial)
  const [name, setName] = useState('')
  const [title, setTitle] = useState('')
  const [author, setAuthor] = useState(`did:bns:${principal}`)
  const [algorithm, setAlgorithm] = useState<'fixed' | 'stack'>('fixed')
  const [replace, setReplace] = useState(true)
  const [busy, setBusy] = useState(false)
  const [outcome, setOutcome] = useState<BuildOutcome | null>(null)
  const [error, setError] = useState<string | null>(null)
  const dir = homeControlDir(principal, store.session.info().title, name || '…')
  const slotsValid = rows.every((r) => SLOT.test(r.slot)) && new Set(rows.map((r) => r.slot)).size === rows.length
  const ready = NAME.test(name) && author.startsWith('did:') && slotsValid && blocks.length > 0
  const create = async () => {
    setBusy(true)
    setError(null)
    try {
      const deps = [...new Set(rows.map((r) => r.accepts).filter((a) => a.startsWith('cell:')).map((a) => a.slice(5)))]
      const slots = Object.fromEntries(rows.map((r) => [r.slot, { accepts: [r.accepts], max: 1, placement: algorithm === 'fixed' ? 'free' as const : 'stack' as const, ...(r.title ? { title: r.title } : {}) }]))
      await createSource(dir, { authorDid: author, name, title: title || name, kind: 'composite', slots, deps, algorithm })
      const built = await buildFromHome(store, dir)
      setOutcome(built)
      if (!built.ok || !built.result.def_entity_id) return
      const defId = built.result.def_entity_id
      store.userState.set(sourceStateKey(defId), dir)
      // where the instance goes: the selection's box (its top-left, padding inside); else beside it
      const xs = blocks.map((l) => l.rect)
      const box = { x: Math.min(...xs.map((r) => r.x)), y: Math.min(...xs.map((r) => r.y)), r: Math.max(...xs.map((r) => r.x + r.w)), b: Math.max(...xs.map((r) => r.y + r.h)) }
      const containerId = randomId('b')
      const key = store.core.order_key_between(store.outline.childrenOf(surfaceId).at(-1)?.order_key ?? undefined, undefined)
      const origin = replace ? { x: box.x - DEFAULT_PADDING, y: box.y - DEFAULT_PADDING - 28 } : { x: box.r + 40, y: box.y }
      const ops: Operation[] = [{ op: 'entity.create', entity_id: containerId, type_id: 'buckyos.container', parent_id: surfaceId, order_key: key,
        placement: { x: Math.round(origin.x), y: Math.round(origin.y), w: 'auto', h: 'auto' }, payload: { kind: 'composite', def_ref: { entity_id: defId }, ...(title ? { title } : {}) } }]
      if (replace) {
        const seed = randomId('s')
        let childKey: string | undefined
        blocks.forEach((l, i) => {
          childKey = store.core.order_key_between(childKey, undefined)
          const r = l.rect
          const placement = algorithm === 'fixed'
            ? { x: Math.round(r.x - origin.x), y: Math.round(r.y - origin.y), w: Math.round(r.w), h: Math.round(r.h) }
            : { x: 'auto', y: 'auto', w: Math.round(r.w), h: Math.round(r.h) }
          ops.push({ op: 'tree.clone', source_id: l.entity.entity_id, parent_id: containerId, order_key: childKey, placement: placement as Json, slot: rows[i].slot, id_seed: seed })
        })
        for (const l of blocks) {
          const kids = store.outline.descendants(l.entity.entity_id).map((d) => d.entity_id)
          ops.push({ op: 'entity.delete', entity_id: l.entity.entity_id, expect: { rev: l.entity.life_rev }, ...(kids.length > 0 ? { subtree: { delete: kids } } : {}) })
        }
      }
      const done = await store.submit({ editId: `define-control:${containerId}`, label: t('aiworkspace.ext.define.label', { name: title || name }), operations: ops })
      if (done.status !== 'accepted' && done.status !== 'saved_locally') { setError(t('aiworkspace.ext.define.notPlaced')); return }
      onDone([containerId])
    } catch (e) {
      setError(describeError(e))
    } finally {
      setBusy(false)
    }
  }
  const failed = outcome && !outcome.ok ? outcome : null
  return (
    <Modal label={t('aiworkspace.ext.define.title')} testId="aiws-define-dialog" className="aiws-ext-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><LayoutPanelTop size={18} aria-hidden="true" /><b>{t('aiworkspace.ext.define.title')}</b><span className="aiws-grow" />
        <button type="button" className="aiws-link" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <div className="aiws-ext-form">
        <p className="aiws-muted">{t('aiworkspace.ext.define.explain', { count: blocks.length })}</p>
        <label className="aiws-field">{t('aiworkspace.ext.name')}<input value={name} data-testid="aiws-define-name" placeholder="task-board" onChange={(e) => setName(e.target.value.trim())} /></label>
        {name && !NAME.test(name) && <div className="aiws-warning">{t('aiworkspace.ext.nameRule')}</div>}
        <label className="aiws-field">{t('aiworkspace.ext.displayTitle')}<input value={title} data-testid="aiws-define-title" onChange={(e) => setTitle(e.target.value)} /></label>
        <label className="aiws-field">{t('aiworkspace.ext.author')}<input value={author} data-testid="aiws-define-author" onChange={(e) => setAuthor(e.target.value.trim())} /></label>
        <label className="aiws-field">{t('aiworkspace.ext.define.layout')}
          <select value={algorithm} data-testid="aiws-define-layout" onChange={(e) => setAlgorithm(e.target.value === 'stack' ? 'stack' : 'fixed')}>
            <option value="fixed">{t('aiworkspace.ext.define.layoutFixed')}</option><option value="stack">{t('aiworkspace.ext.define.layoutStack')}</option>
          </select>
        </label>
        <table className="aiws-define-slots">
          <thead><tr><th>{t('aiworkspace.ext.define.block')}</th><th>{t('aiworkspace.ext.define.slot')}</th><th>{t('aiworkspace.ext.define.accepts')}</th></tr></thead>
          <tbody>{rows.map((r, i) => (
            <tr key={r.id}>
              <td>{store.outline.get(r.id)?.title || r.id}</td>
              <td><input value={r.slot} data-testid={`aiws-define-slot-${i}`} onChange={(e) => setRows(rows.map((x, j) => (j === i ? { ...x, slot: e.target.value.trim() } : x)))} /></td>
              <td><code>{r.accepts}</code></td>
            </tr>
          ))}</tbody>
        </table>
        {!slotsValid && <div className="aiws-warning">{t('aiworkspace.ext.define.slotRule')}</div>}
        <label className="aiws-inline-form"><input type="checkbox" checked={replace} data-testid="aiws-define-replace" onChange={(e) => setReplace(e.target.checked)} />{t('aiworkspace.ext.define.replace')}</label>
        <div className="aiws-muted">{t('aiworkspace.ext.newFolder', { path: dir })}</div>
        {failed && <div className="aiws-error" role="alert" data-testid="aiws-define-failed">{failed.error}{failed.diagnostics.map((d, i) => <div key={i}><code>{d.file}</code> {d.message}</div>)}</div>}
        {error && <div className="aiws-error" role="alert">{error}</div>}
        <div className="aiws-dialog-actions">
          <button type="button" className="is-primary" data-testid="aiws-define-create" disabled={busy || !ready} onClick={() => void create()}>{busy ? t('aiworkspace.ext.building') : t('aiworkspace.ext.define.create')}</button>
        </div>
      </div>
    </Modal>
  )
}
