/* 自定义工具栏… (UI 框架 PRD §4.6): the shared layers of the toolbar. First the scope — this canvas (`structure` on it)
 * or this Workspace (`manage`); a scope the person may not change is disabled with the reason — then the zones:
 * the title zone and the status bar as a whole, the collaboration zone and the tool selection zone's objects item by
 * item: tick to show, drag to reorder (move up / down on a touch screen), add an action or a macro (with a name and an
 * icon of its own) or an object type, lock a zone against personal changes, restore the defaults, or hide and lock
 * everything (App mode). Saving is one undoable Commit of the chosen layer; it travels with templates, Clones and
 * exports. Hiding is not a permission: what people may do is still checked by the service. Not offered on a phone. */

import { createElement, useMemo, useState, type DragEvent } from 'react'
import { ArrowDown, ArrowUp, GripVertical, Plus, Trash2 } from 'lucide-react'
import { describeError } from '../../api/session'
import type { Json } from '../../api/types'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { useSurfaceToolbar, useWorkspaceSettings } from '../../state/settings'
import { randomId } from '../../api/ids'
import { useAiwsText } from '../../useAiwsText'
import { entryId, ENTRY_ICONS, registryEntries } from '../canvas/catalog'
import { actionRegistry, type ZoneId } from '../framework/actions'
import { builtinToolbar, CUSTOMIZABLE_ZONES, INSERT_ACTION, parseToolbarConfig, type ConfigLayer } from '../framework/config'
import { useActionContext } from '../framework/hooks'
import { ACTION_ICONS, iconOf } from '../framework/icons'
import { useMacroWatch } from '../framework/macros'
import { holdsMacro } from '../wish/macro'
import { Modal } from '../shell/dialogs'
import { useShell } from '../shell/shellContext'
import { appModeDraft, defaultDraft, draftOf, isAppModeDraft, layerOf, moveItem, placeBefore, type Draft, type DraftItem, type DraftZone } from './model'
import './customize.css'

type Scope = 'surface' | 'workspace'

const ZONE_LABEL: Record<ZoneId, string> = {
  title: 'aiworkspace.customize.zone.title', collab: 'aiworkspace.framework.zone.collab', tools: 'aiworkspace.framework.zone.tools',
  context: 'aiworkspace.framework.zone.context', status: 'aiworkspace.customize.zone.status',
}
const ITEM_ZONES: ZoneId[] = ['collab', 'tools']

/** A string argument of an item (`entry` of an object, `wish` of a macro). */
function argOf(item: DraftItem, name: string): string | null {
  const args = item.args
  const value = args && typeof args === 'object' && !Array.isArray(args) ? args[name] : undefined
  return typeof value === 'string' ? value : null
}

const coarse = () => typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(pointer: coarse)').matches

