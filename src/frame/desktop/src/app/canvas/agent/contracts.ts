/* ── Agent protocol (PRD §13) – shared by Mock and HTTP adapters ── */

import type {
  CanvasBlock,
  CellPrimitive,
  DataBinding,
  OutputPreference,
  PresentationStep,
  TableCell,
} from '../domain/types'

export const AGENT_PROTOCOL_VERSION = '0.1' as const

export type AgentCapability =
  | 'read_canvas_context'
  | 'create_standard_blocks'
  | 'create_interactive_block'
  | 'generate_media'

export type AgentContextItem =
  | {
      kind: 'table'
      refKey: string
      blockId: string
      title: string
      columns: Array<{ name: string; type: string }>
      rows: Record<string, CellPrimitive>[]
      totalRows: number
      truncated: boolean
      revision: number
    }
  | { kind: 'text'; refKey: string; blockId: string; title: string; text: string; revision: number }
  | { kind: 'metric'; refKey: string; blockId: string; title: string; label: string; value: string; revision: number }
  | { kind: 'image'; refKey: string; blockId: string; title: string; src: string; alt?: string; caption?: string; width?: number; height?: number; revision: number }
  | { kind: 'group'; refKey: string; blockId: string; title: string; summary: string; items: AgentContextItem[]; revision: number }
  | { kind: 'other'; refKey: string; blockId: string; title: string; type: string; summary: string; revision: number }

export interface AgentRunRequest {
  protocolVersion: typeof AGENT_PROTOCOL_VERSION
  runId: string
  canvas: { id: string; revision: number; locale: string }
  wish: { blockId: string; prompt: string; outputPreference: OutputPreference }
  context: AgentContextItem[]
  destination: { sheetId: string; anchor: { x: number; y: number }; maxWidth: number }
  capabilities: AgentCapability[]
  /** cell-level wish (table AI cell) – optional */
  cell?: { tableBlockId: string; rowId: string; columnId: string }
}

export type AgentStage = 'planning' | 'running' | 'validating' | 'applying'

export type AgentRunEvent =
  | { type: 'status'; stage: AgentStage; message: string }
  | { type: 'progress'; stage: AgentStage; percent: number; message: string }
  | { type: 'warning'; message: string }
  | { type: 'log'; message: string }
  | { type: 'completed'; jobId: string }

export type CanvasPatchOperation =
  | { op: 'createBlock'; block: CanvasBlock }
  | { op: 'updateBlock'; blockId: string; patch: Partial<Pick<CanvasBlock, 'title' | 'rect' | 'content'>> }
  | { op: 'createBinding'; binding: DataBinding }
  | { op: 'createGroup'; groupId: string; childBlockIds: string[] }
  | { op: 'resizeToFit'; blockId: string }
  | { op: 'addPresentationStep'; pathId: string; step: PresentationStep }
  | {
      op: 'updateTableCells'
      blockId: string
      cells: Array<{ rowId: string; columnId: string; cell: TableCell }>
    }

export interface CanvasPatch {
  protocolVersion: typeof AGENT_PROTOCOL_VERSION
  runId: string
  baseCanvasRevision: number
  summary: string
  assumptions: string[]
  warnings: string[]
  operations: CanvasPatchOperation[]
}

export interface CanvasAgentAdapter {
  id: string
  health(): Promise<{ available: boolean; message?: string }>
  run(
    request: AgentRunRequest,
    onEvent: (event: AgentRunEvent) => void,
    signal: AbortSignal,
  ): Promise<CanvasPatch>
}

export class AgentRunError extends Error {
  kind: 'unavailable' | 'timeout' | 'cancelled' | 'invalid_patch' | 'conflict' | 'failed'
  details?: string[]
  constructor(kind: AgentRunError['kind'], message: string, details?: string[]) {
    super(message)
    this.kind = kind
    this.details = details
  }
}
