/* Pure helpers of extension controls (扩展控件 P0): which definitions are controls, and the instance-form rules over
 * a definition's `config_schema` (the A7 subset) and named `bindings`. */

import { z } from 'zod'
import type { BlockDefPayload, EntityEnvelope, Json } from '../../api/types'
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

/** Required bindings left unbound. */
export function missingBindings(def: BlockDefPayload | undefined, bindings: Record<string, string>): string[] {
  return Object.entries(def?.bindings ?? {}).filter(([name, b]) => b.required && !bindings[name]).map(([name]) => name)
}

