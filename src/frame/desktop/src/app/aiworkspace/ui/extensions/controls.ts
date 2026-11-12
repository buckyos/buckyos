/* Pure helpers of extension controls (扩展控件 P0): which definitions are controls, and the instance-form rules over
 * a definition's `config_schema` (the A7 subset) and named `bindings`. */

import { z } from 'zod'
import type { BlockDefPayload, ControlBinding, EntityEnvelope, Json } from '../../api/types'
import { MOCK_WISH_DEF_ID } from '../wish/mockWishDef'

/** The Workspace's control definitions (the Mock wish executor's is not a control). */
export function controlDefinitions(entities: EntityEnvelope[]): EntityEnvelope[] {
  return entities.filter((e) => !e.deleted && e.type_id === 'buckyos.block-def' && e.entity_id !== MOCK_WISH_DEF_ID)
}

export interface PropSchema { type?: string | string[]; enum?: Json[]; default?: Json; title?: string; description?: string; minimum?: number; maximum?: number; minLength?: number; maxLength?: number }

/** The editable properties of a definition's schema (top-level object properties). */
export function schemaProperties(def: BlockDefPayload | undefined): { key: string; schema: PropSchema; required: boolean }[] {
  const schema = def?.config_schema as { type?: string; properties?: Record<string, PropSchema>; required?: string[] } | undefined
  if (!schema || typeof schema !== 'object' || !schema.properties) return []
  return Object.entries(schema.properties).map(([key, s]) => ({ key, schema: s ?? {}, required: (schema.required ?? []).includes(key) }))
}

/** Problems of `config` under the definition's schema (empty when it validates or there is no schema). */
export function configProblems(def: BlockDefPayload | undefined, config: Record<string, Json>): string[] {
  if (!def?.config_schema || typeof def.config_schema !== 'object') return []
  try {
    const parsed = z.fromJSONSchema(def.config_schema as never).safeParse(config)
    return parsed.success ? [] : parsed.error.issues.map((i) => `${i.path.join('.') || '·'}: ${i.message}`)
  } catch (error) {
    return [error instanceof Error ? error.message : String(error)]
  }
}


/** The newest `manifest_version` this host reads (扩展控件 P2/P3); a package above it is listed, never imported (Q33). */
export const HOST_MANIFEST_VERSION = 5

/** What a newer version of a control changes in the contract its instances rely on (`ext.upgrade`, Q30): the inputs of
 * the migration form. `next` is the new `component.json` (and `composition.json` for a composite). */
export interface InterfaceChanges {
  removedConfig: string[]
  newConfig: string[]
  requiredConfig: string[]
  removedBindings: string[]
  newBindings: string[]
  removedSlots: string[]
  newSlots: string[]
  removedEvents: string[]
}

export function interfaceChanges(current: BlockDefPayload, next: { config_schema?: Json; bindings?: Record<string, ControlBinding>; events?: Record<string, unknown> }, nextSlots: string[] | null): InterfaceChanges {
  const props = (schema: Json | undefined) => Object.keys((schema as { properties?: Record<string, unknown> } | undefined)?.properties ?? {})
  const oldConfig = props(current.config_schema)
  const newConfig = props(next.config_schema)
  const schema = next.config_schema as { properties?: Record<string, { default?: Json }>; required?: string[] } | undefined
  const oldBindings = Object.keys(current.bindings ?? {})
  const newBindings = Object.keys(next.bindings ?? {})
  const oldSlots = Object.keys(current.composition?.slots ?? {})
  return {
    removedConfig: oldConfig.filter((k) => !newConfig.includes(k)),
    newConfig: newConfig.filter((k) => !oldConfig.includes(k)),
    requiredConfig: (schema?.required ?? []).filter((k) => schema?.properties?.[k]?.default === undefined && !((current.config_schema as { required?: string[] } | undefined)?.required ?? []).includes(k)),
    removedBindings: oldBindings.filter((k) => !newBindings.includes(k)),
    newBindings: newBindings.filter((k) => !oldBindings.includes(k)),
    removedSlots: nextSlots ? oldSlots.filter((k) => !nextSlots.includes(k)) : [],
    newSlots: nextSlots ? nextSlots.filter((k) => !oldSlots.includes(k)) : [],
    removedEvents: Object.keys(current.events ?? {}).filter((k) => !(k in (next.events ?? {}))),
  }
}
