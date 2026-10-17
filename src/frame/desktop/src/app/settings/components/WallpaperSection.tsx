/* ── Settings › Appearance › Desktop background: a picture of this device's own, how it covers the screen,
 * and a desktop and a phone preview to check both shapes before leaving Settings. ── */

import { Button, Slider, useMediaQuery } from '@mui/material'
import { ImagePlus, LocateFixed, RotateCcw } from 'lucide-react'
import {
  useEffect,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react'
import { useI18n } from '../../../i18n/provider'
import { DesktopApiError } from '../../../api/desktop_state'
import { WallpaperImageError, type WallpaperImageErrorReason } from '../../../desktop/wallpaper/image'
import {
  TILE_SCALE_MAX,
  TILE_SCALE_MIN,
  coverOverhang,
  fitCrops,
  panFocus,
  wallpaperFits,
  type WallpaperFit,
} from '../../../desktop/wallpaper/model'
import { useWallpaperSnapshot, wallpaperStore } from '../../../desktop/wallpaper/store'
import { WallpaperImageLayers } from '../../../desktop/wallpaper/WallpaperImageLayers'
import type { DesktopWallpaper } from '../../../models/ui'
import { Section } from './shared/Section'

const fitText: Record<WallpaperFit, { label: string; hint: string }> = {
  fill: { label: 'Fill', hint: 'Covers the whole screen; the edges may be cropped. Drag a preview to choose what stays in view.' },
  fit: { label: 'Fit', hint: 'Shows the whole picture; a blurred copy fills the space around it.' },
  stretch: { label: 'Stretch', hint: 'Stretches the picture to the screen, changing its proportions.' },
  tile: { label: 'Tile', hint: 'Repeats the picture across the screen.' },
  center: { label: 'Center', hint: 'Shows the picture at its own pixel size in the middle of the screen.' },
  panorama: {
    label: 'Span pages',
    hint: 'Spreads the picture across the desktop pages; it moves as you swipe between them. Drag a preview to choose what stays in view.',
  },
}

interface Screen {
  /** CSS px. */
  width: number
  height: number
  pixelRatio: number
}

/** What each preview stands for when it is not the screen Settings is shown on. */
const typicalScreens: Record<Device, Screen> = {
  desktop: { width: 1440, height: 900, pixelRatio: 2 },
  phone: { width: 390, height: 844, pixelRatio: 3 },
}

const deviceLook = {
  desktop: { radius: 6, columns: 7, icons: 14, aspect: [1.2, 2.4] },
  phone: { radius: 12, columns: 4, icons: 16, aspect: [0.42, 0.8] },
} as const

type Device = 'desktop' | 'phone'

function readThisScreen(): Screen {
  return { width: window.innerWidth, height: window.innerHeight, pixelRatio: window.devicePixelRatio || 1 }
}

/** The screen Settings is shown on, for the preview of its own kind: it then matches the desktop exactly. */
function useThisScreen(): Screen {
  const [screen, setScreen] = useState(readThisScreen)
  useEffect(() => {
    const update = () => setScreen(readThisScreen())
    window.addEventListener('resize', update)
    return () => window.removeEventListener('resize', update)
  }, [])
  return screen
}

/** An odd window shape would make a preview unreadable; such a one is shown at the nearest sensible shape. */
function previewAspect(device: Device, screen: Screen) {
  const [min, max] = deviceLook[device].aspect
  return Math.min(max, Math.max(min, screen.width / screen.height))
}

const PAN_STEP = 5
const panKeys: Record<string, [number, number]> = {
  ArrowLeft: [-PAN_STEP, 0],
  ArrowRight: [PAN_STEP, 0],
  ArrowUp: [0, -PAN_STEP],
  ArrowDown: [0, PAN_STEP],
}

function useElementWidth(ref: RefObject<HTMLElement | null>) {
  const [width, setWidth] = useState(0)
  useEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width))
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref])
  return width
}

