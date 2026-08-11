import clsx from 'clsx'
import { X } from 'lucide-react'
import type {
  CSSProperties,
  PointerEvent as ReactPointerEvent,
  ReactNode,
} from 'react'
import { AppIcon } from '../../components/DesktopVisuals'
import { useI18n } from '../../i18n/provider'
import type { ThemeMode, WindowAppearancePreferences } from '../../models/ui'
import {
  WindowDialogProvider,
  resolveWindowDialogPermissions,
} from './dialogs'
import type { DesktopWindowDataModel, ResizeDirection } from './types'

function WindowMinimizeIcon() {
  return (
    <svg
      viewBox="0 0 10 10"
      aria-hidden="true"
      className="h-2.5 w-2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.15"
      strokeLinecap="square"
    >
      <path d="M1.5 5.5h7" />
    </svg>
  )
}

function WindowMaximizeIcon() {
  return (
    <svg
      viewBox="0 0 10 10"
      aria-hidden="true"
      className="h-2.5 w-2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.05"
      strokeLinecap="square"
      strokeLinejoin="miter"
    >
      <rect x="1.5" y="1.5" width="7" height="7" />
    </svg>
  )
}

function WindowRestoreIcon() {
  return (
    <svg
      viewBox="0 0 10 10"
      aria-hidden="true"
      className="h-2.5 w-2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.05"
      strokeLinecap="square"
      strokeLinejoin="miter"
    >
      <path d="M3 1.5h5.5V7" />
      <path d="M1.5 3h5.5v5.5H1.5z" />
    </svg>
  )
}

function WindowChromeButton({
  ariaLabel,
  children,
  className,
  onClick,
}: {
  ariaLabel: string
  children: ReactNode
  className?: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      className={clsx(
        'flex h-6 w-6 items-center justify-center border-0 bg-transparent text-[color:var(--cp-muted)] transition-colors duration-150 hover:[background:linear-gradient(to_top,color-mix(in_srgb,var(--cp-text)_8%,transparent),transparent)] hover:text-[color:var(--cp-text)] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-[color:var(--cp-accent)]',
        className,
      )}
      onPointerDown={(event) => event.stopPropagation()}
      onClick={(event) => {
        event.stopPropagation()
        onClick()
      }}
    >
      {children}
    </button>
  )
}

function shouldIgnoreWindowFocus(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) {
    return false
  }

  return Boolean(
    target.closest(
      'button, input, select, textarea, a, [role="button"], [role="tab"], [role="switch"], [role="checkbox"], [role="radio"], [role="slider"], [contenteditable="true"], .MuiButtonBase-root, .MuiInputBase-root',
    ),
  )
}

function clampOpacityPercent(value: number) {
  return Math.min(Math.max(value, 0), 100)
}

function mixWithTransparency(color: string, opacityPercent: number) {
  return `color-mix(in srgb, ${color} ${clampOpacityPercent(opacityPercent)}%, transparent)`
}

