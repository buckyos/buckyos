/* eslint-disable react-refresh/only-export-components -- property helpers and their controls */
/* A type's properties (UI 框架 PRD §10.1, §10.2, §10.4): one declaration (`BlockDefinition.configFields`) read by the
 * properties panel (every property, in its section) and the Block toolbar (the ones marked `quick`, as a colour, a menu, a
 * toggle, a size — or several together behind one button, `quickGroup`). Both read the value with `valueOf` and write it
 * with `writeField`, so the toolbar and the panel always show the same value (§15 scenario 34). */

import { useState, type ReactNode } from 'react'
import { SlidersHorizontal } from 'lucide-react'
import { aiwsT } from '../../i18n'
import type { Json } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { setCellConfig } from '../blocks/ops'
import type { BlockDefinition, ConfigField, RenderContext, ToolbarItem } from '../blocks/registry'
import { PaletteGrid } from '../canvas/tools'

/** The tool id of a property (`aiws-near-<id>`). */
export const fieldId = (definition: BlockDefinition, field: ConfigField) => field.id ?? `${definition.type}-${field.key.replace(/_/g, '-')}`

/** The properties that apply to the Block as it is now. */
export function fieldsOf(definition: BlockDefinition | undefined, context: RenderContext | null): ConfigField[] {
  if (!definition?.configFields || !context) return []
  return definition.configFields.filter((field) => { try { return field.when ? field.when(context) : true } catch { return false } })
}

/** A flag without a declared default is off by default. */
const defaultOf = (field: ConfigField): Json | undefined => field.default ?? (field.kind === 'boolean' ? false : undefined)

export function valueOf(field: ConfigField, context: RenderContext): Json | undefined {
  const value = field.get ? field.get(context) : context.payload.config?.[field.key]
  return value === undefined || value === null ? defaultOf(field) : value
}

/** Why the property cannot be changed now, or false. A Cell's own configuration needs `update` on the Cell in edit mode;
 * its data's state does not matter. */
export function readOnlyOf(field: ConfigField, context: RenderContext): string | false {
  if (field.readOnly) return field.readOnly(context)
  const reason = context.readOnlyReason
  const dataOnly = reason === aiwsT('aiworkspace.blocks.readOnly.dataUnavailable') || reason === aiwsT('aiworkspace.blocks.readOnly.noDataPermission')
  if (reason && !dataOnly) return reason
  if (context.mode !== 'edit' || !context.capabilities.includes('update')) return aiwsT('aiworkspace.blocks.noPermission')
  return false
}

/** What a value reads as in the edit history. */
function shown(field: ConfigField, value: Json | null): string {
  if (value === null || value === undefined || value === '') return aiwsT('aiworkspace.blocks.edit.cleared')
  const option = field.options?.find((o) => o.value === String(value))
  if (option) return option.label
  if (typeof value === 'boolean') return aiwsT(value ? 'aiworkspace.blockui.on' : 'aiworkspace.blockui.off')
  return String(value)
}

/** Write one property: the type's own writer, or `config.<key>` of the Cell (the default stored as absent). One undoable
 * commit. */
export function writeField(field: ConfigField, context: RenderContext, store: WorkspaceStore, value: Json | null) {
  if (field.set) { field.set(context, store, value); return }
  const fallback = defaultOf(field)
  const stored = value === null || value === '' || (fallback !== undefined && JSON.stringify(value) === JSON.stringify(fallback)) ? null : value
  setCellConfig(context, store, { [field.key]: stored }, aiwsT('aiworkspace.blocks.edit.change', { label: field.label, value: shown(field, value) }))
}

const asString = (value: Json | undefined) => (value === undefined || value === null ? '' : String(value))

