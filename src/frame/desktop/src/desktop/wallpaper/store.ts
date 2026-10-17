/* ── The desktop picture of the signed-in account ──
 *
 * The account holds the wallpaper (`appearance.wallpaper` of the desktop state, see AccountAppearanceSync); the
 * picture itself is a file in the user's home, uploaded here. This store is what the page shows: it starts
 * from the copy cached in localStorage, so the desktop paints at once, and takes the account's value when it
 * arrives. Another tab of this browser follows through the `storage` event.
 */

import { useSyncExternalStore } from 'react'
import { fetchWallpaperWithToken, uploadWallpaper, wallpaperAssetUrl, wallpaperAssetUrlNow } from '../../api/desktop_state'
import type { DesktopWallpaper } from '../../models/ui'
import { prepareWallpaperImage } from './image'
import {
  normalizeAccountWallpaper,
  normalizePreferences,
  toDesktopWallpaper,
  type AccountWallpaper,
  type WallpaperPreferences,
} from './model'

const CACHE_KEY = 'buckyos.desktop.wallpaper.v2'
/** A server picture that fails to load, even with the token in a header, is tried again (say, offline). */
const RETRY_DELAYS_MS = [2_000, 8_000, 30_000]

export interface WallpaperSnapshot {
  /** The account's wallpaper (as last known here). */
  account: AccountWallpaper
  /** Where the picture is drawn from; null when there is none or while it is looked up. */
  url: string | null
  /** What the desktop draws; null keeps the built-in background. */
  wallpaper: DesktopWallpaper | null
}

function readCache(): AccountWallpaper {
  try {
    return normalizeAccountWallpaper(JSON.parse(window.localStorage.getItem(CACHE_KEY) ?? 'null'))
  } catch {
    return normalizeAccountWallpaper(null)
  }
}

function writeCache(account: AccountWallpaper) {
  try {
    window.localStorage.setItem(CACHE_KEY, JSON.stringify(account))
  } catch {
    // Not cached (private window); the account still has it.
  }
}

function derive(account: AccountWallpaper, url: string | null): DesktopWallpaper | null {
  const { image } = account
  return toDesktopWallpaper(image && url ? { url, width: image.width, height: image.height, name: image.name } : null, account)
}

export class WallpaperStore {
  private snapshot: WallpaperSnapshot
  private listeners = new Set<() => void>()
  private started = false
  /** Bumped by every picture change, so a slower earlier lookup or upload never lands after it. */
  private imageSeq = 0
  /** `blob:` URLs this store made from a token fetch; revoked once the picture moves on. */
  private readonly ownBlobUrls = new Set<string>()

  constructor() {
    const account = readCache()
    const url = account.image ? wallpaperAssetUrlNow(account.image.asset) : null
    this.snapshot = { account, url, wallpaper: derive(account, url) }
  }