/** Stand-ins for launcher icons, so the preview reads as a desktop and shows the label contrast. */
function PreviewIcons({ device, onPicture }: { device: Device; onPicture: boolean }) {
  const look = deviceLook[device]
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute grid"
      style={
        device === 'desktop'
          ? { left: '4%', top: '9%', width: '44%', gridTemplateColumns: `repeat(${look.columns}, 1fr)`, rowGap: '6%' }
          : { left: '7%', right: '7%', top: '9%', gridTemplateColumns: `repeat(${look.columns}, 1fr)`, rowGap: '14%' }
      }
    >
      {Array.from({ length: look.icons }, (_, index) => (
        <span key={index} className="flex flex-col items-center gap-[2px]">
          <span
            className="w-[58%]"
            style={{
              aspectRatio: '1',
              borderRadius: '28%',
              background: 'color-mix(in srgb, var(--cp-surface) 86%, transparent)',
              boxShadow: '0 1px 2px color-mix(in srgb, var(--cp-shadow) 18%, transparent)',
            }}
          />
          <span
            className="h-[2px] w-[64%] rounded-full"
            style={{ background: onPicture ? 'rgb(255 255 255 / 0.9)' : 'color-mix(in srgb, var(--cp-text) 32%, transparent)' }}
          />
        </span>
      ))}
    </div>
  )
}

function ScreenPreview({
  device,
  screen,
  pages,
  wallpaper,
  label,
  panLabel,
  onPan,
}: {
  device: Device
  screen: Screen
  /** Desktop pages a `panorama` picture spans. */
  pages: number
  wallpaper: DesktopWallpaper | null
  label: string
  panLabel: string
  onPan: (focus: { x: number; y: number }, done: boolean) => void
}) {
  const look = deviceLook[device]
  const frame = useRef<HTMLDivElement>(null)
  const width = useElementWidth(frame)
  const drag = useRef<{
    pointerId: number
    x: number
    y: number
    focus: { x: number; y: number }
    overhang: { x: number; y: number }
  } | null>(null)
  const [dragging, setDragging] = useState(false)
  const pannable = Boolean(wallpaper && fitCrops(wallpaper.mode as WallpaperFit))
  const focus = { x: wallpaper?.focusX ?? 50, y: wallpaper?.focusY ?? 50 }

  const dragTo = (event: ReactPointerEvent<HTMLDivElement>, done: boolean) => {
    const current = drag.current
    if (!current || current.pointerId !== event.pointerId) return
    if (done) {
      drag.current = null
      setDragging(false)
    }
    onPan(panFocus(current.focus, { dx: event.clientX - current.x, dy: event.clientY - current.y }, current.overhang), done)
  }

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pannable || !wallpaper || event.button !== 0) return
    const rect = event.currentTarget.getBoundingClientRect()
    drag.current = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      focus,
      overhang: coverOverhang(
        wallpaper.mode === 'panorama' ? rect.width * pages : rect.width,
        rect.height,
        wallpaper.imageWidth ?? 0,
        wallpaper.imageHeight ?? 0,
      ),
    }
    event.currentTarget.setPointerCapture(event.pointerId)
    setDragging(true)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!pannable) return
    const step = panKeys[event.key]
    if (!step) return
    event.preventDefault()
    onPan({ x: Math.min(100, Math.max(0, focus.x + step[0])), y: Math.min(100, Math.max(0, focus.y + step[1])) }, true)
  }

  return (
    <figure className="m-0 flex min-w-0 flex-col items-center gap-1.5">
      <div
        ref={frame}
        role={pannable ? 'group' : 'img'}
        aria-label={pannable ? `${label}. ${panLabel}` : label}
        tabIndex={pannable ? 0 : undefined}
        data-testid={`settings-wallpaper-preview-${device}`}
        className="relative w-full select-none overflow-hidden"
        style={{
          aspectRatio: `${previewAspect(device, screen)}`,
          borderRadius: look.radius,
          border: '1px solid var(--cp-border)',
          background: 'var(--cp-bg)',
          boxShadow: '0 4px 14px color-mix(in srgb, var(--cp-shadow) 10%, transparent)',
          cursor: pannable ? (dragging ? 'grabbing' : 'grab') : undefined,
          // The page must not scroll while a finger pans the picture.
          touchAction: pannable ? 'none' : undefined,
        }}
        onPointerDown={onPointerDown}
        onPointerMove={(event) => dragTo(event, false)}
        onPointerUp={(event) => dragTo(event, true)}
        onPointerCancel={(event) => dragTo(event, true)}
        onKeyDown={onKeyDown}
      >
        {wallpaper && width > 0 ? (
          <WallpaperImageLayers
            wallpaper={wallpaper}
            pages={pages}
            // Width-matched: a clamped shape keeps the screen's width and shows more or less of its height.
            view={{ scale: width / screen.width, pixelRatio: screen.pixelRatio }}
          />
        ) : null}
        <PreviewIcons device={device} onPicture={Boolean(wallpaper)} />
      </div>
      {/* May be wider than the phone frame; it overhangs both sides rather than wrapping. */}
      <figcaption className="whitespace-nowrap text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
        {label}
      </figcaption>
    </figure>
  )
}

