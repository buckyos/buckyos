/**
 * NFSP `host` realm: the computer behind a Desktop Lite node, through a sudo
 * session (Bearer = the window's BuckyOS sudo token, `aud: nfs-server`).
 *
 * The client keeps no cache outside memory and opens no watch; nothing it
 * learns outlives the window's grant. Server task paths name the computer as
 * `/@host/...`; the UI uses `device://<session>/...`.
 */

import type { CacheStore } from '../../../../api/nfs_browser_client'
import { NfsBrowserClient } from '../../../../api/nfs_browser_client'
import { NfspClient, type Listing, type LocatorLike } from '../../../../api/nfsp_client'
import { registerPreviewSourceResolver } from '../../../../components/preview/hostSources'
import { createNfspPreviewProvider } from '../../../../components/preview/nfspProvider'
import { extensionRefField, type ContentRef } from '../../../../components/preview/types'
import type { FileEntry } from '../../types'
import type { FileItem } from '../FolderReader'
import {
  HOST_SOURCE_KIND,
  isSudoRequired,
  lockHost,
  registerHostBackend,
  sudoRequiredError,
  type HostConnection,
  type HostGrant,
} from '../hostRealm'
import { DEVICE_SCHEME, deviceUrl, parseDeviceUrl } from '../urls'
import { effectiveListLimit, nfspBaseUrl } from './client'
import { createNfspCopy } from './copy'
import { nfspToError, nfspToUiError } from './errors'
import { createNfspFolderOps } from './folderOps'
import { uploadInto } from './transfers'
import { notifyDfsPath } from './invalidation'
import { mapEntryToItem } from './mapping'
import { NfspContainerReader } from './readers'

const SERVER_HOST_ROOT = '/@host'
const TREE_PAGE = 200

class MemoryCacheStore implements CacheStore {
  private map = new Map<string, string>()
  async get(key: string) { return this.map.get(key) ?? null }
  async set(key: string, value: string) { this.map.set(key, value) }
  async delete(key: string) { this.map.delete(key) }
  async deletePrefix(prefix: string) { for (const key of [...this.map.keys()]) if (key.startsWith(prefix)) this.map.delete(key) }
  async keys(prefix: string) { return [...this.map.keys()].filter((key) => key.startsWith(prefix)) }
}

/** Each unlocked window's sudo-session client, for its Preview sources. */
const clients = new Map<string, NfsBrowserClient>()

const hostPathOf = (url: string) => parseDeviceUrl(url)?.path ?? '/'
/** A UI path (DFS path or device URL) as an NFSP locator. */
const locatorOf = (path: string): LocatorLike => (path.startsWith(DEVICE_SCHEME) ? { realm: 'host', path: hostPathOf(path) } : path)

const FOLDER_CAPABILITIES = {
  kind: 'folder' as const,
  availability: 'loading' as const,
  acceptsContent: false,
  acceptsReferences: false,
  removal: null,
  canReorder: false,
  sortKeys: ['name' as const, 'size' as const, 'modified' as const],
  sortDirs: ['asc' as const],
  defaultSortKey: 'name' as const,
}

class NfspHostReader extends NfspContainerReader {
  private readonly session: string
  private readonly hostClient: NfsBrowserClient
  private readonly ready: () => Promise<unknown>

  constructor(url: string, session: string, client: NfsBrowserClient, ready: () => Promise<unknown>) {
    super(url, FOLDER_CAPABILITIES)
    // Same-client writes notify the folder's UI path (the device URL).
    this.dfsPath = url
    this.session = session
    this.hostClient = client
    this.ready = ready
  }

  protected override client() {
    return this.hostClient
  }

  protected override ensure() {
    return this.ready()
  }

  protected async locator(): Promise<LocatorLike> {
    return { realm: 'host', path: hostPathOf(this.url) }
  }

  protected mapListing(listing: Listing): FileItem[] {
    return listing.entries.map((entry) => {
      const item = mapEntryToItem(entry, { containerDfsPath: deviceUrl(this.session, hostPathOf(this.url)) })
      return { ...item, entry: { ...item.entry, publicUrl: undefined } }
    })
  }
}

