/**
 * Bytes of mock files. Seed entries without attached bytes read as a small generated image (images) or a line of
 * text, so flows that read a picked Zone file (import, avatar, wallpaper) work in the mock runtime. Tests attach real
 * fixtures with `window.__mockZoneFiles.put(path, base64)`.
 */

import type { FileEntry } from '../types'
import { classifyFileKind } from '../data/fileKinds'
import { invalidateMockPath } from '../data/mockReader'
import { mockAddEntry, mockEntryByPath } from './data'
import { mockHue, mockThumbnailUrl } from './thumbnail'

const contents = new Map<string, Blob>()

export function mockSetContent(id: string, blob: Blob) {
  contents.set(id, blob)
}

async function mockImage(name: string): Promise<Blob> {
  const canvas = new OffscreenCanvas(96, 96)
  const context = canvas.getContext('2d')
  if (!context) return new Blob([`mock content of ${name}\n`])
  context.fillStyle = `hsl(${mockHue(name)} 55% 62%)`
  context.fillRect(0, 0, 96, 96)
  return canvas.convertToBlob({ type: 'image/png' })
}

export async function mockReadContent(entry: FileEntry): Promise<Blob> {
  const stored = contents.get(entry.id)
  if (stored) return stored
  if (entry.kind === 'image') return mockImage(entry.name)
  return new Blob([`mock content of ${entry.path}\n`])
}

/** Places a file with these bytes at a DFS path, creating missing folders; an existing file keeps its entry. */
export function mockPutFile(path: string, blob: Blob): FileEntry {
  const segments = path.split('/').filter(Boolean)
  let parent = ''
  for (const segment of segments.slice(0, -1)) {
    const folder = `${parent}/${segment}`
    if (!mockEntryByPath(folder)) {
      mockAddEntry({ id: `mock-dir:${folder}`, name: segment, kind: 'folder', path: folder, modifiedAt: new Date().toISOString() })
      invalidateMockPath(parent || '/')
    }
    parent = folder
  }
  const name = segments[segments.length - 1]
  let entry = mockEntryByPath(path)
  if (!entry) {
    const kind = classifyFileKind(name, blob.type || undefined)
    entry = { id: `mock-file:${path}`, name, kind, path, sizeBytes: blob.size, modifiedAt: new Date().toISOString(), ...(kind === 'image' ? { thumbnailUrl: mockThumbnailUrl(name) } : {}) }
    mockAddEntry(entry)
  } else entry.sizeBytes = blob.size
  contents.set(entry.id, blob)
  invalidateMockPath(parent || '/')
  return entry
}

declare global {
  interface Window {
    /** Mock runtime only: put a file into the mock Zone (bytes as base64). */
    __mockZoneFiles?: { put(path: string, base64: string, mime?: string): void }
  }
}

export function exposeMockZoneFiles() {
  window.__mockZoneFiles = {
    put(path, base64, mime) {
      const bytes = Uint8Array.from(atob(base64), (ch) => ch.charCodeAt(0))
      mockPutFile(path, new Blob([bytes], mime ? { type: mime } : {}))
    },
  }
}
