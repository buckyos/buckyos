import { IconButton, Slider } from '@mui/material'
import clsx from 'clsx'
import { FolderOpen, Music, Pause, Play, Shuffle, SkipBack, SkipForward } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { cyfsPathToLocal, normalizeCyfsPath } from '../../components/preview/session'
import { useSelectContent } from '../../sysdlg'
import { listMusicFolder, trackSource } from './music/library'
import {
  folderName,
  formatTime,
  newShuffleSeed,
  playOrder,
  readSavedPlayback,
  resumePoint,
  trackAlbum,
  trackTitle,
  writeSavedPlayback,
  type PlayOrder,
} from './music/playlist'
import type { DesktopWidgetProps } from './types'

/** How often a playing track's position is saved, besides on pause, track change and page close. */
const SAVE_INTERVAL_MS = 5000
/** "Previous" past this point starts the track over instead. */
const RESTART_AFTER_S = 3

/** A folder's listing; `tracks` is null when it could not be listed. `key` says which folder and which (re)load. */
interface Listing {
  key: string
  tracks: string[] | null
}

/**
 * Plays the audio files of a Zone folder, in order or shuffled. The folder and the order are kept in the widget's
 * `.widget` file; the track and position on this device, so closing the page leaves it paused where it was and the
 * next visit picks up there.
 */
