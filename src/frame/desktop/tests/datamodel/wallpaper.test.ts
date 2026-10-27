import {
  coverOverhang,
  defaultAccountWallpaper,
  defaultWallpaperPreferences,
  normalizeAccountWallpaper,
  normalizePreferences,
  panFocus,
  toDesktopWallpaper,
} from '../../src/desktop/wallpaper/model.ts'
import { planSync, readAppearance, type AccountAppearance } from '../../src/desktop/appearance/model.ts'
import { AppearanceSync, type AppearanceSyncIo } from '../../src/desktop/appearance/sync.ts'
import { WallpaperStore, type WallpaperIo } from '../../src/desktop/wallpaper/store.ts'
import { wallpapersToPrune } from '../../src/api/desktop_state.ts'
import {
  MAX_WALLPAPER_EDGE,
  WallpaperImageError,
  checkWallpaperFile,
  keepsOriginal,
  scaledSize,
} from '../../src/desktop/wallpaper/image.ts'
import { imageLayerStyle } from '../../src/desktop/wallpaper/style.ts'
import type { DesktopWallpaper } from '../../src/models/ui.ts'

function assertEquals<T>(actual: T, expected: T, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
}

function assertThrowsReason(run: () => void, reason: string, message: string) {
  try {
    run()
  } catch (error) {
    if (error instanceof WallpaperImageError && error.reason === reason) return
    throw new Error(`${message}: threw ${String(error)}`)
  }
  throw new Error(`${message}: did not throw`)
}

const picture = { url: 'blob:http://desk/1', width: 3000, height: 2000, name: 'a.jpg' }

Deno.test('stored preferences are clamped, and anything unusable falls back to the default', () => {
  assertEquals(normalizePreferences(null), defaultWallpaperPreferences, 'nothing stored')
  assertEquals(normalizePreferences({ fit: 'zoom', tileScale: 'big', focusX: Number.NaN }), defaultWallpaperPreferences, 'garbage')
  assertEquals(
    normalizePreferences({ fit: 'tile', tileScale: 1000, focusX: -20, focusY: 140 }),
    { fit: 'tile', tileScale: 400, focusX: 0, focusY: 100 },
    'out of range',
  )
  assertEquals(normalizePreferences({ fit: 'panorama', tileScale: 37.6 }).tileScale, 38, 'tile scale is whole percent')
})

Deno.test('the desktop draws the picture with the chosen fit, or the built-in background without one', () => {
  assertEquals(toDesktopWallpaper(null, defaultWallpaperPreferences), null, 'no picture')
  assertEquals(
    toDesktopWallpaper(picture, { fit: 'tile', tileScale: 50, focusX: 20, focusY: 80 }),
    { mode: 'tile', imageUrl: picture.url, imageWidth: 3000, imageHeight: 2000, tileScale: 50, focusX: 20, focusY: 80 },
    'picture',
  )
})

Deno.test('dragging a covering picture moves the focus only along the cropped axis', () => {
  // A 3:2 picture on a portrait phone frame overhangs sideways only.
  const overhang = coverOverhang(100, 200, 3000, 2000)
  assertEquals(overhang, { x: 200, y: 0 }, 'phone overhang')
  assertEquals(panFocus({ x: 50, y: 50 }, { dx: 50, dy: 30 }, overhang), { x: 25, y: 50 }, 'drag right shows the left part')
  assertEquals(panFocus({ x: 10, y: 50 }, { dx: 100, dy: 0 }, overhang), { x: 0, y: 50 }, 'clamped at the edge')
  assertEquals(panFocus({ x: 50, y: 50 }, { dx: -400, dy: 0 }, overhang), { x: 100, y: 50 }, 'clamped at the other edge')
  assertEquals(coverOverhang(0, 100, 10, 10), { x: 0, y: 0 }, 'unmeasured frame')
})

Deno.test('picked files: what is kept as it is, what is scaled, what is refused', () => {
  assertThrowsReason(() => checkWallpaperFile({ type: 'text/plain', size: 10 }), 'type', 'text')
  assertThrowsReason(() => checkWallpaperFile({ type: 'image/svg+xml', size: 10 }), 'type', 'svg')
  assertThrowsReason(() => checkWallpaperFile({ type: 'image/jpeg', size: 41 * 1024 * 1024 }), 'too_large', 'huge')
  checkWallpaperFile({ type: '', size: 10 })
  checkWallpaperFile({ type: 'image/heic', size: 10 })

  assertEquals(keepsOriginal({ type: 'image/gif', size: 2_000_000 }, 800, 600), true, 'small GIF stays animated')
  assertEquals(keepsOriginal({ type: 'image/jpeg', size: 2_000_000 }, 6000, 4000), false, 'oversized photo')
  assertEquals(keepsOriginal({ type: 'image/bmp', size: 200_000 }, 800, 600), false, 'BMP is re-encoded')
  assertEquals(scaledSize(8192, 4096), { width: MAX_WALLPAPER_EDGE, height: 2048 }, 'long side capped')
  assertEquals(scaledSize(1000, 500), { width: 1000, height: 500 }, 'never enlarged')
})

