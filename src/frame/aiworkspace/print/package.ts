/* The print package (打印 TODO §4.2, Q20, T7): what the aiworkspace service builds for one print and `media.page`
 * renders. A zip with, at its root:
 *
 *   index.html   print.js   print.css        the print renderer (the Desktop build's aiws-print/, classic scripts)
 *   data.js                                  `window.__AIWS_PRINT__ = <PrintPackage>;`
 *   assets/<object id file name>.<ext>       the bytes of the assets the pages show (the extension of the media type)
 *
 * Everything loads from the package over file:// (no network, no module scripts, no fetch of files). The renderer
 * sets `window.__AIWS_PRINT_READY__ = true` once every page is laid out; `media.page` prints then. */

import type { Paper } from './page.ts'
import type { FontSettings } from './fonts.ts'

export const PRINT_FORMAT = 'buckyos.aiws-print'
export const PRINT_FORMAT_VERSION = 1
/** `media.page`'s `wait_ready`. */
export const READY_EXPRESSION = 'window.__AIWS_PRINT_READY__ === true'

export interface PrintJob {
  surface_id: string
  mode: 'flow' | 'free'
  /** The presentation path of a free canvas. */
  path_id: string | null
  /** As printed (already in its orientation). */
  paper: Paper
  /** The PDF Frame control used, if any. */
  frame_id: string | null
  /** The Workspace's mapping of `sans` / `serif` / `mono` (root `config.fonts`). */
  fonts: FontSettings
  /** The canvas title: the page context's `title`. */
  title: string
  /** The page context's `date`: the person's own day in their language, as a PDF Frame shows it while edited. */
  date: string
  /** The language of the person printing (placeholder texts). */
  locale: string
}

/** Rows of the WASM `Replica` (its constructor's `tables_json`): `{ entities, tree_edges, table_fields, table_records,
 * richtext_states, refs, assets }`, BLOBs as base64 — the caller's readable, shared rows of a fixed head. */
export type ReplicaRows = Record<string, unknown[]>

export type PrintIssueCode =
  /** A step of the path that is not printed (disabled, its target gone, on a flow canvas). */
  | 'step_skipped'
  /** A Surface on the path the caller may not export: its steps are not printed. */
  | 'surface_not_exportable'
  /** A Surface on the path the caller may not read: its steps are not printed. */
  | 'surface_unreadable'
  /** An asset whose bytes are not in the store: shown as a placeholder. */
  | 'asset_missing'
  /** An asset too large to travel in the package (video and audio: their poster only). */
  | 'asset_omitted'
  /** Content of the page the caller may not read: left empty, unnamed. */
  | 'content_unreadable'

export interface PrintIssue {
  code: PrintIssueCode
  entity_id?: string
  step_id?: string
  surface_id?: string
  detail?: string
}

export interface PrintPackage {
  format: typeof PRINT_FORMAT
  format_version: number
  workspace_id: string
  /** The Workspace title. */
  title: string
  epoch: string
  head_seq: number
  principal: string
  /** When the printed head was written (its Commit's time, else the Workspace's creation): the same head and job give
   * the same package bytes, so `media.page` answers a reprint from its cache. */
  printed_at: string
  job: PrintJob
  rows: ReplicaRows
  /** object id → its file in the package (`assets/…`). */
  assets: Record<string, string>
  issues: PrintIssue[]
}

/** `print.package` (the Desktop → the aiworkspace service). */
export interface PrintPackageParams {
  workspace_id: string
  surface_id: string
  path_id?: string
  paper: Paper
  frame_id?: string
  date: string
  locale: string
}

export interface PrintPackageResult {
  ok: true
  /** The package (a FileObject the caller holds): `media.page`'s `obj_id`. */
  obj_id: string
  size: number
  job: PrintJob
  issues: PrintIssue[]
}

declare global {
  interface Window {
    __AIWS_PRINT__?: PrintPackage
    __AIWS_PRINT_READY__?: boolean
  }
}
