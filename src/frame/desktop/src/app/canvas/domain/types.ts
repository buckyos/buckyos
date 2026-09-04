/* ── BuckyOS AI Canvas – domain types (PRD §12) ── */

export const CANVAS_SCHEMA_VERSION = '0.1' as const
export type CanvasSchemaVersion = typeof CANVAS_SCHEMA_VERSION

export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

export interface Camera {
  x: number
  y: number
  zoom: number
}

export type BlockType =
  | 'text'
  | 'table'
  | 'wish'
  | 'metric'
  | 'chart'
  | 'frame'
  | 'group'
  | 'interactive'
  | 'image'
  | 'video'

export type GeneratedStatus = 'fresh' | 'stale' | 'broken'

export interface GeneratedMeta {
  runId: string
  wishBlockId: string
  agentAdapter: string
  generatedAt: string
  sourceRevisions: Array<{ refKey: string; revision: number }>
  status: GeneratedStatus
  userModified: boolean
  detached: boolean
  assumptions?: string[]
  warnings?: string[]
}

interface CanvasBlockBase {
  id: string
  sheetId: string
  title?: string
  rect: Rect
  zIndex: number
  locked: boolean
  contentRevision: number
  dataRevision: number
  generated?: GeneratedMeta
  createdAt: string
  updatedAt: string
}

/* ── text ── */
export interface TextBlockContent {
  text: string
  format: 'markdown' | 'plain'
}

/* ── table ── */
export type CellPrimitive = string | number | boolean | null
export type CellValueType = 'string' | 'number' | 'date' | 'boolean' | 'null'

export type TableCell =
  | {
      kind: 'value'
      value: CellPrimitive
      displayValue?: string
      valueType: CellValueType
      warning?: string
    }
  | {
      kind: 'ai'
      wishId: string
      displayValue?: string
      value?: CellPrimitive
      valueType?: CellValueType
    }

export interface TableColumn {
  id: string
  name: string
  width?: number
  inferredType?: CellValueType
}

export interface TableRow {
  id: string
  cells: Record<string, TableCell>
}

export interface CellWish {
  id: string
  prompt: string
  rowId: string
  columnId: string
  state: 'idle' | 'running' | 'succeeded' | 'failed'
  lastRunAt?: string
  error?: string
}

export interface TableBlockContent {
  columns: TableColumn[]
  rows: TableRow[]
  source?: {
    kind: 'manual' | 'csv' | 'xlsx' | 'paste' | 'sample'
    filename?: string
    worksheet?: string
    importedAt?: string
    truncated?: { originalRows: number; keptRows: number }
  }
  cellWishes?: Record<string, CellWish>
}

/* ── wish ── */
export type OutputPreference = 'auto' | 'table' | 'visual' | 'brief'

export type WishState =
  | 'idle'
  | 'planning'
  | 'waiting_permission'
  | 'running'
  | 'validating'
  | 'applying'
  | 'succeeded'
  | 'failed'
  | 'cancelled'

export type ContextRef =
  | { kind: 'block'; blockId: string; revision: number }
  | {
      kind: 'tableRange'
      blockId: string
      range: { rowStart: number; rowEnd: number; colStart: number; colEnd: number }
      revision: number
    }

export interface WishRunSummary {
  runId: string
  startedAt: string
  finishedAt?: string
  status: WishState
  promptExcerpt: string
  sourceRevisions: Array<{ refKey: string; revision: number }>
  groupId?: string
  error?: string
  adapter: string
}

export interface RefreshPolicy {
  mode: 'manual' | 'notify_on_change' | 'on_change' | 'interval'
  intervalMinutes?: number
}

export interface WishBlockContent {
  prompt: string
  contextRefs: ContextRef[]
  outputPreference: OutputPreference
  refreshPolicy: RefreshPolicy
  state: WishState
  lastRunId?: string
  lastError?: string
  generatedGroupIds: string[]
  runHistory: WishRunSummary[]
}

/* ── metric ── */
export interface MetricBlockContent {
  label: string
  value: number | string
  unit?: string
  format?: 'plain' | 'percent' | 'currency'
  delta?: { value: number; label?: string; format?: 'plain' | 'percent' }
  tone?: 'neutral' | 'positive' | 'negative' | 'warning'
  note?: string
}

/* ── chart ── */
export type ChartType = 'bar' | 'line' | 'pie' | 'horizontalBar'

export interface ChartBlockContent {
  chartType: ChartType
  data:
    | { kind: 'inline'; rows: Record<string, CellPrimitive>[] }
    | { kind: 'tableBlock'; blockId: string }
  xField?: string
  yFields?: string[]
  seriesField?: string
  aggregation?: 'sum' | 'avg' | 'count' | 'min' | 'max'
  sort?: { field: string; direction: 'asc' | 'desc' }
  numberFormat?: 'plain' | 'percent' | 'currency'
  caption?: string
}