  subscribe = (listener: () => void): (() => void) => {
    this.start()
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getSnapshot = (): WallpaperSnapshot => this.snapshot

  private start() {
    if (this.started) return
    this.started = true
    window.addEventListener('storage', (event) => {
      if (event.key === CACHE_KEY || event.key === null) this.show(readCache(), false)
    })
    if (this.snapshot.account.image && !this.snapshot.url) void this.lookUp(this.snapshot.account)
    else if (this.snapshot.url) this.watchLoad(this.snapshot.url)
  }

  private patch(account: AccountWallpaper, url: string | null) {
    const previousUrl = this.snapshot.url
    const wallpaperChanged =
      url !== previousUrl || JSON.stringify(account) !== JSON.stringify(this.snapshot.account)
    if (!wallpaperChanged) return
    this.setUrl(account, url)
    if (url && url !== previousUrl && this.started) this.watchLoad(url)
  }

  private setUrl(account: AccountWallpaper, url: string | null) {
    const previousUrl = this.snapshot.url
    this.snapshot = { account, url, wallpaper: derive(account, url) }
    this.listeners.forEach((listener) => listener())
    if (previousUrl && previousUrl !== url && this.ownBlobUrls.delete(previousUrl)) URL.revokeObjectURL(previousUrl)
  }

  /**
   * CSS gives no load error, so a probe watches a server picture. A refused plain load (no usable session
   * cookie) is fetched again with the token in a header; if that fails too it is retried later.
   */
  private watchLoad(url: string, attempt = 0) {
    if (url.startsWith('blob:') || typeof Image === 'undefined') return
    const asset = this.snapshot.account.image?.asset
    const current = () => this.snapshot.url === url && this.snapshot.account.image?.asset === asset
    const probe = new Image()
    probe.onerror = () => {
      void fetchWallpaperWithToken(url).then((blobUrl) => {
        if (!current()) {
          if (blobUrl) URL.revokeObjectURL(blobUrl)
          return
        }
        if (blobUrl) {
          this.ownBlobUrls.add(blobUrl)
          this.setUrl(this.snapshot.account, blobUrl)
          return
        }
        if (attempt >= RETRY_DELAYS_MS.length) return
        setTimeout(() => {
          if (!current()) return
          const retry = `${url.split('?')[0]}?attempt=${attempt + 1}`
          this.setUrl(this.snapshot.account, retry)
          this.watchLoad(retry, attempt + 1)
        }, RETRY_DELAYS_MS[attempt])
      })
    }
    probe.src = url
  }

  private async lookUp(account: AccountWallpaper) {
    const seq = ++this.imageSeq
    const url = account.image ? await wallpaperAssetUrl(account.image.asset).catch(() => null) : null
    if (seq === this.imageSeq) this.patch(this.snapshot.account, url)
  }

  /** Shows `account`; a new picture is looked up. `cache: false` when it came from the cache itself. */
  private show(account: AccountWallpaper, cache = true) {
    const sameImage = account.image?.asset === this.snapshot.account.image?.asset
    if (sameImage) {
      this.patch(account, this.snapshot.url)
    } else {
      this.imageSeq++
      const url = account.image ? wallpaperAssetUrlNow(account.image.asset) : null
      this.patch(account, url)
      if (account.image && !url) void this.lookUp(account)
    }
    if (cache) writeCache(account)
  }

  /** The account's value, from the sync. */
  applyAccount(raw: unknown) {
    this.show(normalizeAccountWallpaper(raw))
  }

  /**
   * Uploads the picture to the user's home and shows it. Throws `WallpaperImageError` when the file is not a
   * usable picture, `DesktopApiError` when the upload fails; the current wallpaper stays then.
   */
  async setImage(file: File): Promise<void> {
    const seq = ++this.imageSeq
    const prepared = await prepareWallpaperImage(file)
    const asset = await uploadWallpaper(prepared.blob)
    const url = await wallpaperAssetUrl(asset)
    if (seq !== this.imageSeq) return
    // A new picture starts centred; the fit is the user's choice and stays.
    const account: AccountWallpaper = {
      ...this.snapshot.account,
      focusX: 50,
      focusY: 50,
      image: { asset, width: prepared.width, height: prepared.height, name: prepared.name },
    }
    this.patch(account, url)
    writeCache(account)
  }

  /** Back to the built-in background; the preferences stay for the next picture. */
  clearImage() {
    this.imageSeq++
    this.show({ ...this.snapshot.account, image: null })
  }

  /** `persist: false` previews a change (a drag in progress) without caching it. */
  setPreferences(patch: Partial<WallpaperPreferences>, { persist = true }: { persist?: boolean } = {}) {
    const account = { ...this.snapshot.account, ...normalizePreferences({ ...this.snapshot.account, ...patch }) }
    this.patch(account, this.snapshot.url)
    if (persist) writeCache(account)
  }
}

/** Shared by the Settings app, the desktop shell and the account sync. */
export const wallpaperStore = new WallpaperStore()

export function useWallpaperSnapshot(): WallpaperSnapshot {
  return useSyncExternalStore(wallpaperStore.subscribe, wallpaperStore.getSnapshot)
}

/** Whether a picture is set; re-renders only when that flips, not on every step of a preview drag. */
export function useHasWallpaperPicture(): boolean {
  return useSyncExternalStore(wallpaperStore.subscribe, () => wallpaperStore.getSnapshot().wallpaper !== null)
}
