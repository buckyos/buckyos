import { useMemo } from 'react'
import type { ContentSelector, LocalFile, ZoneContent } from 'buckyos/content'
import { installFileBrowserData } from '../../app/filebrowser/data/install'
import { useSelectContent } from '..'
import { readZoneFiles, readZoneFolder } from './zoneFiles'

/**
 * Selectors for a native `<input accept>` string: MIME patterns and extensions become two selectors, since a selector
 * with a `mime` does not fall back to its `ext`.
 */
export function acceptToSelectors(accept?: string): ContentSelector[] {
  const mime: string[] = []
  const ext: string[] = []
  for (const raw of accept?.split(',') ?? []) {
    const token = raw.trim().toLowerCase()
    if (token.startsWith('.') && token.length > 1) ext.push(token)
    else if (/^[\w.+*-]+\/[\w.+*-]+$/.test(token)) mime.push(token)
  }
  return [...(mime.length ? [{ mime }] : []), ...(ext.length ? [{ ext }] : [])]
}

export interface PickFilesOptions {
  /** As for `<input accept>`. */
  accept?: string
  multiple?: boolean
  /** One folder: every file below it, each with `webkitRelativePath` starting with the folder's name. */
  folder?: boolean
  title?: string
  signal?: AbortSignal
}

export type PickFiles = (options?: PickFilesOptions) => Promise<File[] | null>

// A File that crossed documents (path C) keeps its bytes but not its relative path.
function withRelativePath(file: File, relativePath: string): File {
  if (file.webkitRelativePath !== relativePath) Object.defineProperty(file, 'webkitRelativePath', { value: relativePath, configurable: true })
  return file
}

/**
 * Files chosen in the system file picker, from the Zone first or from this device (the picker's "This device"), for
 * callers that need the bytes (parse, import, upload to their own store). Zone files are read with the user's own
 * session into `File` objects, so both sources go through the caller's existing path. Resolves with null when the
 * user cancels. `callerId` keys the folder the picker starts in next time.
 */
export function usePickFiles(callerId: string): PickFiles {
  const select = useSelectContent()
  return useMemo(() => async ({ accept, multiple = false, folder = false, title, signal } = {}) => {
    const result = await select(folder
      ? { mode: 'folder', allowLocal: true, ...(title ? { title } : {}) }
      : { mode: 'open', multiple, accept: acceptToSelectors(accept), allowLocal: true, ...(title ? { title } : {}) },
    { id: callerId })
    if (result.action !== 'selected' || !result.items.length) return null
    const first = result.items[0]
    if (first.origin === 'local') {
      return first.kind === 'folder'
        ? first.files.map(entry => withRelativePath(entry.file, entry.relativePath))
        : (result.items as LocalFile[]).map(item => item.file)
    }
    installFileBrowserData()
    return folder ? readZoneFolder(first, signal) : readZoneFiles(result.items as ZoneContent[], signal)
  }, [select, callerId])
}