/** A quick property as one toolbar item. */
function quickItem(definition: BlockDefinition, field: ConfigField, context: RenderContext, store: WorkspaceStore): ToolbarItem | null {
  const id = fieldId(definition, field)
  const value = valueOf(field, context)
  const disabled = readOnlyOf(field, context)
  const write = (next: Json | null) => writeField(field, context, store, next)
  if (field.picker) {
    const picker = field.picker
    return { kind: 'panel', id, icon: field.options?.find((o) => o.value === asString(value))?.icon ?? field.icon ?? SlidersHorizontal, label: field.label, disabled,
      render: (close) => picker({ value: asString(value), pick: (next) => { close(); if (next !== asString(value)) write(next) } }) }
  }
  switch (field.kind) {
    case 'color':
      return { kind: 'color', id, label: field.label, palette: field.palette, extras: field.extras, value: value === undefined ? undefined : asString(value), disabled, onPick: (hex) => write(hex || null) }
    case 'select':
      return { kind: 'menu', id, icon: field.options?.find((o) => o.value === asString(value))?.icon ?? field.icon, label: field.label, value: asString(value), disabled,
        items: (field.options ?? []).map((o) => ({ value: o.value, label: o.label, icon: o.icon })), onPick: (next) => write(next) }
    case 'boolean':
      return { kind: 'button', id, icon: field.icon, label: field.label, active: value === true, disabled, run: () => write(value !== true) }
    case 'number':
      if (!field.presets) return null
      return { kind: 'menu', id, icon: field.icon, label: field.label, value: asString(value), disabled,
        items: field.presets.map((n) => ({ value: String(n), label: String(n) })), onPick: (next) => write(Number(next)) }
    default:
      return null
  }
}

/** The quick properties of the Block for its toolbar (edit mode only: view mode has none, §10.4). */
export function quickItems(definition: BlockDefinition | undefined, context: RenderContext | null, store: WorkspaceStore): ToolbarItem[] {
  if (!definition || !context || context.mode !== 'edit') return []
  const fields = fieldsOf(definition, context).filter((f) => f.quick || f.quickGroup)
  const out: ToolbarItem[] = []
  const groups = new Set<string>()
  for (const field of fields) {
    if (field.quickGroup) {
      const group = field.quickGroup
      if (groups.has(group.id)) continue
      groups.add(group.id)
      const members = fields.filter((f) => f.quickGroup?.id === group.id)
      const disabled = members.every((f) => readOnlyOf(f, context)) ? readOnlyOf(members[0], context) : false
      out.push({ kind: 'panel', id: group.id, icon: group.icon, label: group.label, disabled, render: (close) => <QuickGroup definition={definition} fields={members} context={context} store={store} testId={`aiws-${group.id}-panel`} close={close} /> })
      continue
    }
    const item = quickItem(definition, field, context, store)
    if (item) out.push(item)
  }
  return out
}

/** Several quick properties behind one toolbar button: a row of choices each (`options`), a toggle for a flag. */
function QuickGroup({ definition, fields, context, store, testId, close }: { definition: BlockDefinition; fields: ConfigField[]; context: RenderContext; store: WorkspaceStore; testId: string; close: () => void }) {
  const { t } = useAiwsText()
  const flags = fields.filter((f) => f.kind === 'boolean')
  return (
    <div className="aiws-shape-panel" data-testid={testId}>
      {fields.filter((f) => f.kind !== 'boolean' && f.options).map((field) => {
        const value = asString(valueOf(field, context))
        const id = fieldId(definition, field)
        const disabled = Boolean(readOnlyOf(field, context))
        return (
          <div key={field.key} className="aiws-shape-panel-row">
            <span>{field.label}</span>
            <div className="aiws-segmented" role="group" aria-label={field.label}>
              {(field.options ?? []).map((o) => (
                <button key={o.value} type="button" aria-pressed={o.value === value} disabled={disabled} data-testid={`aiws-${id}-${o.value}`} onMouseDown={(e) => e.preventDefault()}
                  onClick={() => writeField(field, context, store, field.kind === 'number' ? Number(o.value) : o.value)}>{o.label}</button>
              ))}
            </div>
          </div>
        )
      })}
      {flags.length > 0 && (
        <div className="aiws-shape-panel-row">
          <span />
          <div className="aiws-segmented" role="group">
            {flags.map((field) => {
              const on = valueOf(field, context) === true
              return <button key={field.key} type="button" aria-pressed={on} disabled={Boolean(readOnlyOf(field, context))} data-testid={`aiws-${fieldId(definition, field)}`} onMouseDown={(e) => e.preventDefault()}
                onClick={() => writeField(field, context, store, !on)}>{field.label}</button>
            })}
          </div>
        </div>
      )}
      <div className="aiws-dialog-actions"><button type="button" onClick={close}>{t('aiworkspace.blocks.shape.done')}</button></div>
    </div>
  )
}

