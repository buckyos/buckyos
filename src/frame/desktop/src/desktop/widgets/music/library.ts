/**
 * The music player's Zone access: the tracks of a folder (its subfolders too, an album each), and a track as something
 * an `<audio>` element can play. Both go through the user's own session, like Files and Preview.
 */
import { installFileBrowserData } from '../../../app/filebrowser/data/install'
import { resolveReader } from '../../../app/filebrowser/data/readerRegistry'
import { normalizeUrl } from '../../../app/filebrowser/data/urls'
import { ensurePreviewProvider } from '../../../components/preview/provider'
import { normalizeCyfsPath } from '../../../components/preview/session'
import { isAudioFile, sortTracks } from './playlist'

/** A library folder of artists and albums is about this deep; more than this many tracks are not listed. */
const MAX_DEPTH = 3
export const MAX_TRACKS = 2000
const PAGE_SIZE = 200

/**
 * The audio files under `folder` (a DFS path such as `/home/Music`), in folder order. Throws when the folder itself
 * cannot be listed (gone, no access); a subfolder that cannot be is skipped.
 */
export async function listMusicFolder(folder: string, signal?: AbortSignal): Promise<string[]> {
  installFileBrowserData()
  const tracks: string[] = []
  const pending = [{ path: folder, depth: 0 }]
  while (pending.length && tracks.length < MAX_TRACKS) {
    const dir = pending.shift()!
    const reader = resolveReader(normalizeUrl(dir.path))
    try {
      for (let offset = 0; ; offset += PAGE_SIZE) {
        signal?.throwIfAborted()
        const page = await reader.list({ sortKey: 'name', sortDir: 'asc', foldersFirst: false, offset, limit: PAGE_SIZE })
        for (const { entry } of page.items) {
          if (entry.kind === 'folder') {
            if (dir.depth < MAX_DEPTH) pending.push({ path: entry.path, depth: dir.depth + 1 })
          } else if (isAudioFile(entry.name)) {
            tracks.push(entry.path)
          }
        }
        if (!page.items.length || !(page.hasMore || (page.totalCount ?? 0) > offset + page.items.length)) break
      }
    } catch (error) {
      if (dir.depth === 0 || signal?.aborted) throw error
    } finally {
      reader.dispose()
    }
  }
  return sortTracks(tracks).slice(0, MAX_TRACKS)
}

/** A playable URL for a track; `release` frees it once the player moves on. */
export async function trackSource(path: string, signal?: AbortSignal): Promise<{ url: string; release: () => void }> {
  const provider = await ensurePreviewProvider()
  const { readRef } = await provider.resolvePreviewSource({ kind: 'cyfs-path', path: normalizeCyfsPath(path) }, { signal })
  if (readRef.kind === 'url') return { url: readRef.url, release: () => {} }
  const url = URL.createObjectURL(readRef.blob)
  return { url, release: () => URL.revokeObjectURL(url) }
}