export function WallpaperSection({ pageCount }: { pageCount: number }) {
  const { t } = useI18n()
  const { account: preferences, wallpaper } = useWallpaperSnapshot()
  const image = preferences.image
  // Same breakpoint as the desktop shell's mobile layout.
  const thisDevice: Device = useMediaQuery('(max-width:768px)') ? 'phone' : 'desktop'
  const thisScreen = useThisScreen()
  const screenOf = (device: Device) => (device === thisDevice ? thisScreen : typicalScreens[device])
  const pages = Math.max(1, pageCount)
  const fileInput = useRef<HTMLInputElement>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<WallpaperImageErrorReason | 'upload' | null>(null)
  const [dropping, setDropping] = useState(false)

  const pick = async (file?: File | null) => {
    if (!file || busy) return
    setBusy(true)
    setError(null)
    try {
      await wallpaperStore.setImage(file)
    } catch (caught) {
      if (caught instanceof WallpaperImageError) setError(caught.reason)
      else if (caught instanceof DesktopApiError && caught.status === 413) setError('too_large')
      else if (caught instanceof DesktopApiError && caught.status === 415) setError('type')
      else setError('upload')
    } finally {
      setBusy(false)
      if (fileInput.current) fileInput.current.value = ''
    }
  }

  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setDropping(false)
    void pick(event.dataTransfer.files[0])
  }

  const pan = (focus: { x: number; y: number }, done: boolean) =>
    wallpaperStore.setPreferences({ focusX: Math.round(focus.x * 10) / 10, focusY: Math.round(focus.y * 10) / 10 }, { persist: done })

  const errorText =
    error === 'type'
      ? t('settings.wallpaper.errorType', 'Choose a picture file (JPEG, PNG, WebP, GIF, AVIF…).')
      : error === 'too_large'
        ? t('settings.wallpaper.errorSize', 'That picture is too large (40 MB at most).')
        : error === 'upload'
          ? t('settings.wallpaper.errorUpload', 'The picture could not be saved to your account. The current background is kept; try again later.')
          : t('settings.wallpaper.errorUnreadable', 'That picture could not be read. The current background is kept.')

  const fit = preferences.fit
  const centred = Math.abs(preferences.focusX - 50) < 0.5 && Math.abs(preferences.focusY - 50) < 0.5
  const panLabel = t('settings.wallpaper.panHint', 'Drag, or use the arrow keys, to choose the part of the picture in view.')

  return (
    <Section
      title={t('settings.wallpaper.title', 'Desktop background')}
      description={t('settings.wallpaper.description', 'A picture for your desktop. It is saved to your account and shown on every device you sign in on.')}
      testId="settings-wallpaper"
    >
      <div className="flex flex-col gap-3">
        <div
          className="-m-2 grid w-[calc(100%+1rem)] max-w-[436px] items-start gap-3 rounded-xl p-2"
          style={{
            gridTemplateColumns: `${previewAspect('desktop', screenOf('desktop'))}fr ${previewAspect('phone', screenOf('phone'))}fr`,
            outline: dropping ? '2px dashed color-mix(in srgb, var(--cp-accent) 60%, transparent)' : undefined,
            background: dropping ? 'color-mix(in srgb, var(--cp-accent-soft) 14%, var(--cp-surface))' : undefined,
          }}
          onDragOver={(event) => {
            if (!event.dataTransfer.types.includes('Files')) return
            event.preventDefault()
            setDropping(true)
          }}
          onDragLeave={() => setDropping(false)}
          onDrop={onDrop}
        >
          <ScreenPreview
            device="desktop"
            screen={screenOf('desktop')}
            pages={pages}
            wallpaper={wallpaper}
            label={thisDevice === 'desktop' ? t('settings.wallpaper.thisScreen', '{{device}} · this screen', { device: t('settings.wallpaper.previewDesktop', 'Desktop') }) : t('settings.wallpaper.previewDesktop', 'Desktop')}
            panLabel={panLabel}
            onPan={pan}
          />
          <ScreenPreview
            device="phone"
            screen={screenOf('phone')}
            pages={pages}
            wallpaper={wallpaper}
            label={thisDevice === 'phone' ? t('settings.wallpaper.thisScreen', '{{device}} · this screen', { device: t('settings.wallpaper.previewPhone', 'Phone') }) : t('settings.wallpaper.previewPhone', 'Phone')}
            panLabel={panLabel}
            onPan={pan}
          />
        </div>

        <div className="flex min-w-0 flex-col gap-3">
          <div className="flex flex-col items-start gap-1.5">
            <input
              ref={fileInput}
              type="file"
              accept="image/*"
              className="hidden"
              aria-label={t('settings.wallpaper.choose', 'Choose picture…')}
              data-testid="settings-wallpaper-file"
              disabled={busy}
              onChange={(event) => void pick(event.target.files?.[0])}
            />
            <div className="flex flex-wrap gap-2">
              <Button
                size="small"
                variant="outlined"
                startIcon={<ImagePlus size={14} />}
                disabled={busy}
                onClick={() => fileInput.current?.click()}
                data-testid="settings-wallpaper-choose"
              >
                {image ? t('settings.wallpaper.replace', 'Change picture…') : t('settings.wallpaper.choose', 'Choose picture…')}
              </Button>
              {image ? (
                <Button
                  size="small"
                  variant="text"
                  startIcon={<RotateCcw size={14} />}
                  disabled={busy}
                  onClick={() => {
                    setError(null)
                    wallpaperStore.clearImage()
                  }}
                  data-testid="settings-wallpaper-reset"
                >
                  {t('settings.wallpaper.reset', 'Use default background')}
                </Button>
              ) : null}
            </div>
            <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }} data-testid="settings-wallpaper-status">
              {error ? (
                <span role="alert" style={{ color: 'var(--cp-danger)' }}>{errorText}</span>
              ) : busy ? (
                t('settings.wallpaper.processing', 'Uploading the picture…')
              ) : image ? (
                <span className="break-all">{`${image.name || t('settings.wallpaper.picture', 'Picture')} · ${image.width} × ${image.height}`}</span>
              ) : (
                t('settings.wallpaper.usingDefault', 'Using the default background. You can also drop a picture on the previews.')
              )}
            </p>
          </div>

          {image ? (
            <div className="flex flex-col gap-2">
              <span className="text-sm" style={{ color: 'var(--cp-text)' }} id="settings-wallpaper-fit-label">
                {t('settings.wallpaper.fit', 'Picture position')}
              </span>
              <div
                role="radiogroup"
                aria-labelledby="settings-wallpaper-fit-label"
                className="grid grid-cols-3 gap-1 rounded-lg p-1 sm:grid-cols-6"
                style={{ background: 'var(--cp-bg)' }}
              >
                {wallpaperFits.map((value) => (
                  <button
                    key={value}
                    type="button"
                    role="radio"
                    aria-checked={fit === value}
                    data-testid={`settings-wallpaper-fit-${value}`}
                    onClick={() => wallpaperStore.setPreferences({ fit: value })}
                    className="min-h-8 truncate rounded-md px-1.5 text-xs"
                    style={{
                      background: fit === value ? 'var(--cp-surface)' : 'transparent',
                      color: fit === value ? 'var(--cp-text)' : 'var(--cp-muted)',
                      border: fit === value ? '1px solid var(--cp-border)' : '1px solid transparent',
                      fontWeight: fit === value ? 600 : 500,
                    }}
                  >
                    {t(`settings.wallpaper.fit.${value}`, fitText[value].label)}
                  </button>
                ))}
              </div>
              <p className="text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
                {t(`settings.wallpaper.fitHint.${fit}`, fitText[fit].hint)}
              </p>

              {fit === 'tile' ? (
                <div>
                  <div className="flex items-center justify-between text-sm">
                    <span style={{ color: 'var(--cp-text)' }}>{t('settings.wallpaper.tileScale', 'Tile size')}</span>
                    <span style={{ color: 'var(--cp-muted)' }}>{preferences.tileScale}%</span>
                  </div>
                  <Slider
                    size="small"
                    min={TILE_SCALE_MIN}
                    max={TILE_SCALE_MAX}
                    step={5}
                    value={preferences.tileScale}
                    onChange={(_, next) => wallpaperStore.setPreferences({ tileScale: next as number }, { persist: false })}
                    onChangeCommitted={(_, next) => wallpaperStore.setPreferences({ tileScale: next as number })}
                    slotProps={{ input: { 'aria-label': t('settings.wallpaper.tileScale', 'Tile size'), 'data-testid': 'settings-wallpaper-tile-scale' } as never }}
                  />
                </div>
              ) : null}

              {fitCrops(fit) && !centred ? (
                <div>
                  <Button
                    size="small"
                    variant="text"
                    startIcon={<LocateFixed size={14} />}
                    onClick={() => wallpaperStore.setPreferences({ focusX: 50, focusY: 50 })}
                    data-testid="settings-wallpaper-recenter"
                  >
                    {t('settings.wallpaper.recenter', 'Center the picture')}
                  </Button>
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </Section>
  )
}
