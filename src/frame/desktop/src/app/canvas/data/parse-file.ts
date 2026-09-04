/* ── Main-thread client for the parsing worker, with in-thread fallback ── */

import { parseCsvBuffer, type ParsedSheet } from './csv'
import { listXlsxSheets, readXlsxSheet, type XlsxSheetResult, type XlsxWorkbook } from './xlsx'
import type { WorkerRequest, WorkerResponse } from '../workers/spreadsheet.worker'

let worker: Worker | null = null
let seq = 0
const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>()

function getWorker(): Worker | null {
  if (worker) return worker
  try {
    worker = new Worker(new URL('../workers/spreadsheet.worker.ts', import.meta.url), { type: 'module' })
    worker.onmessage = (ev: MessageEvent<WorkerResponse>) => {
      const p = pending.get(ev.data.id)
      if (!p) return
      pending.delete(ev.data.id)
      if (ev.data.ok) p.resolve(ev.data.result)
      else p.reject(new Error(ev.data.error))
    }
    worker.onerror = () => {
      for (const p of pending.values()) p.reject(new Error('解析线程异常'))
      pending.clear()
      worker = null
    }
    return worker
  } catch {
    return null
  }
}

type WorkerRequestBody = WorkerRequest extends infer R ? (R extends { id: number } ? Omit<R, 'id'> : never) : never

function call<T>(req: WorkerRequestBody, fallback: () => Promise<T>): Promise<T> {
  const w = getWorker()
  if (!w) return fallback()
  const id = ++seq
  return new Promise<T>((resolve, reject) => {
    pending.set(id, { resolve: (v) => resolve(v as T), reject })
    // copy buffer so the caller can reuse it (xlsx needs it for list + sheet)
    const buffer = req.buffer.slice(0)
    w.postMessage({ ...req, id, buffer } as WorkerRequest, [buffer])
  })
}

export function parseCsv(buffer: ArrayBuffer): Promise<ParsedSheet> {
  return call({ kind: 'csv', buffer }, async () => parseCsvBuffer(buffer))
}

export function listSheets(buffer: ArrayBuffer): Promise<XlsxWorkbook> {
  return call({ kind: 'xlsx-list', buffer }, () => listXlsxSheets(buffer))
}

export function readSheet(buffer: ArrayBuffer, path: string, maxRows?: number): Promise<XlsxSheetResult> {
  return call({ kind: 'xlsx-sheet', buffer, path, maxRows }, () => readXlsxSheet(buffer, path, maxRows))
}

export function fileKind(name: string): 'csv' | 'xlsx' | 'aicanvas' | null {
  const lower = name.toLowerCase()
  if (lower.endsWith('.csv') || lower.endsWith('.tsv') || lower.endsWith('.txt')) return 'csv'
  if (lower.endsWith('.xlsx') || lower.endsWith('.xlsm')) return 'xlsx'
  if (lower.endsWith('.aicanvas.json') || lower.endsWith('.json')) return 'aicanvas'
  return null
}
