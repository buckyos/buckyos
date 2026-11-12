/* A table Block made from the catalog's standard-table variants (`config.standard`): which standard table it stands for,
 * and a free name for a new one in the canvas's content folder. A control's data is settled by its on_bind (binding.ts). */

import { CALENDAR_SCHEMA_ID, TODO_SCHEMA_ID } from 'buckyos'
import { uniqueName } from '@table/operations'
import type { Json } from '../../../api/types'
import type { WorkspaceStore } from '../../../state/store'

/** A standard table: its schema, and which of websdk `standardTableOp`'s tables it is. */
export interface StandardTable { schemaId: string; table: 'todo' | 'calendar' }

/** A table name free in `folderId` (names are unique among siblings): `base`, else `base (2)` … */
export function freeTableName(store: WorkspaceStore, folderId: string, base: string): string {
  return uniqueName(base, new Set(store.outline.childrenOf(folderId).filter((e) => !e.deleted).map((e) => e.name).filter((n): n is string => Boolean(n))))
}

/** The standard table a table Block's catalog variant stands for (`config.standard`: `todo` / `calendar`), else null. */
export function standardTableOf(config: Record<string, Json> | null | undefined): StandardTable | null {
  const kind = config?.standard
  if (kind === 'todo') return { schemaId: TODO_SCHEMA_ID, table: 'todo' }
  if (kind === 'calendar') return { schemaId: CALENDAR_SCHEMA_ID, table: 'calendar' }
  return null
}
