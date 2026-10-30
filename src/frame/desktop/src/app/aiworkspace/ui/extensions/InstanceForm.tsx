/* The initialization of a control instance (扩展控件 P0, B8; 扩展实现思路 §6.3): its properties from the definition's
 * `config_schema` (the A7 subset: type / enum / default / required / title / description / minimum / maximum /
 * minLength / maxLength) and its named data bindings. Only what the user changes is stored as an override; a
 * default stays the definition's. The same subset is what `resolve` validates with `z.fromJSONSchema`. */

import type { BlockDefPayload, Json } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { dataFor } from '../canvas/catalog'
import { schemaProperties, type PropSchema } from './controls'

const kindOf = (s: PropSchema) => (Array.isArray(s.type) ? s.type.find((t) => t !== 'null') : s.type) ?? 'string'

export function InstanceForm({ def, config, bindings, onConfig, onBindings }: {
  def: BlockDefPayload
  config: Record<string, Json>
  bindings: Record<string, string>
  onConfig: (config: Record<string, Json>) => void
  onBindings: (bindings: Record<string, string>) => void
}) {
  const store = useStore()
  const { t } = useAiwsText()
  const props = schemaProperties(def)
  const set = (key: string, value: Json | undefined) => {
    const next = { ...config }
    if (value === undefined || value === '') delete next[key]
    else next[key] = value
    onConfig(next)
  }
  const named = Object.entries(def.bindings ?? {})
  if (props.length === 0 && named.length === 0) return null
  return (
    <div className="aiws-instance-form" data-testid="aiws-instance-form">
      {props.map(({ key, schema, required }) => {
        const label = `${schema.title ?? key}${required ? ' *' : ''}`
        const value = config[key]
        const placeholder = schema.default !== undefined ? String(schema.default) : undefined
        const kind = kindOf(schema)
        return (
          <label key={key} className="aiws-field" title={schema.description}>{label}
            {schema.enum ? (
              <select value={value === undefined ? '' : String(value)} data-testid={`aiws-instance-config-${key}`} onChange={(e) => set(key, e.target.value === '' ? undefined : (schema.enum ?? []).find((o) => String(o) === e.target.value) ?? e.target.value)}>
                <option value="">{placeholder !== undefined ? t('aiworkspace.ext.form.default', { value: placeholder }) : t('aiworkspace.ext.form.unset')}</option>
                {schema.enum.map((o) => <option key={String(o)} value={String(o)}>{String(o)}</option>)}
              </select>
            ) : kind === 'boolean' ? (
              <input type="checkbox" checked={value === undefined ? schema.default === true : value === true} data-testid={`aiws-instance-config-${key}`} onChange={(e) => set(key, e.target.checked)} />
            ) : kind === 'number' || kind === 'integer' ? (
              <input type="number" value={value === undefined ? '' : String(value)} placeholder={placeholder} min={schema.minimum} max={schema.maximum} step={kind === 'integer' ? 1 : 'any'} data-testid={`aiws-instance-config-${key}`}
                onChange={(e) => set(key, e.target.value === '' ? undefined : Number(e.target.value))} />
            ) : (
              <input value={value === undefined ? '' : String(value)} placeholder={placeholder} maxLength={schema.maxLength} data-testid={`aiws-instance-config-${key}`} onChange={(e) => set(key, e.target.value)} />
            )}
            {schema.description && <span className="aiws-muted">{schema.description}</span>}
          </label>
        )
      })}
      {named.map(([name, b]) => {
        const options = dataFor(store, b.accepts && b.accepts.length > 0 ? b.accepts : null)
        return (
          <label key={name} className="aiws-field" title={b.description}>{t('aiworkspace.ext.form.binding', { name: b.title ?? name })}{b.required ? ' *' : ''}
            <select value={bindings[name] ?? ''} data-testid={`aiws-instance-binding-${name}`} onChange={(e) => { const next = { ...bindings }; if (e.target.value) next[name] = e.target.value; else delete next[name]; onBindings(next) }}>
              <option value="">{t(options.length === 0 ? 'aiworkspace.canvas.catalog.noData' : 'aiworkspace.canvas.catalog.chooseData')}</option>
              {options.map((e) => <option key={e.entity_id} value={e.entity_id}>{t('aiworkspace.canvas.catalog.nameWithType', { name: e.title ?? e.name ?? e.entity_id, type: e.type_id.replace('buckyos.', '') })}</option>)}
            </select>
            {b.description && <span className="aiws-muted">{b.description}</span>}
          </label>
        )
      })}
    </div>
  )
}