/** A text or number field that writes when it is left (blur, Enter): one commit per change, not per key. */
function Draft({ value, kind, disabled, testId, label, onCommit }: { value: string; kind: 'text' | 'number' | 'textarea'; disabled: boolean; testId: string; label: string; onCommit: (value: string) => void }) {
  const [draft, setDraft] = useState<string | null>(null)
  const done = () => { if (draft !== null && draft !== value) onCommit(draft); setDraft(null) }
  if (kind === 'textarea') {
    return <textarea rows={3} aria-label={label} value={draft ?? value} disabled={disabled} data-testid={testId} onChange={(e) => setDraft(e.target.value)} onBlur={done}
      onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setDraft(null) } }} />
  }
  return <input type={kind === 'number' ? 'number' : 'text'} step="any" aria-label={label} value={draft ?? value} disabled={disabled} data-testid={testId} onChange={(e) => setDraft(e.target.value)} onBlur={done}
    onKeyDown={(e) => { if (e.key === 'Enter') e.currentTarget.blur(); else if (e.key === 'Escape') { e.stopPropagation(); setDraft(null) } }} />
}

/** One property in the properties panel. */
export function PropertyControl({ definition, field, context, store }: { definition: BlockDefinition; field: ConfigField; context: RenderContext; store: WorkspaceStore }): ReactNode {
  const value = valueOf(field, context)
  const reason = readOnlyOf(field, context)
  const disabled = Boolean(reason)
  const testId = `aiws-${definition.type}-field-${field.key}`
  const write = (next: Json | null) => { if (!disabled) writeField(field, context, store, next) }
  const control = (() => {
    switch (field.kind) {
      case 'select':
        return (
          <select aria-label={field.label} value={asString(value)} disabled={disabled} data-testid={testId} onChange={(e) => write(e.target.value)}>
            {!field.options?.some((o) => o.value === asString(value)) && <option value={asString(value)}>{asString(value)}</option>}
            {(field.options ?? []).map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </select>
        )
      case 'boolean':
        return <input type="checkbox" aria-label={field.label} checked={value === true} disabled={disabled} data-testid={testId} onChange={(e) => write(e.target.checked)} />
      case 'color':
        return <PaletteGrid label={field.label} palette={field.palette ?? 'fill'} extras={field.extras} value={value === undefined ? undefined : asString(value)} disabled={disabled} testId={testId} onPick={(hex) => write(hex || null)} />
      case 'number':
        return <Draft kind="number" label={field.label} value={asString(value)} disabled={disabled} testId={testId} onCommit={(v) => { const n = Number(v); if (v.trim() === '') write(null); else if (Number.isFinite(n)) write(field.min !== undefined || field.max !== undefined ? Math.min(field.max ?? Infinity, Math.max(field.min ?? -Infinity, n)) : n) }} />
      default:
        return <Draft kind={field.kind} label={field.label} value={asString(value)} disabled={disabled} testId={testId} onCommit={(v) => write(v)} />
    }
  })()
  const Wrap = field.kind === 'color' ? 'div' : 'label'
  return (
    <Wrap className={`aiws-props-field is-${field.kind}`} title={reason || undefined} data-quick={field.quick || field.quickGroup ? 'true' : undefined}>
      <span className="aiws-props-label">{field.label}</span>
      {control}
    </Wrap>
  )
}