export function MusicPlayerWidget({ item, onUpdateConfig }: DesktopWidgetProps) {
  const { t } = useI18n()
  const selectContent = useSelectContent()
  const folder = typeof item.config.folder === 'string' ? item.config.folder : ''
  const order: PlayOrder = item.config.order === 'shuffle' ? 'shuffle' : 'sequential'
  const compact = item.h < 2

  const audioRef = useRef<HTMLAudioElement | null>(null)
  const [reloads, setReloads] = useState(0)
  const listingKey = folder ? `${reloads}:${folder}` : ''
  const [listing, setListing] = useState<Listing>({ key: '', tracks: null })
  const libraryStatus = !folder
    ? 'none'
    : listing.key !== listingKey
      ? 'loading'
      : listing.tracks
        ? 'ready'
        : 'unavailable'
  const [seed, setSeed] = useState(() => readSavedPlayback(item.id)?.seed ?? newShuffleSeed())
  // The track is the source of truth; `nonce` loads it again (a one-track folder, a retry).
  const [track, setTrack] = useState<{ path: string | null; nonce: number }>({ path: null, nonce: 0 })
  const [playing, setPlaying] = useState(false)
  const [position, setPosition] = useState(0)
  const [duration, setDuration] = useState(0)
  const [scrub, setScrub] = useState<number | null>(null)
  const [failed, setFailed] = useState(false)

  // Play as soon as the track's source is in (the user asked for it, or the previous one ended while playing).
  const playWhenLoaded = useRef(false)
  // Seconds to seek to once the track's metadata is in (a resumed track).
  const pendingSeek = useRef(0)
  const failures = useRef(0)
  const retried = useRef(false)
  const lastSave = useRef(0)

  const tracks = useMemo(() => (libraryStatus === 'ready' && listing.tracks ? listing.tracks : []), [libraryStatus, listing])
  const queue = useMemo(() => playOrder(tracks, order, seed), [tracks, order, seed])
  const index = track.path ? queue.indexOf(track.path) : -1

  const save = useCallback(() => {
    const audio = audioRef.current
    if (!folder || !track.path || !audio) return
    lastSave.current = Date.now()
    writeSavedPlayback(item.id, {
      folder,
      track: track.path,
      position: pendingSeek.current || audio.currentTime || 0,
      seed,
    })
  }, [folder, item.id, seed, track.path])

  // The folder's tracks, and where this device left off in them.
  useEffect(() => {
    if (!listingKey) return
    const controller = new AbortController()
    audioRef.current?.pause()
    listMusicFolder(folder, controller.signal).then(
      (found) => {
        if (controller.signal.aborted) return
        const saved = readSavedPlayback(item.id)
        const nextSeed = saved?.folder === folder ? saved.seed : newShuffleSeed()
        const nextQueue = playOrder(found, order, nextSeed)
        const resume = resumePoint(nextQueue, saved, folder)
        pendingSeek.current = resume.position
        failures.current = 0
        retried.current = false
        setFailed(false)
        setSeed(nextSeed)
        setListing({ key: listingKey, tracks: found })
        setPosition(resume.position)
        setDuration(0)
        setTrack((current) => ({ path: nextQueue[resume.index] ?? null, nonce: current.nonce + 1 }))
      },
      () => {
        if (!controller.signal.aborted) setListing({ key: listingKey, tracks: null })
      },
    )
    return () => controller.abort()
    // The key names the folder; the order only changes which track comes next.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listingKey])

  // The current track's source.
  useEffect(() => {
    const audio = audioRef.current
    if (!audio || !track.path) return
    const controller = new AbortController()
    let release = () => {}
    trackSource(track.path, controller.signal).then(
      (source) => {
        if (controller.signal.aborted) {
          source.release()
          return
        }
        release = source.release
        audio.src = source.url
        if (playWhenLoaded.current) void audio.play().catch(() => setPlaying(false))
      },
      // Not readable (gone, no access): handled like a track the browser cannot play. The previous track's source
      // goes first, so its position is not taken for one that stopped mid-track.
      () => {
        if (controller.signal.aborted) return
        audio.removeAttribute('src')
        audio.load()
        audio.dispatchEvent(new Event('error'))
      },
    )
    return () => {
      controller.abort()
      release()
    }
  }, [track])

  // Where this device stopped: kept on page close (it comes back paused there) and when the widget goes away.
  useEffect(() => {
    window.addEventListener('pagehide', save)
    return () => {
      window.removeEventListener('pagehide', save)
      save()
    }
  }, [save])

  // Stops for good when the widget leaves the desktop.
  useEffect(() => {
    const audio = audioRef.current
    return () => audio?.pause()
  }, [])

  const goTo = useCallback(
    (path: string | null, options: { play?: boolean } = {}) => {
      playWhenLoaded.current = options.play ?? !(audioRef.current?.paused ?? true)
      pendingSeek.current = 0
      retried.current = false
      setPosition(0)
      setDuration(0)
      setTrack((current) => ({ path, nonce: current.nonce + 1 }))
    },
    [],
  )

  const next = useCallback(
    (options: { play?: boolean } = {}) => {
      if (!queue.length) return
      if (index + 1 < queue.length) {
        goTo(queue[index + 1], options)
        return
      }
      // Past the last track: from the top again, in a new shuffle when shuffled (not starting with the one just heard).
      if (order === 'shuffle' && queue.length > 1) {
        const nextSeed = newShuffleSeed()
        const round = playOrder(tracks, order, nextSeed)
        setSeed(nextSeed)
        goTo(round[0] === track.path ? round[1] : round[0], options)
      } else {
        goTo(queue[0], options)
      }
    },
    [goTo, index, order, queue, track.path, tracks],
  )

  const previous = useCallback(() => {
    const audio = audioRef.current
    if (!queue.length || !audio) return
    if (audio.currentTime > RESTART_AFTER_S || queue.length === 1) {
      audio.currentTime = 0
      return
    }
    goTo(queue[(index - 1 + queue.length) % queue.length])
  }, [goTo, index, queue])

  const togglePlay = useCallback(() => {
    const audio = audioRef.current
    if (!audio || !track.path) return
    if (!audio.paused) {
      audio.pause()
      return
    }
    playWhenLoaded.current = true
    if (audio.getAttribute('src')) void audio.play().catch(() => setPlaying(false))
  }, [track.path])

  const toggleOrder = useCallback(() => {
    const nextOrder: PlayOrder = order === 'shuffle' ? 'sequential' : 'shuffle'
    if (nextOrder === 'shuffle') setSeed(newShuffleSeed())
    onUpdateConfig(item.id, { order: nextOrder })
  }, [item.id, onUpdateConfig, order])

  const chooseFolder = useCallback(async () => {
    const result = await selectContent(
      {
        mode: 'folder',
        title: t('music.chooseFolder', 'Choose a music folder'),
        ...(folder ? { start: { kind: 'cyfs-path' as const, path: normalizeCyfsPath(folder) } } : {}),
      },
      { id: 'desktop-music-player' },
    )
    if (result.action !== 'selected') return
    const chosen = result.items[0]
    if (chosen?.origin !== 'zone' || chosen.source.kind !== 'cyfs-path') return
    playWhenLoaded.current = true
    const path = cyfsPathToLocal(chosen.source.path)
    if (path === folder) setReloads((count) => count + 1)
    else onUpdateConfig(item.id, { folder: path })
  }, [folder, item.id, onUpdateConfig, selectContent, t])

  // Lock screen and media keys on phones and desktops.
  useEffect(() => {
    if (!('mediaSession' in navigator) || !track.path || typeof MediaMetadata === 'undefined') return
    const session = navigator.mediaSession
    session.metadata = new MediaMetadata({ title: trackTitle(track.path), album: trackAlbum(track.path, folder) })
    const handlers: Array<[MediaSessionAction, MediaSessionActionHandler]> = [
      ['play', () => togglePlay()],
      ['pause', () => audioRef.current?.pause()],
      ['nexttrack', () => next()],
      ['previoustrack', () => previous()],
    ]
    for (const [action, handler] of handlers) {
      try {
        session.setActionHandler(action, handler)
      } catch {
        // An action this browser does not offer.
      }
    }
    return () => {
      for (const [action] of handlers) {
        try {
          session.setActionHandler(action, null)
        } catch {
          // As above.
        }
      }
    }
  }, [folder, next, previous, togglePlay, track.path])

  const audio = (
    <audio
      ref={audioRef}
      preload="metadata"
      data-testid="music-audio"
      onPlay={() => setPlaying(true)}
      onPause={() => {
        setPlaying(false)
        save()
      }}
      onLoadedMetadata={(event) => {
        const element = event.currentTarget
        setDuration(Number.isFinite(element.duration) ? element.duration : 0)
        const seek = pendingSeek.current
        pendingSeek.current = 0
        if (seek > 0 && seek < element.duration) element.currentTime = seek
        // A closed tab is not always told (a phone browser killed in the background): the new track is kept now.
        save()
      }}
      onPlaying={() => {
        failures.current = 0
        setFailed(false)
      }}
      onTimeUpdate={(event) => {
        setPosition(event.currentTarget.currentTime)
        if (!event.currentTarget.paused && Date.now() - lastSave.current > SAVE_INTERVAL_MS) save()
      }}
      onEnded={() => next({ play: true })}
      onError={(event) => {
        const element = event.currentTarget
        // A source that stopped working mid-track (an expired link): asked again once, from where it was.
        if (!retried.current && element.currentTime > 0) {
          retried.current = true
          pendingSeek.current = element.currentTime
          playWhenLoaded.current = !element.paused || playWhenLoaded.current
          setTrack((current) => ({ ...current, nonce: current.nonce + 1 }))
          return
        }
        failures.current += 1
        if (failures.current >= queue.length) {
          setFailed(true)
          setPlaying(false)
          return
        }
        next({ play: playWhenLoaded.current || !element.paused })
      }}
    />
  )


  const ready = libraryStatus === 'ready' && queue.length > 0 && !failed
  const title = track.path ? trackTitle(track.path) : ''
  const album = track.path ? trackAlbum(track.path, folder) : folderName(folder)
  const shown = scrub ?? position
  const count = `${index + 1}/${queue.length}`
  const playButton = (
    <button
      type="button"
      onClick={togglePlay}
      aria-label={playing ? t('music.pause', 'Pause') : t('music.play', 'Play')}
      data-testid="music-play"
      data-playing={playing ? 'true' : 'false'}
      className="flex size-10 shrink-0 items-center justify-center rounded-full bg-[color:var(--cp-text)] text-[color:var(--cp-surface)] transition-transform active:scale-95"
    >
      {playing ? <Pause className="size-4" fill="currentColor" /> : <Play className="ml-0.5 size-4" fill="currentColor" />}
    </button>
  )
  const nextButton = (
    <IconButton size="small" aria-label={t('music.next', 'Next')} onClick={() => next()} data-testid="music-next">
      <SkipForward className="size-4" />
    </IconButton>
  )

  let content: ReactNode
  if (!ready) {
    const message =
      libraryStatus === 'none'
        ? t('music.empty', 'Play a folder of music from your Zone.')
        : libraryStatus === 'loading'
          ? t('music.loading', 'Looking for music…')
          : libraryStatus === 'unavailable'
            ? t('music.unavailable', 'This folder cannot be opened.')
            : failed
              ? t('music.failed', 'The music in this folder cannot be played.')
              : t('music.noTracks', 'There is no music in this folder.')
    content = (
      <div className={clsx('flex min-h-0 flex-1 gap-3', compact ? 'items-center' : 'flex-col items-start justify-center')}>
        <span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] bg-[color:var(--cp-surface-2)]">
          <Music className="size-4" />
        </span>
        <p className={clsx('min-w-0 text-xs leading-5 text-[color:var(--cp-muted)]', compact && 'flex-1 truncate')} data-testid="music-status">
          {folder && libraryStatus !== 'none' ? `${folderName(folder)} · ${message}` : message}
        </p>
        {libraryStatus === 'loading' ? null : (
          <button
            type="button"
            onClick={() => void chooseFolder()}
            data-testid="music-choose-folder"
            className="shrink-0 rounded-[8px] border border-[color:var(--cp-border)] px-2.5 py-1 text-xs font-medium text-[color:var(--cp-text)] transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]"
          >
            {folder ? t('music.otherFolder', 'Choose another folder') : t('music.chooseFolderShort', 'Choose folder')}
          </button>
        )}
      </div>
    )
  } else if (compact) {
    content = (
      <>
        <div className="flex min-h-0 flex-1 items-center gap-2.5">
          {playButton}
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium" data-testid="music-title">{title}</p>
            <p className="truncate text-[11px] text-[color:var(--cp-muted)]" data-testid="music-folder">{album} · {count}</p>
          </div>
          {nextButton}
        </div>
        <div className="h-0.5 shrink-0 overflow-hidden rounded-full bg-[color:var(--cp-border)]" aria-hidden="true">
          <div className="h-full bg-[color:var(--cp-text)]" style={{ width: duration ? `${Math.min(100, (shown / duration) * 100)}%` : 0 }} />
        </div>
      </>
    )
  } else {
    content = (
      <>
        <header className="-mr-1.5 -mt-1 flex shrink-0 items-center gap-1.5 text-[11px] text-[color:var(--cp-muted)]">
          <Music className="size-3.5 shrink-0" />
          <span className="min-w-0 truncate" data-testid="music-folder">{album}</span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0 tabular-nums" data-testid="music-count">{count}</span>
          <IconButton
            size="small"
            className="!ml-auto"
            aria-label={t('music.chooseFolder', 'Choose a music folder')}
            title={t('music.chooseFolder', 'Choose a music folder')}
            onClick={() => void chooseFolder()}
            data-testid="music-choose-folder"
          >
            <FolderOpen className="size-3.5" />
          </IconButton>
        </header>
        <p className="mt-1 line-clamp-2 min-h-0 break-words text-sm font-medium leading-5" data-testid="music-title" title={title}>
          {title}
        </p>
        <div className="mt-auto">
          <Slider
            size="small"
            className="widget-interactive swiper-no-swiping !py-2"
            aria-label={t('music.position', 'Position')}
            min={0}
            max={duration || 1}
            step={1}
            value={Math.min(shown, duration || 1)}
            disabled={!duration}
            onChange={(_, value) => setScrub(value as number)}
            onChangeCommitted={(_, value) => {
              if (audioRef.current) audioRef.current.currentTime = value as number
              setScrub(null)
            }}
            sx={{ color: 'var(--cp-text)', '& .MuiSlider-thumb': { width: 10, height: 10 }, '& .MuiSlider-rail': { opacity: 0.2 } }}
            data-testid="music-progress"
          />
          <div className="-mt-1 flex justify-between text-[10px] tabular-nums text-[color:var(--cp-muted)]" data-testid="music-time">
            <span>{formatTime(shown)}</span>
            <span>{formatTime(duration)}</span>
          </div>
          <div className="mx-auto mt-1 flex w-full max-w-[200px] items-center justify-between">
            <IconButton
              size="small"
              aria-label={t('music.shuffle', 'Shuffle')}
              aria-pressed={order === 'shuffle'}
              title={order === 'shuffle' ? t('music.shuffleOn', 'Shuffle: on') : t('music.shuffleOff', 'Shuffle: off, playing in order')}
              onClick={toggleOrder}
              data-testid="music-shuffle"
              sx={{ color: order === 'shuffle' ? 'var(--cp-accent)' : 'var(--cp-muted)' }}
            >
              <Shuffle className="size-4" />
            </IconButton>
            <IconButton size="small" aria-label={t('music.previous', 'Previous')} onClick={previous} data-testid="music-previous">
              <SkipBack className="size-4" />
            </IconButton>
            {playButton}
            {nextButton}
          </div>
        </div>
      </>
    )
  }

  // One `<audio>`, always the first child: a change of layout (resized, a folder loading) must not replace it.
  return (
    <Shell compact={compact && ready} label={t('widgets.musicPlayer', 'Music Player')}>
      {audio}
      {content}
    </Shell>
  )
}

function Shell({ children, compact = false, label }: { children: ReactNode; compact?: boolean; label: string }) {
  return (
    <section
      aria-label={label}
      data-testid="music-player"
      className={clsx(
        'flex h-full min-h-0 flex-col rounded-[15px] bg-[color:var(--cp-surface)] text-[color:var(--cp-text)]',
        compact ? 'justify-center gap-1.5 px-3 py-2' : 'p-3',
      )}
    >
      {children}
    </section>
  )
}
