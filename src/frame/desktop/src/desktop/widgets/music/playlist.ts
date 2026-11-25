/**
 * The music player widget's playlist (`shared/music/playlist.ts`), and where this device stopped. The folder and the
 * order are the widget's settings (its `.widget` file, the same on every device); the track and position are this
 * device's, saved as it plays, so closing the page leaves it paused there.
 */

import { parseSavedPlayback, type SavedPlayback } from '../../../shared/music/playlist.ts'

export * from '../../../shared/music/playlist.ts'

const storageKey = (itemId: string) => `buckyos.desktop.music-player.v1:${itemId}`

export function readSavedPlayback(itemId: string): SavedPlayback | null {
  try {
    return parseSavedPlayback(JSON.parse(window.localStorage.getItem(storageKey(itemId)) ?? 'null'))
  } catch {
    return null
  }
}

export function writeSavedPlayback(itemId: string, playback: SavedPlayback) {
  try {
    window.localStorage.setItem(storageKey(itemId), JSON.stringify(playback))
  } catch {
    // Private mode or a full store: the player still works, it just starts over next time.
  }
}