export function CustomizeDialog({ onClose }: { onClose: () => void }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  useOutlineVersion()
  const ctx = useActionContext(null)
  const surfaceId = shell.activeSurface?.entity_id ?? null
  const settings = useWorkspaceSettings()
  const surface = useSurfaceToolbar(surfaceId)
  const reasons: Record<Scope, string | null> = { surface: surface.writeReason, workspace: settings.writeReason }
  const [scope, setScope] = useState<Scope>(reasons.surface === null || reasons.workspace !== null ? 'surface' : 'workspace')
  const [drafts, setDrafts] = useState<Partial<Record<Scope, Draft>>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [touch] = useState(coarse)

  const loaded = !settings.loading && (scope === 'workspace' || !surface.loading)
  const below: ConfigLayer[] = useMemo(() => [
    { layer: 'builtin', config: builtinToolbar() },
    ...(scope === 'surface' ? [{ layer: 'workspace' as const, config: parseToolbarConfig(settings.value.toolbar) }] : []),
  ], [scope, settings.value.toolbar])
  const layer: ConfigLayer = { layer: scope, config: parseToolbarConfig(scope === 'surface' ? surface.value : settings.value.toolbar) }
  const draft = drafts[scope] ?? (loaded ? draftOf(below, layer) : null)
  const reason = reasons[scope]
  const readOnly = reason !== null || busy
  const setDraft = (next: Draft) => setDrafts((all) => ({ ...all, [scope]: next }))
  const setZone = (zone: ZoneId, next: DraftZone) => { if (draft) setDraft({ ...draft, [zone]: next }) }

  const entries = useMemo(() => registryEntries(), [])
  const itemLabel = (item: DraftItem): string => {
    if (item.title) return item.title
    if (item.action === INSERT_ACTION) {
      const id = argOf(item, 'entry')
      return entries.find((e) => entryId(e) === id)?.title ?? t('aiworkspace.customize.unknown', { id: item.key })
    }
    const def = actionRegistry.get(item.action)
    if (!def) return t('aiworkspace.customize.unknown', { id: item.action })
    const wishId = item.action === 'collab.macro' ? argOf(item, 'wish') : null
    if (wishId) {
      const wish = store.outline.get(wishId)
      return t('aiworkspace.framework.macro.one', { name: wish?.title ?? wish?.name ?? wishId })
    }
    try { return def.label(ctx) } catch { return item.action }
  }
  const itemIcon = (item: DraftItem) => {
    if (item.icon) return iconOf(item.icon)
    const entry = item.action === INSERT_ACTION ? argOf(item, 'entry') : null
    if (entry) return ENTRY_ICONS[entry] ?? null
    return iconOf(actionRegistry.get(item.action)?.icon)
  }

  const save = async () => {
    if (!draft || reason) return
    setBusy(true)
    setError(null)
    try {
      const config = layerOf(below, draft) as Json | null
      const outcome = scope === 'surface' ? await surface.set(config) : await settings.update({ toolbar: config })
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') { onClose(); return }
      setError(t('aiworkspace.customize.refused', { code: 'code' in outcome ? String(outcome.code) : outcome.status }))
    } catch (failure) {
      setError(t('aiworkspace.customize.refused', { code: describeError(failure) }))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal label={t('aiworkspace.customize.title')} testId="aiws-customize-dialog" className="aiws-customize" onClose={onClose}>
      <div className="aiws-dialog-head"><b>{t('aiworkspace.customize.title')}</b></div>
      <fieldset className="aiws-customize-scope" data-testid="aiws-customize-scope">
        <legend>{t('aiworkspace.customize.scope')}</legend>
        {(['surface', 'workspace'] as Scope[]).map((s) => (
          <label key={s} className="aiws-customize-scope-option" title={reasons[s] ?? undefined}>
            <input type="radio" name="aiws-customize-scope" value={s} checked={scope === s} data-testid={`aiws-customize-scope-${s}`} disabled={reasons[s] !== null || busy} onChange={() => { setScope(s); setError(null) }} />
            <span>{s === 'surface' ? t('aiworkspace.customize.scope.surface', { name: shell.activeSurface?.title ?? shell.activeSurface?.name ?? '' }) : t('aiworkspace.customize.scope.workspace')}</span>
            {reasons[s] && <small className="aiws-muted" data-testid={`aiws-customize-scope-${s}-reason`}>{reasons[s]}</small>}
          </label>
        ))}
      </fieldset>
      <p className="aiws-muted aiws-customize-note">{t(scope === 'surface' ? 'aiworkspace.customize.note.surface' : 'aiworkspace.customize.note.workspace')}</p>
      {!draft ? <div className="aiws-muted" role="status">{t('aiworkspace.customize.loading')}</div> : (
        <div className="aiws-customize-zones" data-testid="aiws-customize-zones" data-readonly={readOnly ? 'true' : undefined}>
          {CUSTOMIZABLE_ZONES.map((zone) => (
            <ZoneSection key={zone} zone={zone} value={draft[zone]} readOnly={readOnly} touch={touch} label={t(ZONE_LABEL[zone])} itemLabel={itemLabel} itemIcon={itemIcon}
              onChange={(next) => setZone(zone, next)} baseLocked={defaultDraft(below)[zone].locked} />
          ))}
        </div>
      )}
      {error && <div className="aiws-error" role="alert" data-testid="aiws-customize-error">{error}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" data-testid="aiws-customize-app-mode" disabled={!draft || readOnly} aria-pressed={draft ? isAppModeDraft(draft) : false}
          title={t('aiworkspace.customize.appModeHint')} onClick={() => { if (draft) setDraft(appModeDraft(draft)) }}>{t('aiworkspace.customize.appMode')}</button>
        <button type="button" data-testid="aiws-customize-reset" disabled={!draft || readOnly} onClick={() => setDraft(defaultDraft(below))}>{t('aiworkspace.customize.reset')}</button>
        <span className="aiws-grow" />
        <button type="button" onClick={onClose} data-testid="aiws-customize-cancel">{t('aiworkspace.common.cancel')}</button>
        <button type="button" className="is-primary" data-testid="aiws-customize-save" disabled={!draft || readOnly} title={reason ?? undefined} onClick={() => { void save() }}>{t('aiworkspace.common.save')}</button>
      </div>
    </Modal>
  )
}

function ZoneSection({ zone, value, readOnly, touch, label, itemLabel, itemIcon, onChange, baseLocked }: {
  zone: ZoneId; value: DraftZone; readOnly: boolean; touch: boolean; label: string; baseLocked: boolean
  itemLabel: (item: DraftItem) => string; itemIcon: (item: DraftItem) => ReturnType<typeof iconOf>
  onChange: (next: DraftZone) => void
}) {
  const { t } = useAiwsText()
  const [dragging, setDragging] = useState<string | null>(null)
  const items = ITEM_ZONES.includes(zone)
  const setItem = (key: string, patch: Partial<DraftItem>) => onChange({ ...value, items: value.items.map((item) => (item.key === key ? { ...item, ...patch } : item)) })
  const drop = (event: DragEvent, beforeKey: string | null) => {
    event.preventDefault()
    if (dragging) onChange(placeBefore(value, dragging, beforeKey))
    setDragging(null)
  }
  return (
    <section className="aiws-customize-zone" data-testid={`aiws-customize-zone-${zone}`} data-hidden={value.hidden ? 'true' : undefined} data-locked={value.locked ? 'true' : undefined}>
      <div className="aiws-customize-zone-head">
        <b>{label}</b>
        <span className="aiws-grow" />
        <label className="aiws-customize-check"><input type="checkbox" data-testid={`aiws-customize-zone-${zone}-shown`} checked={!value.hidden} disabled={readOnly} onChange={(event) => onChange({ ...value, hidden: !event.target.checked })} /> {t('aiworkspace.customize.shown')}</label>
        <label className="aiws-customize-check" title={baseLocked ? t('aiworkspace.customize.lockedBelow') : t('aiworkspace.customize.lockHint')}>
          <input type="checkbox" data-testid={`aiws-customize-zone-${zone}-locked`} checked={value.locked} disabled={readOnly || baseLocked} onChange={(event) => onChange({ ...value, locked: event.target.checked })} /> {t('aiworkspace.customize.locked')}
        </label>
      </div>
      {items && (
        <ol className="aiws-customize-items" onDragOver={(event) => { if (dragging) event.preventDefault() }} onDrop={(event) => drop(event, null)}>
          {value.items.map((item, index) => {
            const icon = itemIcon(item)
            return (
              <li key={item.key} className={`aiws-customize-item${dragging === item.key ? ' is-dragging' : ''}`} data-testid="aiws-customize-item" data-key={item.key} data-hidden={item.hidden ? 'true' : undefined}
                draggable={!readOnly && !touch} onDragStart={(event) => { setDragging(item.key); event.dataTransfer.effectAllowed = 'move'; event.dataTransfer.setData('text/plain', item.key) }}
                onDragEnd={() => setDragging(null)} onDragOver={(event) => { if (dragging) event.preventDefault() }} onDrop={(event) => { event.stopPropagation(); drop(event, item.key) }}>
                {!touch && <GripVertical size={14} className="aiws-customize-grip" aria-hidden="true" />}
                <label className="aiws-customize-item-label">
                  <input type="checkbox" data-testid="aiws-customize-item-shown" checked={!item.hidden} disabled={readOnly} onChange={(event) => setItem(item.key, { hidden: !event.target.checked })} />
                  {icon ? createElement(icon, { size: 16, 'aria-hidden': true }) : <span className="aiws-customize-noicon" aria-hidden="true" />}
                  <span className="aiws-user-text">{itemLabel(item)}</span>
                </label>
                {(touch || !readOnly) && (
                  <span className="aiws-customize-item-tools">
                    {touch && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.customize.up')} title={t('aiworkspace.customize.up')} data-testid="aiws-customize-item-up" disabled={readOnly || index === 0} onClick={() => onChange(moveItem(value, item.key, -1))}><ArrowUp size={16} /></button>}
                    {touch && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.customize.down')} title={t('aiworkspace.customize.down')} data-testid="aiws-customize-item-down" disabled={readOnly || index === value.items.length - 1} onClick={() => onChange(moveItem(value, item.key, 1))}><ArrowDown size={16} /></button>}
                    {item.added && <button type="button" className="aiws-tool" aria-label={t('aiworkspace.customize.remove')} title={t('aiworkspace.customize.remove')} data-testid="aiws-customize-item-remove" disabled={readOnly} onClick={() => onChange({ ...value, items: value.items.filter((entry) => entry.key !== item.key) })}><Trash2 size={16} /></button>}
                  </span>
                )}
              </li>
            )
          })}
        </ol>
      )}
      {items && !readOnly && (zone === 'tools' ? <AddObject value={value} onChange={onChange} /> : <AddAction value={value} onChange={onChange} />)}
    </section>
  )
}

/** Another object type for the tool selection zone. */
function AddObject({ value, onChange }: { value: DraftZone; onChange: (next: DraftZone) => void }) {
  const { t } = useAiwsText()
  const [chosen, setChosen] = useState('')
  const present = new Set(value.items.map((item) => item.key))
  const options = registryEntries().filter((entry) => !entry.variant && !present.has(`insert:${entryId(entry)}`))
  if (options.length === 0) return null
  return (
    <div className="aiws-customize-add" data-testid="aiws-customize-add-object">
      <select aria-label={t('aiworkspace.customize.addObject')} data-testid="aiws-customize-add-object-type" value={chosen} onChange={(event) => setChosen(event.target.value)}>
        <option value="">{t('aiworkspace.customize.addObject')}</option>
        {options.map((entry) => <option key={entryId(entry)} value={entryId(entry)}>{entry.title}</option>)}
      </select>
      <button type="button" data-testid="aiws-customize-add-object-confirm" disabled={!chosen} onClick={() => {
        onChange({ ...value, items: [...value.items, { key: `insert:${chosen}`, action: INSERT_ACTION, args: { entry: chosen }, hidden: false, added: true }] })
        setChosen('')
      }}><Plus size={14} aria-hidden="true" /> {t('aiworkspace.customize.add')}</button>
    </div>
  )
}

/** An action, or a macro, for the collaboration zone — with a name and an icon of its own if wanted. The macros are the
 * wishes holding one (Q27: not the Mock executor's), whether or not they can run at this moment. */
function AddAction({ value, onChange }: { value: DraftZone; onChange: (next: DraftZone) => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const ctx = useActionContext(null)
  const [chosen, setChosen] = useState('')
  const [title, setTitle] = useState('')
  const [icon, setIcon] = useState('')
  useMacroWatch()
  const wishes = store.outline.all().filter((e) => holdsMacro(e, store.freshness.get(e.entity_id)))
  const actions = actionRegistry.list().filter((def) => (def.run || def.candidates) && !def.zone?.control && def.id !== 'collab.macro')
  const label = (id: string) => { try { return actionRegistry.get(id)?.label(ctx) ?? id } catch { return id } }
  const add = () => {
    if (!chosen) return
    const macro = chosen.startsWith('macro:') ? chosen.slice('macro:'.length) : null
    const wish = macro ? store.outline.get(macro) : undefined
    const item: DraftItem = {
      key: `custom-${randomId('item')}`, action: macro ? 'collab.macro' : chosen, ...(macro ? { args: { wish: macro } } : {}),
      title: title.trim() || (wish ? wish.title ?? wish.name ?? undefined : undefined), icon: icon || (macro ? 'macro' : undefined), hidden: false, added: true,
    }
    onChange({ ...value, items: [...value.items, item] })
    setChosen(''); setTitle(''); setIcon('')
  }
  return (
    <div className="aiws-customize-add" data-testid="aiws-customize-add-action">
      <select aria-label={t('aiworkspace.customize.addAction')} data-testid="aiws-customize-add-action-pick" value={chosen} onChange={(event) => setChosen(event.target.value)}>
        <option value="">{t('aiworkspace.customize.addAction')}</option>
        {wishes.length > 0 && (
          <optgroup label={t('aiworkspace.customize.macros')}>
            {wishes.map((w) => <option key={w.entity_id} value={`macro:${w.entity_id}`}>{w.title ?? w.name ?? w.entity_id}</option>)}
          </optgroup>
        )}
        <optgroup label={t('aiworkspace.customize.actions')}>
          {actions.map((def) => <option key={def.id} value={def.id}>{label(def.id)}</option>)}
        </optgroup>
      </select>
      <input aria-label={t('aiworkspace.customize.itemName')} placeholder={t('aiworkspace.customize.itemName')} data-testid="aiws-customize-add-action-title" value={title} maxLength={40} onChange={(event) => setTitle(event.target.value)} />
      <select aria-label={t('aiworkspace.customize.itemIcon')} data-testid="aiws-customize-add-action-icon" value={icon} onChange={(event) => setIcon(event.target.value)}>
        <option value="">{t('aiworkspace.customize.itemIcon')}</option>
        {Object.keys(ACTION_ICONS).map((name) => <option key={name} value={name}>{name}</option>)}
      </select>
      <button type="button" data-testid="aiws-customize-add-action-confirm" disabled={!chosen} onClick={add}><Plus size={14} aria-hidden="true" /> {t('aiworkspace.customize.add')}</button>
    </div>
  )
}
