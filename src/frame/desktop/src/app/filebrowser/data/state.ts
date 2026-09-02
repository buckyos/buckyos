/**
 * Common async-state vocabulary for File Browser fetch points (UI_DATAMODEL.md
 * §4). `FileItemList` stays the optimized listing controller; everything else
 * (sidebar sources, search, preview enrichment, mutations, transfers) speaks
 * `DataState`/`MutationState` so one unavailable source never blanks the app.
 */

import type {
  CollectionSummary,
  DeviceNode,
  DfsNode,
  FileEntry,
  ISODateTime,
  LocationUrl,
  SearchResultPage,
  Topic,
} from '../types'
import type { FileItem, LocationCapabilities, LocationMeta } from './FolderReader'
import type { UploadCandidateInput } from './schemas'

export type LoadingState = 'idle' | 'loading' | 'success' | 'error'

/** Normalized, display-safe error. Raw paths/refs/server text stay out. */
export interface UiError {
  code: string
  messageKey: string
  fallback: string
  retryable: boolean
  details?: Record<string, unknown>
}

export interface DataState<T> {
  status: LoadingState
  data: T | null
  error: UiError | null
}

export type MutationStatus = 'idle' | 'submitting' | 'success' | 'error'

export interface MutationState<TResult = void> {
  status: MutationStatus
  result: TResult | null
  error: UiError | null
}

// ─── Constructors ───

export function dataIdle<T>(): DataState<T> {
  return { status: 'idle', data: null, error: null }
}

/** Loading keeps the previous projection when one exists (progress states). */
export function dataLoading<T>(previous?: T | null): DataState<T> {
  return { status: 'loading', data: previous ?? null, error: null }
}

export function dataSuccess<T>(data: T): DataState<T> {
  return { status: 'success', data, error: null }
}

/** Errors keep the last successful projection unless told to drop it. */
export function dataError<T>(error: UiError, previous?: T | null): DataState<T> {
  return { status: 'error', data: previous ?? null, error }
}

/**
 * Normalize an unknown thrown value into UiError. The mock layer throws plain
 * Errors; the NFSP adapter maps backend codes here (§8.5) before anything
 * reaches a component.
 */
export function toUiError(err: unknown, retryable = true): UiError {
  if (isUiError(err)) return err
  const message = err instanceof Error ? err.message : String(err)
  return {
    code: 'INTERNAL',
    messageKey: 'filebrowser.error.generic',
    fallback: message || 'Something went wrong',
    retryable,
  }
}

function isUiError(value: unknown): value is UiError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    'messageKey' in value &&
    'fallback' in value &&
    'retryable' in value
  )
}

// ─── Location listing state (§4.2) ───

export interface ListPageInfo {
  loadedCount: number
  totalCount?: number
  hasMore: boolean
  nextCursor?: string
}

export interface LocationListData {
  url: LocationUrl
  capabilities: LocationCapabilities
  meta?: LocationMeta
  pageInfo: ListPageInfo
}

export type LocationListState = DataState<LocationListData>

// ─── Sidebar source states (§4.3) ───

export interface FileBrowserSidebarState {
  dfs: DataState<DfsNode[]>
  devices: DataState<DeviceNode[]>
  topics: DataState<Topic[]>
  collections: DataState<CollectionSummary[]>
}

// ─── Search state (§4.4) ───

export type SearchViewState = DataState<SearchResultPage>

// ─── Preview state (§4.5) ───

export interface PreviewData {
  item: FileItem
  topics: Topic[]
}

export type PreviewState = DataState<PreviewData>

// ─── Upload/transfer progress (§4.7) ───

export type TransferStatus =
  | 'queued'
  | 'hashing'
  | 'probing'
  | 'uploading'
  | 'committing'
  | 'success'
  | 'error'
  | 'cancelled'

export interface TransferTask {
  id: string
  targetUrl: LocationUrl
  candidate: UploadCandidateInput
  status: TransferStatus
  bytesSent: number
  totalBytes: number
  error: UiError | null
  /** Set once the destination reader exposes the committed entry. */
  committedAt?: ISODateTime
  /** The entry produced by a successful commit (mock: inserted into the index). */
  committedEntry?: FileEntry
}
