/* ── A music player's playlist, as data ──
 *
 * Which files are tracks, the order they play in, and where this device stopped. Used by the Shell's music widget and
 * by the BuckyOS APP client; where it stopped is kept by the caller.
 */

export type PlayOrder = 'sequential' | 'shuffle'

/** Formats a browser plays; anything else in the folder is skipped. */
const AUDIO_EXTENSIONS = new Set(['mp3', 'm4a', 'aac', 'flac', 'wav', 'ogg', 'oga', 'opus', 'weba'])

export function isAudioFile(name: string): boolean {
  const dot = name.lastIndexOf('.')
  return dot > 0 && AUDIO_EXTENSIONS.has(name.slice(dot + 1).toLowerCase())
}

const lastSegment = (path: string) => path.slice(path.lastIndexOf('/') + 1)

/** A track's name as shown: its file name without the extension. */
export function trackTitle(path: string): string {
  const name = lastSegment(path)
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(0, dot) : name
}

export function folderName(path: string): string {
  return lastSegment(path.replace(/\/+$/, '')) || path
}

/** The folder a track is in (its album), or the chosen folder's name for a track directly in it. */
export function trackAlbum(path: string, root: string): string {
  const parent = path.slice(0, path.lastIndexOf('/'))
  return folderName(parent && parent.length > root.replace(/\/+$/, '').length ? parent : root)
}

/** Tracks in folder order: by path, numbers compared as numbers, so an album's `2 …` comes before `10 …`. */
export function sortTracks(paths: readonly string[]): string[] {
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
  return [...paths].sort(collator.compare)
}

function mulberry32(seed: number) {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let t = state
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

export function newShuffleSeed(): number {
  return Math.floor(Math.random() * 0x7fffffff) + 1
}

/** The play order of sorted tracks: as they are, or shuffled by `seed` (the same seed gives the same order again). */
export function playOrder(tracks: readonly string[], order: PlayOrder, seed: number): string[] {
  if (order === 'sequential') return [...tracks]
  const shuffled = [...tracks]
  const random = mulberry32(seed)
  for (let i = shuffled.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1))
    ;[shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]]
  }
  return shuffled
}

// ---------------------------------------------------------------------------
// Where this device stopped
// ---------------------------------------------------------------------------

export interface SavedPlayback {
  /** The folder it belongs to: a different folder starts from the top. */
  folder: string
  track: string
  /** Seconds into the track. */
  position: number
  seed: number
}

/** A saved playback read back from storage; null when it is none. */
export function parseSavedPlayback(value: unknown): SavedPlayback | null {
  if (!value || typeof value !== 'object') return null
  const { folder, track, position, seed } = value as Record<string, unknown>
  if (typeof folder !== 'string' || typeof track !== 'string') return null
  return {
    folder,
    track,
    position: typeof position === 'number' && Number.isFinite(position) && position > 0 ? position : 0,
    seed: typeof seed === 'number' && Number.isInteger(seed) && seed > 0 ? seed : newShuffleSeed(),
  }
}

/** Where to pick up in `queue`: the saved track at its position, else the first track from the start. */
export function resumePoint(queue: readonly string[], saved: SavedPlayback | null, folder: string): { index: number; position: number } {
  if (!saved || saved.folder !== folder) return { index: 0, position: 0 }
  const index = queue.indexOf(saved.track)
  return index < 0 ? { index: 0, position: 0 } : { index, position: saved.position }
}

export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '0:00'
  const whole = Math.floor(seconds)
  const hours = Math.floor(whole / 3600)
  const minutes = Math.floor((whole % 3600) / 60)
  const secs = String(whole % 60).padStart(2, '0')
  return hours ? `${hours}:${String(minutes).padStart(2, '0')}:${secs}` : `${minutes}:${secs}`
}