export function DesktopWindowContainer({
  children,
  isFront,
  onClose,
  onDragPointerDown,
  onFocus,
  onMaximize,
  onMinimize,
  onResizePointerDown,
  style,
  themeMode,
  uiModel,
  windowAppearance,
}: {
  children: ReactNode
  isFront: boolean
  onClose: () => void
  onDragPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void
  onFocus: () => void
  onMaximize: () => void
  onMinimize: () => void
  onResizePointerDown: (
    direction: ResizeDirection,
  ) => (event: ReactPointerEvent<HTMLDivElement>) => void
  style: CSSProperties
  themeMode: ThemeMode
  uiModel: DesktopWindowDataModel
  windowAppearance: WindowAppearancePreferences
}) {
  const { t } = useI18n()
  const { app } = uiModel
  const isMaximized = uiModel.state === 'maximized'
  const hasFullBleedContent = app.manifest.contentPadding === 'none'
  const titleBarOpacity = clampOpacityPercent(windowAppearance.titleBarOpacity)
  const backgroundOpacity = clampOpacityPercent(windowAppearance.backgroundOpacity)
  const activeTitleBarMix =
    themeMode === 'light'
      ? {
          start: `color-mix(in srgb, ${app.accent} 38%, var(--cp-surface-2-opaque))`,
          end: `color-mix(in srgb, ${app.accent} 22%, var(--cp-surface-opaque))`,
          border: `color-mix(in srgb, ${app.accent} 32%, var(--cp-border-opaque))`,
        }
      : {
          start: `color-mix(in srgb, ${app.accent} 20%, var(--cp-surface-2-opaque))`,
          end: `color-mix(in srgb, ${app.accent} 9%, var(--cp-surface-opaque))`,
          border: `color-mix(in srgb, ${app.accent} 18%, var(--cp-border-opaque))`,
        }
  const titleBarStyle = isFront
    ? {
        background: `linear-gradient(180deg, ${mixWithTransparency(activeTitleBarMix.start, titleBarOpacity)}, ${mixWithTransparency(activeTitleBarMix.end, titleBarOpacity)})`,
        borderBottomColor: mixWithTransparency(activeTitleBarMix.border, titleBarOpacity),
      }
    : {
        background:
          `linear-gradient(180deg, ${mixWithTransparency('var(--cp-surface-2-opaque)', titleBarOpacity)}, ${mixWithTransparency('var(--cp-surface-opaque)', titleBarOpacity)})`,
      }
  const windowStyle = {
    ...style,
    background: `linear-gradient(180deg, ${mixWithTransparency('var(--cp-surface-opaque)', backgroundOpacity)}, ${mixWithTransparency('var(--cp-surface-2-opaque)', backgroundOpacity)})`,
  } satisfies CSSProperties
  const activeTitleTextColor =
    themeMode === 'light'
      ? 'color-mix(in srgb, var(--cp-text) 96%, black)'
      : 'var(--cp-text)'
  const inactiveTitleTextColor =
    'color-mix(in srgb, var(--cp-text) 88%, var(--cp-muted))'
  const titleTextColor = isFront ? activeTitleTextColor : inactiveTitleTextColor
  const activeChromeButtonClass =
    themeMode === 'light'
      ? 'text-[color:color-mix(in_srgb,var(--cp-text)_82%,black)] hover:[background:linear-gradient(to_top,color-mix(in_srgb,var(--cp-text)_10%,transparent),transparent)] hover:text-[color:color-mix(in_srgb,var(--cp-text)_96%,black)]'
      : 'text-[color:color-mix(in_srgb,var(--cp-text)_82%,var(--cp-muted))]'

  return (
    <div
      data-testid={`window-${app.id}`}
      className={clsx(
        'pointer-events-auto shell-window absolute flex flex-col overflow-hidden rounded-[12px] border border-[color:var(--cp-border)] transition-[transform,box-shadow,opacity] duration-200 ease-[var(--cp-ease-emphasis)]',
        isFront ? 'opacity-100' : 'opacity-[0.97]',
      )}
      style={windowStyle}
      onMouseDown={(event) => {
        if (shouldIgnoreWindowFocus(event.target)) {
          return
        }

        onFocus()
      }}
    >
      <WindowDialogProvider
        permissions={resolveWindowDialogPermissions(app)}
        surface="desktop"
      >
        <div
          data-testid={`window-drag-${app.id}`}
          className="flex h-8 cursor-move items-center justify-between gap-2 px-2 py-0.5 pl-2 pr-1"
          style={titleBarStyle}
          onPointerDown={onDragPointerDown}
        >
          <div className="min-w-0 flex items-center gap-2">
            <span
              className="flex size-4 shrink-0 items-center justify-center"
              style={{ color: titleTextColor }}
            >
              <AppIcon
                iconKey={app.iconKey}
                className={clsx(
                  'size-[12px] text-inherit',
                  themeMode === 'light' ? 'stroke-[1.9]' : 'stroke-[1.6]',
                )}
              />
            </span>
            <p
              className="truncate text-xs font-medium"
              style={{ color: titleTextColor }}
            >
              {t(app.labelKey)}
            </p>
          </div>
          <div
            className="flex items-center gap-px"
            onPointerDown={(event) => event.stopPropagation()}
          >
            {app.manifest.allowMinimize ? (
              <WindowChromeButton
                ariaLabel={t('common.minimize')}
                className={isFront ? activeChromeButtonClass : undefined}
                onClick={onMinimize}
              >
                <WindowMinimizeIcon />
              </WindowChromeButton>
            ) : null}
            {app.manifest.allowMaximize ? (
              <WindowChromeButton
                ariaLabel={
                  isMaximized
                    ? t('common.restoreWindow')
                    : t('common.maximize')
                }
                className={isFront ? activeChromeButtonClass : undefined}
                onClick={onMaximize}
              >
                {isMaximized ? <WindowRestoreIcon /> : <WindowMaximizeIcon />}
              </WindowChromeButton>
            ) : null}
            <WindowChromeButton
              ariaLabel={t('common.close')}
              className={isFront ? activeChromeButtonClass : undefined}
              onClick={onClose}
            >
              <X className="size-[12px] stroke-[2]" />
            </WindowChromeButton>
          </div>
        </div>

        <div
          className={clsx(
            'desktop-scrollbar min-h-0 flex-1',
            hasFullBleedContent ? 'overflow-hidden p-0' : 'overflow-y-auto p-5',
          )}
        >
          {children}
        </div>

        {!isMaximized ? (
          <>
            <div
              data-testid={`window-resize-left-${app.id}`}
              className="absolute inset-y-0 left-0 z-20 w-1.5 cursor-ew-resize"
              onPointerDown={onResizePointerDown('left')}
            />
            <div
              data-testid={`window-resize-right-${app.id}`}
              className="absolute inset-y-0 right-0 z-20 w-1.5 cursor-ew-resize"
              onPointerDown={onResizePointerDown('right')}
            />
            <div
              data-testid={`window-resize-bottom-${app.id}`}
              className="absolute bottom-0 left-0 right-0 z-20 h-1.5 cursor-ns-resize"
              onPointerDown={onResizePointerDown('bottom')}
            />
            <div
              data-testid={`window-resize-bottom-left-${app.id}`}
              className="absolute bottom-0 left-0 z-30 h-3.5 w-3.5 cursor-nesw-resize"
              onPointerDown={onResizePointerDown('bottom-left')}
            />
            <div
              data-testid={`window-resize-bottom-right-${app.id}`}
              className="absolute bottom-0 right-0 z-30 h-3.5 w-3.5 cursor-nwse-resize"
              onPointerDown={onResizePointerDown('bottom-right')}
            />
          </>
        ) : null}
      </WindowDialogProvider>
    </div>
  )
}
