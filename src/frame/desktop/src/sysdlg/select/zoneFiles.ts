import { mediaTypeFromExtension, extensionOf, type ZoneContent } from 'buckyos/content'
import { folderOps } from '../../app/filebrowser/data/folderOps'
import { resolveReader } from '../../app/filebrowser/data/readerRegistry'
import { normalizeUrl } from '../../app/filebrowser/data/urls'

const MAX_FOLDER_FILES = 2000
const dfsPath = (item: ZoneContent): string => {
  if (item.source.kind !== 'cyfs-path') throw new Error('Only a Zone path can be read')
  return item.source.path.slice('cyfs://'.length) || '/'
}
async function fileAt(path: string, name: string, relativePath?: string): Promise<File> {
  const blob = await folderOps().readContent(path)
  const file = new File([blob], name, { type: mediaTypeFromExtension(extensionOf(name)) ?? blob.type })
  // Read-only on File, but callers that import a folder (ndm) take the tree from it.
  if (relativePath) Object.defineProperty(file, 'webkitRelativePath', { value: relativePath })
  return file
}

/**
 * The content of Zone files a caller got from the system file picker, as `File` objects — for callers whose import
 * path takes files (AI Workspace). The bytes are read with the user's own session, like any other read of the path.
 */
export async function readZoneFiles(items: ZoneContent[], signal?: AbortSignal): Promise<File[]> {
  const files: File[] = []
  for (const item of items) {
    signal?.throwIfAborted()
    const path = dfsPath(item)
    files.push(await fileAt(path, item.hints.name ?? path.slice(path.lastIndexOf('/') + 1)))
  }
  return files
}

/** Every file below a Zone folder, each with `webkitRelativePath` starting with the folder's name. Empty folders are skipped. */
export async function readZoneFolder(item: ZoneContent, signal?: AbortSignal): Promise<File[]> {
  const root = dfsPath(item)
  const rootName = root.slice(root.lastIndexOf('/') + 1) || 'folder'
  const files: File[] = []
  const pending = [{ path: root, relative: rootName }]
  while (pending.length) {
    const dir = pending.shift()!
    const reader = resolveReader(normalizeUrl(dir.path))
    try {
      for (let offset = 0; ; offset += 200) {
        signal?.throwIfAborted()
        const page = await reader.list({ sortKey: 'name', sortDir: 'asc', foldersFirst: false, offset, limit: 200 })
        for (const { entry } of page.items) {
          const relative = `${dir.relative}/${entry.name}`
          if (entry.kind === 'folder') pending.push({ path: entry.path, relative })
          else {
            if (files.length >= MAX_FOLDER_FILES) throw new Error(`A folder with more than ${MAX_FOLDER_FILES} files cannot be imported at once`)
            signal?.throwIfAborted()
            files.push(await fileAt(entry.path, entry.name, relative))
          }
        }
        if (!page.items.length || !(page.hasMore || (page.totalCount ?? 0) > offset + page.items.length)) break
      }
    } finally { reader.dispose() }
  }
  return files
}
