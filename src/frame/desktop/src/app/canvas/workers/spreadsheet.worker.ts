/* ── Web Worker: file parsing off the main thread (PRD FR-TABLE-004) ── */

import { parseCsvBuffer } from '../data/csv'
import { listXlsxSheets, readXlsxSheet } from '../data/xlsx'

export type WorkerRequest =
  | { id: number; kind: 'csv'; buffer: ArrayBuffer }
  | { id: number; kind: 'xlsx-list'; buffer: ArrayBuffer }
  | { id: number; kind: 'xlsx-sheet'; buffer: ArrayBuffer; path: string; maxRows?: number }

export type WorkerResponse = { id: number; ok: true; result: unknown } | { id: number; ok: false; error: string }

self.onmessage = async (ev: MessageEvent<WorkerRequest>) => {
  const req = ev.data
  try {
    let result: unknown
    if (req.kind === 'csv') result = parseCsvBuffer(req.buffer)
    else if (req.kind === 'xlsx-list') result = await listXlsxSheets(req.buffer)
    else result = await readXlsxSheet(req.buffer, req.path, req.maxRows)
    const res: WorkerResponse = { id: req.id, ok: true, result }
    self.postMessage(res)
  } catch (err) {
    const res: WorkerResponse = { id: req.id, ok: false, error: err instanceof Error ? err.message : String(err) }
    self.postMessage(res)
  }
}