Deno.test('center and tile draw the picture pixel for pixel; the preview scales that down', () => {
  const tile: DesktopWallpaper = { mode: 'tile', imageUrl: 'blob:x', imageWidth: 200, imageHeight: 100, tileScale: 50 }
  assertEquals(imageLayerStyle(tile, { scale: 1, pixelRatio: 2 }).backgroundSize, '50px 25px', 'tile on a 2x screen')
  assertEquals(imageLayerStyle(tile, { scale: 0.5, pixelRatio: 1 }).backgroundSize, '50px 25px', 'tile in a half-size preview')
  const center: DesktopWallpaper = { ...tile, mode: 'center' }
  assertEquals(imageLayerStyle(center, { scale: 1, pixelRatio: 1 }).backgroundSize, '200px 100px', 'center ignores the tile scale')
  const fill: DesktopWallpaper = { mode: 'fill', imageUrl: 'blob:x', focusX: 30, focusY: 70 }
  assertEquals(imageLayerStyle(fill, { scale: 1, pixelRatio: 1 }).backgroundPosition, '30% 70%', 'fill keeps the focus')
  assertEquals(imageLayerStyle({ mode: 'fit', imageUrl: 'blob:x' }, { scale: 1, pixelRatio: 1 }).backgroundSize, 'contain', 'fit')
  assertEquals(
    imageLayerStyle({ mode: 'fill', imageUrl: 'data:image/svg+xml,<svg a="1"/>' }, { scale: 1, pixelRatio: 1 }).backgroundImage,
    'url("data:image/svg+xml,<svg a=%221%22/>")',
    'quotes in the URL cannot end the CSS string',
  )
})

// ── Appearance that follows the account ──

const KEY = `${'a'.repeat(32)}.jpg`
const REF = `obj://bucket/user/control-panel/alice/wallpapers/${KEY}`
const local: AccountAppearance = {
  theme: 'light',
  locale: 'zh-CN',
  iconSize: 'medium',
  window: { titleBarOpacity: 100, backgroundOpacity: 100 },
  wallpaper: { ...defaultAccountWallpaper, image: { ref: REF, width: 1200, height: 800, name: 'a.jpg' } },
}

Deno.test('an account wallpaper names a picture in its wallpapers bucket by content hash; anything else is no picture', () => {
  assertEquals(normalizeAccountWallpaper({ fit: 'tile', image: { ref: REF, width: 10, height: 20, name: 'x' } }).image?.ref, REF, 'kept')
  assertEquals(normalizeAccountWallpaper({ image: { ref: KEY, width: 10, height: 20 } }).image, null, 'a bare key refused')
  assertEquals(
    normalizeAccountWallpaper({ image: { ref: `obj://bucket/user/photos/alice/library/${KEY}`, width: 10, height: 20 } }).image,
    null,
    'another bucket refused',
  )
  assertEquals(normalizeAccountWallpaper({ image: { ref: REF, width: 0, height: 20 } }).image, null, 'no size')
  assertEquals(readAppearance({ theme: 'blue', locale: 'xx', iconSize: 'huge', window: { titleBarOpacity: 300 } }), {}, 'invalid fields dropped')
  assertEquals(readAppearance({ theme: 'dark', iconSize: 'large' }), { theme: 'dark', iconSize: 'large' }, 'valid fields kept')
})

Deno.test('sync: the account wins, a setting it lacks is seeded from this device, a change made here is kept', () => {
  const plan = planSync({ remote: { theme: 'dark', iconSize: 'medium' }, local, dirty: new Set(['locale']), foreignWallpaper: false })
  assertEquals(plan.apply, { theme: 'dark' }, 'account value shown here')
  assertEquals(Object.keys(plan.push).sort(), ['locale', 'wallpaper', 'window'], 'changed here, or missing on the account')
  assertEquals(plan.synced.theme, 'dark', 'synced')

  const foreign = planSync({ remote: {}, local, dirty: new Set(), foreignWallpaper: true })
  assertEquals(foreign.apply.wallpaper, defaultAccountWallpaper, "another account's picture is not shown")
  assertEquals(foreign.push.wallpaper, defaultAccountWallpaper, 'nor saved to this account')
})

class MemoryStorage {
  private items = new Map<string, string>()
  getItem(key: string) { return this.items.get(key) ?? null }
  setItem(key: string, value: string) { this.items.set(key, value) }
  removeItem(key: string) { this.items.delete(key) }
}

function fakeAccount(stored: Record<string, unknown> | null) {
  const writes: [string, unknown][] = []
  let state = stored
  const io: AppearanceSyncIo = {
    read: () => Promise.resolve(state),
    write: (field, value) => {
      writes.push([field, value])
      state = { ...(state ?? {}), [field]: value }
      return Promise.resolve()
    },
  }
  return { io, writes, current: () => state }
}

