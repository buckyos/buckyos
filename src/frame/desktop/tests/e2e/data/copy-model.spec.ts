import { test, expect } from '@playwright/test'
import { copyEntryOf, copyResultOf } from '../../../src/app/filebrowser/data/copyResult'
import type { CopyItem } from '../../../src/api/nfs_copy'

const item = (id: number): CopyItem => ({ id, source_index: 0, source_path: `/home/source/文件-${id}.txt`, target_path: `/home/destination/文件-${id}.txt`, kind: 'file', size: 0, mtime: 1700000123, status: 'success' })

test('copy result mapping preserves identity, zero bytes, errors and unknown metadata', () => {
  const value = copyResultOf(item(7))
  expect(value.resultId).toBe(7)
  expect(value.entry.sizeBytes).toBe(0)
  expect(value.entry.modifiedAt).toBe('2023-11-14T22:15:23.000Z')
  expect(value.targetPath).toBe('/home/destination/文件-7.txt')
  expect(copyEntryOf({ ...item(1), kind: 'dir', mtime: null }).sizeBytes).toBeUndefined()
  expect(copyEntryOf({ ...item(1), mtime: null }).modifiedAt).toBe('')
  const failed = copyResultOf({ ...item(1), status: 'failed', error: { code: 'PERMISSION_DENIED', message: 'Access denied' } })
  expect(failed.status).toBe('failed')
  expect(failed.error?.fallback).toBe('Access denied')
  expect(copyResultOf({ ...item(1), status: 'cancelled' }).status).toBe('cancelled')
})

test('copy model synthetic 1, 10, 1000 and 1000000 items use bounded pages and server aggregates', async ({}, testInfo) => {
  const measurements = []
  for (const total of [1, 10, 1000, 1000000]) {
    const started = performance.now()
    let transformed = 0
    let maxWindow = 0
    for (let after = 0; after < total; after += 100) {
      const rows = new Map(Array.from({ length: Math.min(100, total - after) }, (_, i) => [after + i, item(after + i)]))
      const page = [...rows.values()].map(copyResultOf)
      transformed += page.length
      maxWindow = Math.max(maxWindow, page.length)
    }
    expect(transformed).toBe(total)
    expect(maxWindow).toBeLessThanOrEqual(100)
    const pages = [1, 70, 4321].map((page) => {
      const after = (page - 1) * 100
      const pageStart = performance.now()
      const results = Array.from({ length: Math.max(0, Math.min(100, total - after)) }, (_, i) => copyResultOf(item(after + i)))
      return { page, results: results.length, mappingMs: performance.now() - pageStart, rpcCalls: after < total ? 1 : 0 }
    })
    measurements.push({ total, mappingMs: performance.now() - started, maxWindow, pages, fullEnumerationRpcCalls: Math.ceil(total / 100), estimatedPageWith100msNetworkMs: 100 + pages[0].mappingMs })
  }
  await testInfo.attach('copy-model-performance.json', { body: JSON.stringify({ synthetic: true, networkLatencyMeasured: false, measurements }, null, 2), contentType: 'application/json' })
})