async function connect(session: string, grant: HostGrant): Promise<HostConnection> {
  const raw = new NfspClient({ baseUrl: nfspBaseUrl(), sessionToken: async () => grant.token })
  const client = new NfsBrowserClient({ baseUrl: nfspBaseUrl(), client: raw, store: new MemoryCacheStore(), autoWatch: false })
  let hello = client.hello()
  try {
    const result = await hello
    if (!result.realms.some((realm) => realm.id === 'host')) throw sudoRequiredError()
  } catch (err) {
    if (isSudoRequired(err)) throw err
    const ui = nfspToUiError(err)
    throw ui.code === 'UNAUTHENTICATED' || ui.code === 'PERMISSION_DENIED' ? sudoRequiredError() : ui
  }
  clients.set(session, client)
  const ready = () => hello.catch((err: unknown) => {
    hello = client.hello()
    throw err
  })
  /** Server says the grant is gone: the window locks; callers see SUDO_REQUIRED. */
  const guard = async <T>(op: () => Promise<T>): Promise<T> => {
    try {
      return await op()
    } catch (err) {
      const ui = nfspToUiError(err)
      if (ui.code === 'SUDO_REQUIRED' || ui.code === 'UNAUTHENTICATED' || isSudoRequired(err)) {
        lockHost(session)
        throw sudoRequiredError()
      }
      throw err
    }
  }
  const uiPath = (path: string) =>
    path === SERVER_HOST_ROOT || path.startsWith(`${SERVER_HOST_ROOT}/`) ? deviceUrl(session, path.slice(SERVER_HOST_ROOT.length) || '/') : path
  const serverPath = (path: string) => (path.startsWith(DEVICE_SCHEME) ? `${SERVER_HOST_ROOT}${hostPathOf(path) === '/' ? '' : hostPathOf(path)}` : path)
  const copy = createNfspCopy({
    call: (method, args) => guard(() => client.copyCall(method, args)),
    statDir: (path) => guard(() => client.stat(locatorOf(path), { cache: 'no-cache' })),
    uiPath,
    serverPath,
    touched: async (path) => {
      await client.invalidateContainer(locatorOf(path)).catch(() => undefined)
      notifyDfsPath(path)
    },
  })
  const ops = createNfspFolderOps({
    client: () => client,
    ensure: () => guard(ready),
    at: locatorOf,
    copy,
    downloadUrl: (entry: FileEntry) => {
      if (!entry.readUrl) return null
      const url = new URL(entry.readUrl, window.location.href)
      url.searchParams.set('download', '1')
      url.searchParams.set('name', entry.name)
      return url.toString()
    },
  }, { get supportsCopy() { return true }, copyUnavailableReason: '' })
  return {
    reader: (url) => new NfspHostReader(url, session, client, () => guard(ready)),
    ops,
    upload: (task, controls) => guard(async () => {
      await ready()
      return uploadInto(task, controls, { client, token: async () => grant.token, at: locatorOf(task.targetUrl), parentPath: task.targetUrl })
    }),
    async children(url, cursor) {
      const path = hostPathOf(url)
      const listing = await guard(async () => {
        await ready()
        return client.list({ realm: 'host', path }, { limit: effectiveListLimit(TREE_PAGE), order: 'name', filter: { kind: ['dir'] }, cursor }, ['base', 'access'], { cache: 'no-cache' })
      }).catch((err: unknown) => {
        throw isSudoRequired(err) ? err : nfspToError(err)
      })
      if (!listing) return { folders: [] }
      return {
        folders: listing.entries
          .filter((entry) => entry.target.kind === 'dir')
          .map((entry) => ({ name: entry.name, url: deviceUrl(session, path === '/' ? `/${entry.name}` : `${path}/${entry.name}`), locked: entry.target.access_level === 'none' })),
        nextCursor: listing.truncated && listing.next_cursor ? listing.next_cursor : undefined,
      }
    },
    close() {
      if (clients.get(session) === client) clients.delete(session)
      void raw.bye().catch(() => undefined)
    },
  }
}

/** Preview / thumbnails of the computer's files: resolved and converted through the window's sudo session. */
function registerHostPreviewSources() {
  const providers = new WeakMap<NfsBrowserClient, ReturnType<typeof createNfspPreviewProvider>>()
  const providerFor = (source: ContentRef) => {
    const parts = parseDeviceUrl(extensionRefField(source, 'url') ?? '')
    const client = parts ? clients.get(parts.session) : undefined
    if (!client) throw sudoRequiredError()
    let provider = providers.get(client)
    if (!provider) {
      provider = createNfspPreviewProvider({
        client: () => client,
        ensure: async () => undefined,
        locate: (ref) => locatorOf(extensionRefField(ref, 'url') ?? ''),
      })
      providers.set(client, provider)
    }
    return provider
  }
  registerPreviewSourceResolver(HOST_SOURCE_KIND, {
    resolvePreviewSource: async (source, opts) => providerFor(source).resolvePreviewSource(source, opts),
    ensurePreviewWork: async (request) => providerFor(request.source.originalSource).ensurePreviewWork(request),
  })
}

export function registerNfspHostBackend() {
  registerHostBackend({ connect })
  registerHostPreviewSources()
}