Deno.test('sync engine: shows the account, saves what changes here, and seeds a new account', async () => {
  ;(globalThis as { window?: unknown }).window = { localStorage: new MemoryStorage() }

  const account = fakeAccount({ theme: 'dark', locale: 'zh-CN', iconSize: 'medium', window: local.window, wallpaper: local.wallpaper })
  const shown: Partial<AccountAppearance>[] = []
  let page = local
  const sync = new AppearanceSync('alice', page, account.io, (patch) => {
    shown.push(patch)
    page = { ...page, ...patch }
  })
  await sync.refresh(true)
  assertEquals(shown, [{ theme: 'dark' }], 'account theme shown')
  sync.update(page)
  await sync.flush()
  assertEquals(account.writes, [], 'showing the account value is not a change to save')

  page = { ...page, iconSize: 'large' }
  sync.update(page)
  await sync.flush()
  assertEquals(account.writes, [['iconSize', 'large']], 'only the changed setting is saved')
  sync.stop()

  // Bob signs in on this browser: his new account starts without Alice's picture, with this device's other settings.
  const bob = fakeAccount(null)
  let bobPage = page
  const bobSync = new AppearanceSync('bob', bobPage, bob.io, (patch) => {
    bobPage = { ...bobPage, ...patch }
  })
  await bobSync.refresh(true)
  assertEquals((bob.current() as AccountAppearance).wallpaper, defaultAccountWallpaper, 'no picture')
  assertEquals((bob.current() as AccountAppearance).iconSize, 'large', 'seeded from this device')
  assertEquals(bobPage.wallpaper.image, null, 'page shows the built-in background')
  bobSync.stop()
})

Deno.test('sync engine: a change the account has not confirmed survives a reload and wins over the account', async () => {
  ;(globalThis as { window?: unknown }).window = { localStorage: new MemoryStorage() }
  const account = fakeAccount({ theme: 'light', locale: 'zh-CN', iconSize: 'medium', window: local.window, wallpaper: local.wallpaper })

  const first = new AppearanceSync('alice', local, account.io, () => {})
  await first.refresh(true)
  first.update({ ...local, iconSize: 'large' })
  first.stop() // the page reloads before the pause is over: nothing was saved
  assertEquals(account.writes, [], 'not saved yet')

  // After the reload the page shows its own (cached) value; the account still says medium.
  let page: AccountAppearance = { ...local, iconSize: 'large' }
  const second = new AppearanceSync('alice', page, account.io, (patch) => {
    page = { ...page, ...patch }
  })
  await second.refresh(true)
  assertEquals(page.iconSize, 'large', 'the account did not take it back')
  assertEquals(account.writes, [['iconSize', 'large']], 'saved now')
  second.stop()
})

Deno.test('only the newest pictures stay in the bucket, and never the one shown', () => {
  const items = [
    { key: 'a', created_at: 1 },
    { key: 'b', created_at: 4 },
    { key: 'c', created_at: 3 },
    { key: 'd', created_at: 2 },
    { key: 'new', created_at: 5 },
  ]
  assertEquals(wallpapersToPrune(items, 'new', 3), ['d', 'a'], 'the two newest besides it are kept')
  assertEquals(wallpapersToPrune(items, 'a', 3), ['c', 'd'], 'the current picture is kept even when older')
})

Deno.test('a bucket picture refused as a plain load (no usable cookie) is fetched with the token instead', async () => {
  const requests: string[] = []
  const globals = globalThis as Record<string, unknown>
  const saved = { window: globals.window, Image: globals.Image }
  globals.window = { localStorage: new MemoryStorage(), addEventListener: () => {} }
  // A CSS/<img> load that the server refuses.
  globals.Image = class {
    onerror: (() => void) | null = null
    set src(_url: string) {
      setTimeout(() => this.onerror?.(), 0)
    }
  }
  const url = `/bucket/v1/user/control-panel/alice/wallpapers/${KEY}`
  const io: WallpaperIo = {
    upload: () => Promise.reject(new Error('not used')),
    urlNow: (ref) => (ref === REF ? url : null),
    url: (ref) => Promise.resolve(ref === REF ? url : null),
    fetchWithToken: (ref) => {
      requests.push(ref)
      return Promise.resolve(URL.createObjectURL(new Blob(['picture'], { type: 'image/jpeg' })))
    },
  }
  try {
    const store = new WallpaperStore(io)
    const stop = store.subscribe(() => {})
    store.applyAccount({ ...defaultAccountWallpaper, image: { ref: REF, width: 1200, height: 800, name: 'a.jpg' } })
    assertEquals(store.getSnapshot().url, url, 'drawn from the bucket URL first')
    for (let i = 0; i < 20 && !store.getSnapshot().url?.startsWith('blob:'); i++) await new Promise((done) => setTimeout(done, 5))
    assertEquals(store.getSnapshot().url?.startsWith('blob:'), true, 'then from the fetched copy')
    assertEquals(requests, [REF], 'one fetch')
    assertEquals(store.getSnapshot().wallpaper?.imageUrl, store.getSnapshot().url, 'the desktop draws it')
    stop()
  } finally {
    Object.assign(globals, saved)
  }
})