/* ── image / video ── */
export interface MediaSource {
  kind: 'upload' | 'paste' | 'drop' | 'url' | 'generated'
  filename?: string
  bytes?: number
  /** for generated media: the prompt / seed that produced it */
  prompt?: string
  seed?: number
}

export interface ImageBlockContent {
  /** data: URL or http(s) URL; empty string = placeholder waiting for a file */
  src: string
  alt?: string
  caption?: string
  fit: 'contain' | 'cover'
  naturalWidth?: number
  naturalHeight?: number
  source?: MediaSource
}

export interface VideoFrame {
  src: string
  durationMs: number
  caption?: string
}

export interface VideoBlockContent {
  /** real video file (data: / http URL). When absent, `frames` are played as a frame sequence. */
  src?: string
  poster?: string
  frames?: VideoFrame[]
  durationMs?: number
  caption?: string
  width?: number
  height?: number
  source?: MediaSource
}

/* ── frame / group / interactive ── */
export interface FrameBlockContent {
  color?: string
  moveChildren: boolean
}

export interface GroupBlockContent {
  childBlockIds: string[]
  summary?: string
}

export interface InteractiveBlockContent {
  html: string
  css: string
  js: string
  manifest: { name: string; inputs: string[]; outputs: string[] }
}

export type BlockContentMap = {
  text: TextBlockContent
  table: TableBlockContent
  wish: WishBlockContent
  metric: MetricBlockContent
  chart: ChartBlockContent
  frame: FrameBlockContent
  group: GroupBlockContent
  interactive: InteractiveBlockContent
  image: ImageBlockContent
  video: VideoBlockContent
}

export type CanvasBlockOf<T extends BlockType> = CanvasBlockBase & {
  type: T
  content: BlockContentMap[T]
}

export type CanvasBlock =
  | CanvasBlockOf<'text'>
  | CanvasBlockOf<'table'>
  | CanvasBlockOf<'wish'>
  | CanvasBlockOf<'metric'>
  | CanvasBlockOf<'chart'>
  | CanvasBlockOf<'frame'>
  | CanvasBlockOf<'group'>
  | CanvasBlockOf<'interactive'>
  | CanvasBlockOf<'image'>
  | CanvasBlockOf<'video'>

export type TableBlock = CanvasBlockOf<'table'>
export type WishBlock = CanvasBlockOf<'wish'>
export type GroupBlock = CanvasBlockOf<'group'>
export type ImageBlock = CanvasBlockOf<'image'>
export type VideoBlock = CanvasBlockOf<'video'>

/* ── bindings ── */
export interface DataBinding {
  id: string
  source: ContextRef
  targetBlockId: string
  createdByRunId: string
}

/* ── presentation ── */
export interface PresentationStep {
  id: string
  title?: string
  note?: string
  camera: Camera
  targetBlockIds: string[]
  transitionMs: number
}

export interface PresentationPath {
  id: string
  name: string
  steps: PresentationStep[]
  createdAt: string
  updatedAt: string
}

/* ── comments (P0.5 – type only) ── */
export interface CommentThread {
  id: string
  blockId: string
  resolved: boolean
  messages: Array<{ id: string; author: string; text: string; createdAt: string }>
}

/* ── sheet / document ── */
export interface CanvasSheet {
  id: string
  name: string
  order: number
  blockIds: string[]
  camera: Camera
}

export interface CanvasDocument {
  schemaVersion: CanvasSchemaVersion
  id: string
  title: string
  revision: number
  activeSheetId: string
  sheets: CanvasSheet[]
  blocks: Record<string, CanvasBlock>
  bindings: DataBinding[]
  presentationPaths: PresentationPath[]
  comments: CommentThread[]
  createdAt: string
  updatedAt: string
  metadata: {
    ownerDid?: string
    sourceTemplateId?: string
    importedFrom?: string
  }
}

export interface CanvasSnapshot {
  id: string
  docId: string
  name: string
  createdAt: string
  revision: number
  doc: CanvasDocument
}

/* ── size constraints (PRD §13.5) ── */
export const MIN_BLOCK_WIDTH = 80
export const MIN_BLOCK_HEIGHT = 40
export const MAX_TABLE_ROWS = 20_000
export const MAX_TABLE_COLS = 100
export const MAX_IMPORT_BYTES = 20 * 1024 * 1024
/** Images are re-encoded down to this edge length / byte size before they enter the document. */
export const MAX_IMAGE_EDGE = 2048
export const MAX_IMAGE_BYTES = 4 * 1024 * 1024
