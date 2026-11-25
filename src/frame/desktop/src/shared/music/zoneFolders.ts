/* ── The music of a Zone, over a channel ──
 *
 * For a caller that is not a page of the Zone (the BuckyOS APP client): folders to choose from, the tracks under one,
 * and where a track is read. It speaks NFSP (`/nfs/v1/*` on the Zone's root) as Files and Preview do, without their
 * caches and watches.
 */

import { NfspClient, NfspError, type Entry, type Listing } from 'buckyos/nfsp'
import type { ZoneChannel } from '../channel.ts'
import { isAudioFile, sortTracks } from './playlist.ts'

/** A library folder of artists and albums is about this deep; more than this many tracks are not listed. */
const MAX_DEPTH = 3
const MAX_TRACKS = 2000
const PAGE_SIZE = 200
const MAX_FOLDERS = 1000

export interface ZoneFolder {
  name: string
  /** A path of the Zone's file tree, such as `/home/alice/Music`. */
  path: string
}

/** `/home/a//b/` and `cyfs:///home/a/b` as `/home/a/b`. */
export function folderPath(input: string): string {
  const path = input.trim().replace(/^cyfs:\/\//, '')
  return `/${path.split('/').filter(Boolean).join('/')}`
}

export function parentFolder(path: string): string | null {
  const parts = folderPath(path).split('/').filter(Boolean)
  return parts.length === 0 ? null : `/${parts.slice(0, -1).join('/')}`
}

const childPath = (folder: string, entry: Entry): string =>
  entry.canonical_path?.startsWith('cyfs:///') ? folderPath(entry.canonical_path) : folderPath(`${folder}/${entry.name}`)

const usable = (entry: Entry) => entry.target.target_state === undefined || entry.target.target_state === 'ok'

export class ZoneFolders {
  private readonly client: NfspClient
  private hello: Promise<string> | null = null

  constructor(channel: ZoneChannel) {
    this.client = new NfspClient({
      baseUrl: '',
      fetch: ((input: RequestInfo | URL, init?: RequestInit) => channel.fetch(String(input), init)) as typeof fetch,
    })
  }

  /** The user this session acts as. Nobody is an error: an anonymous session lists public folders only, as if the rest were not there. */
  private session(): Promise<string> {
    this.hello ??= this.client.hello().then((result) => {
      const user = result.identity?.user_id
      if (!user) throw new NfspError({ code: 'PERMISSION_DENIED', message: 'not signed in' }, 403)
      return user
    }).catch((error: unknown) => {
      this.hello = null
      throw error
    })
    return this.hello
  }

  /** Once more with a new session when the server has forgotten this one. */
  private async run<T>(work: () => Promise<T>): Promise<T> {
    await this.session()
    try {
      return await work()
    } catch (error) {
      if (!(error instanceof NfspError) || error.code !== 'PERMISSION_DENIED' || !/session/i.test(error.message)) throw error
      this.hello = null
      await this.session()
      return work()
    }
  }

  /** The signed-in user's own folder. */
  async home(): Promise<string> {
    return `/home/${await this.session()}`
  }

  private async *pages(folder: string, signal?: AbortSignal): AsyncGenerator<Listing> {
    let cursor: string | undefined
    do {
      signal?.throwIfAborted()
      const page: Listing = await this.run(() =>
        this.client.list(folder, { cursor, limit: Math.min(PAGE_SIZE, this.client.limits?.max_list ?? PAGE_SIZE), order: 'name' }, ['base']),
      )
      signal?.throwIfAborted()
      yield page
      cursor = page.next_cursor || undefined
    } while (cursor)
  }

  /** The folders in a folder, by name. Hidden ones (a leading dot) are left out. */
  async folders(path: string, signal?: AbortSignal): Promise<ZoneFolder[]> {
    const folder = folderPath(path)
    const found: ZoneFolder[] = []
    for await (const page of this.pages(folder, signal)) {
      for (const entry of page.entries) {
        if (entry.target.kind === 'dir' && usable(entry) && !entry.name.startsWith('.')) found.push({ name: entry.name, path: childPath(folder, entry) })
      }
      if (found.length >= MAX_FOLDERS) break
    }
    const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
    return found.sort((a, b) => collator.compare(a.name, b.name))
  }

  /**
   * The audio files under a folder (its subfolders too, an album each), in folder order. Rejects when the folder itself
   * cannot be listed or the Zone stops answering; a subfolder the Zone does not list is skipped.
   */
  async tracks(path: string, signal?: AbortSignal): Promise<string[]> {
    const tracks: string[] = []
    const pending = [{ path: folderPath(path), depth: 0 }]
    while (pending.length && tracks.length < MAX_TRACKS) {
      const folder = pending.shift()!
      try {
        for await (const page of this.pages(folder.path, signal)) {
          for (const entry of page.entries) {
            if (!usable(entry)) continue
            if (entry.target.kind === 'dir') {
              if (folder.depth < MAX_DEPTH) pending.push({ path: childPath(folder.path, entry), depth: folder.depth + 1 })
            } else if (entry.target.kind === 'file' && isAudioFile(entry.name)) {
              tracks.push(childPath(folder.path, entry))
            }
          }
          if (tracks.length >= MAX_TRACKS) break
        }
      } catch (error) {
        if (folder.depth === 0 || signal?.aborted || !(error instanceof NfspError)) throw error
      }
    }
    return sortTracks(tracks).slice(0, MAX_TRACKS)
  }

  /** Where a file's bytes are read: path and query on the Zone's root. */
  async readPath(path: string): Promise<string> {
    const info = await this.run(() => this.client.resolve(folderPath(path), ['base', 'ident', 'access']))
    const access = info.access_urls?.find((url) => url.kind === 'read')?.url
    const id = info.node_id ?? (info.ref.type === 'live' ? info.ref.node_id : info.ref.obj_id)
    const url = new URL(access ?? this.client.readUrl(id), 'https://zone.invalid/')
    return `${url.pathname}${url.search}`
  }
}
