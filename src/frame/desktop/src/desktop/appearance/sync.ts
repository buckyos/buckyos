/* ── Keeping this page's appearance and the account's in step ──
 *
 * The page keeps its settings where it always did (theme and language providers, the settings and desktop
 * stores, the wallpaper store), so it paints at once from the last values; this engine reads the account's
 * on start and whenever the page comes back to the foreground, shows them, and saves each setting changed
 * here as soon as the user stops changing it.
 */

import { readDesktopState, writeDesktopState } from '../../api/desktop_state'
import {
  APPEARANCE_STATE_KEY,
  appearanceFields,
  planSync,
  readAppearance,
  sameValue,
  type AccountAppearance,
  type AppearanceField,
} from './model'

/** Whose settings this browser's are: set once they were synced with an account. */
const OWNER_KEY = 'buckyos.desktop.appearance-owner.v1'
/** Settings changed here that the account has not confirmed, so a reload right after a change keeps it. */
const PENDING_KEY = 'buckyos.desktop.appearance-pending.v1'
const PUSH_DELAY_MS = 600
const REFRESH_INTERVAL_MS = 10_000

export interface AppearanceSyncIo {
  /** The account's stored appearance, null when it has none. */
  read(): Promise<unknown>
  /** Saves one setting, leaving the account's others as they are. */
  write(field: AppearanceField, value: unknown): Promise<void>
}

export const desktopStateIo: AppearanceSyncIo = {
  read: async () => (await readDesktopState<unknown>(APPEARANCE_STATE_KEY)).value,
  write: (field, value) => writeDesktopState(APPEARANCE_STATE_KEY, value, field),
}

function readOwner(): string | null {
  try {
    return window.localStorage.getItem(OWNER_KEY)
  } catch {
    return null
  }
}

function writeOwner(userId: string) {
  try {
    window.localStorage.setItem(OWNER_KEY, userId)
  } catch {
    // Without storage the next start compares nothing; the account still wins.
  }
}

function readPending(userId: string): Set<AppearanceField> {
  try {
    const stored = JSON.parse(window.localStorage.getItem(PENDING_KEY) ?? 'null') as { user?: unknown; fields?: unknown } | null
    if (stored?.user !== userId || !Array.isArray(stored.fields)) return new Set()
    return new Set(appearanceFields.filter((field) => (stored.fields as unknown[]).includes(field)))
  } catch {
    return new Set()
  }
}

function writePending(userId: string, fields: ReadonlySet<AppearanceField>) {
  try {
    if (fields.size === 0) window.localStorage.removeItem(PENDING_KEY)
    else window.localStorage.setItem(PENDING_KEY, JSON.stringify({ user: userId, fields: [...fields] }))
  } catch {
    // Without storage a reload during the pause loses the change; it is still saved if the page stays.
  }
}

export class AppearanceSync {
  private local: AccountAppearance
  /** The page's settings when the engine started, before the account was read. */
  private readonly baseline: AccountAppearance
  /** What the account holds, as far as this page knows; null until it was read once. */
  private synced: AccountAppearance | null = null
  /** Settings changed here that the account has not taken yet (kept across reloads). */
  private readonly dirty: Set<AppearanceField>
  private pushTimer: ReturnType<typeof setTimeout> | null = null
  private writes: Promise<void> = Promise.resolve()
  private refreshing: Promise<void> | null = null
  private lastRefresh = 0
  private stopped = false

  private readonly userId: string
  private readonly io: AppearanceSyncIo
  private readonly show: (patch: Partial<AccountAppearance>) => void

  constructor(
    userId: string,
    local: AccountAppearance,
    io: AppearanceSyncIo,
    /** Shows account values on this page; the page then reports them back through `update`. */
    show: (patch: Partial<AccountAppearance>) => void,
  ) {
    this.userId = userId
    this.local = local
    this.baseline = local
    this.io = io
    this.show = show
    this.dirty = readPending(userId)
  }

  /** The page's settings now: changed by the user, or showing values `show` was given. */
  update(local: AccountAppearance) {
    this.local = local
    let changed = false
    for (const field of appearanceFields) {
      if (!this.synced) {
        // Before the account is read only additions count: a setting left over from before a reload stays.
        if (!sameValue(local[field], this.baseline[field])) this.dirty.add(field)
      } else if (sameValue(local[field], this.synced[field])) {
        this.dirty.delete(field)
      } else {
        this.dirty.add(field)
        changed = true
      }
    }
    writePending(this.userId, this.dirty)
    if (changed) this.schedulePush()
  }

  /** Reads the account's settings (at most every 10 s unless `force`), shows them and seeds what it lacks. */
  refresh(force = false): Promise<void> {
    if (this.stopped) return Promise.resolve()
    if (this.refreshing) return this.refreshing
    if (!force && Date.now() - this.lastRefresh < REFRESH_INTERVAL_MS) return Promise.resolve()
    this.refreshing = (async () => {
      try {
        const remote = readAppearance(await this.io.read())
        if (this.stopped) return
        this.lastRefresh = Date.now()
        const owner = readOwner()
        const plan = planSync({
          remote,
          local: this.local,
          dirty: this.dirty,
          foreignWallpaper: owner !== null && owner !== this.userId && this.local.wallpaper.image !== null,
        })
        this.synced = plan.synced
        writeOwner(this.userId)
        if (Object.keys(plan.apply).length) this.show(plan.apply)
        if (Object.keys(plan.push).length) await this.write(plan.push)
      } catch {
        // Offline or signed out: the page keeps what it shows and the next refresh tries again.
      } finally {
        this.refreshing = null
      }
    })()
    return this.refreshing
  }

  /** Saves the settings changed here now instead of after the pause. */
  flush(): Promise<void> {
    if (this.pushTimer !== null) {
      clearTimeout(this.pushTimer)
      this.pushTimer = null
    }
    if (!this.synced || this.dirty.size === 0) return Promise.resolve()
    const patch: Partial<AccountAppearance> = {}
    for (const field of this.dirty) Object.assign(patch, { [field]: this.local[field] })
    return this.write(patch).catch(() => {
      // Kept dirty: saved with the next change or refresh.
    })
  }

  stop() {
    this.stopped = true
    if (this.pushTimer !== null) clearTimeout(this.pushTimer)
    this.pushTimer = null
  }

  private schedulePush() {
    if (this.pushTimer !== null) clearTimeout(this.pushTimer)
    this.pushTimer = setTimeout(() => {
      this.pushTimer = null
      void this.flush()
    }, PUSH_DELAY_MS)
  }

  /** One write at a time, in order, so a later value of a setting is never overtaken by an earlier one. */
  private write(patch: Partial<AccountAppearance>): Promise<void> {
    const run = async () => {
      for (const field of appearanceFields) {
        if (!(field in patch)) continue
        const value = patch[field]
        await this.io.write(field, value)
        if (this.synced) Object.assign(this.synced, { [field]: value })
        if (sameValue(this.local[field], value)) this.dirty.delete(field)
      }
      writePending(this.userId, this.dirty)
    }
    const next = this.writes.then(run, run)
    this.writes = next.catch(() => {})
    return next
  }
}
