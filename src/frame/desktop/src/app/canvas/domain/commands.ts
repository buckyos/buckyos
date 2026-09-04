/* ── Command bus contract (PRD §15.4) – every document mutation is a command ── */

import type { CanvasPatch } from '../agent/contracts'
import type {
  CanvasBlock,
  CanvasDocument,
  Camera,
  PresentationStep,
  Rect,
  TableCell,
  TableColumn,
  TableRow,
  WishRunSummary,
  WishState,
} from './types'

export type CanvasCommand =
  | { type: 'SET_TITLE'; title: string }
  | { type: 'CREATE_BLOCKS'; blocks: CanvasBlock[] }
  | { type: 'DELETE_BLOCKS'; ids: string[] }
  | { type: 'MOVE_BLOCKS'; ids: string[]; dx: number; dy: number }
  | { type: 'RESIZE_BLOCK'; id: string; rect: Rect }
  | {
      type: 'UPDATE_BLOCK'
      id: string
      patch: Partial<Pick<CanvasBlock, 'title' | 'locked' | 'zIndex' | 'rect'>> & {
        content?: CanvasBlock['content']
      }
      /** manual edit of an AI-generated block → generated.userModified = true */
      userEdit?: boolean
    }
  | { type: 'REORDER_Z'; id: string; to: 'front' | 'back' }
  | {
      type: 'UPDATE_TABLE_CELLS'
      id: string
      edits: Array<{ rowId: string; columnId: string; cell: TableCell }>
    }
  | {
      type: 'TABLE_STRUCTURE'
      id: string
      action:
        | { kind: 'addRow'; afterRowId?: string; row?: TableRow }
        | { kind: 'addColumn'; afterColumnId?: string; column?: TableColumn }
        | { kind: 'deleteRows'; rowIds: string[] }
        | { kind: 'deleteColumns'; columnIds: string[] }
        | { kind: 'renameColumn'; columnId: string; name: string }
        | { kind: 'setColumnWidth'; columnId: string; width: number }
        | { kind: 'setCellWish'; key: string; wish: import('./types').CellWish | null }
    }
  | { type: 'ADD_SHEET'; name?: string }
  | { type: 'RENAME_SHEET'; id: string; name: string }
  | { type: 'DELETE_SHEET'; id: string }
  | { type: 'MOVE_SHEET'; id: string; direction: -1 | 1 }
  | { type: 'DUPLICATE_SHEET'; id: string }
  | { type: 'SET_ACTIVE_SHEET'; id: string }
  | { type: 'SET_CAMERA'; sheetId: string; camera: Camera }
  | {
      type: 'APPLY_AGENT_PATCH'
      patch: CanvasPatch
      wishId: string
      adapter: string
      replaceGroupIds: string[]
    }
  | { type: 'DETACH_GENERATED'; groupId: string }
  | { type: 'WISH_SET_STATE'; id: string; state: WishState; error?: string; runId?: string }
  | { type: 'WISH_PUSH_HISTORY'; id: string; summary: WishRunSummary }
  | { type: 'PRESENTATION_CREATE_PATH'; name: string; id?: string }
  | { type: 'PRESENTATION_RENAME_PATH'; pathId: string; name: string }
  | { type: 'PRESENTATION_DELETE_PATH'; pathId: string }
  | { type: 'PRESENTATION_ADD_STEP'; pathId: string; step: PresentationStep; index?: number }
  | { type: 'PRESENTATION_UPDATE_STEP'; pathId: string; stepId: string; patch: Partial<PresentationStep> }
  | { type: 'PRESENTATION_REMOVE_STEP'; pathId: string; stepId: string }
  | { type: 'PRESENTATION_MOVE_STEP'; pathId: string; stepId: string; direction: -1 | 1 }
  | { type: 'RESTORE_SNAPSHOT'; doc: CanvasDocument }

/** Commands that are navigation/runtime only: no history entry, no revision bump. */
export const QUIET_COMMANDS = new Set<CanvasCommand['type']>([
  'SET_ACTIVE_SHEET',
  'SET_CAMERA',
  'WISH_SET_STATE',
  'WISH_PUSH_HISTORY',
])
