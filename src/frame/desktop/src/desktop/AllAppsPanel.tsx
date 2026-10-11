import clsx from 'clsx'
import { Pin, Search, X } from 'lucide-react'
import {
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type Ref,
} from 'react'
import type { DesktopAppItem } from '../app/types'
import { AppIcon } from '../components/DesktopVisuals'
import { appIconSurfaceStyle } from '../components/DesktopVisualTokens'
import { useI18n } from '../i18n/provider'
import { isLauncherApp } from '../models/ui'
import { taskbarPopupSurface } from './DesktopTaskbarCards'

/**
 * Every installed app as a searchable list. On the desktop it slides out of
 * the taskbar (`panel`); on mobile it covers the screen (`fullscreen`).
 * Mount it only while open: the search starts empty each time.
 */
export function AllAppsPanel({
  apps,
  insets = { top: 0, bottom: 0 },
  onAppContextMenu,
  onClose,
  onOpenApp,
  panelRef,
  pinnedAppIds,
  style,
  variant,
}: {
  apps: DesktopAppItem[]
  /** Safe-area / dead-zone space the fullscreen variant keeps clear. */
  insets?: { top: number; bottom: number }
  onAppContextMenu?: (appId: string, event: ReactMouseEvent) => void
  onClose: () => void
  onOpenApp: (appId: string) => void
  panelRef?: Ref<HTMLDivElement>
  pinnedAppIds?: ReadonlySet<string>
  style?: CSSProperties
  variant: 'panel' | 'fullscreen'
}) {
  const { locale, t } = useI18n()
  const [query, setQuery] = useState('')
  const listRef = useRef<HTMLUListElement | null>(null)
  const fullscreen = variant === 'fullscreen'

  const sortedApps = useMemo(() => {
    const collator = new Intl.Collator(locale, { sensitivity: 'base', numeric: true })
    return apps
      .filter(isLauncherApp)
      .map((app) => ({ app, label: t(app.labelKey), summary: t(app.summaryKey, '') }))
      .sort((a, b) => collator.compare(a.label, b.label))
  }, [apps, locale, t])

  const needle = query.trim().toLocaleLowerCase(locale)
  // Names starting with the query come first, then names containing it,
  // then apps that only match by id or description.
  const results = needle
    ? sortedApps
        .map((entry) => {
          const label = entry.label.toLocaleLowerCase(locale)
          const rank = label.startsWith(needle)
            ? 0
            : label.includes(needle)
              ? 1
              : [entry.app.id, entry.summary].some((text) => text.toLocaleLowerCase(locale).includes(needle))
                ? 2
                : -1
          return { entry, rank }
        })
        .filter(({ rank }) => rank >= 0)
        .sort((a, b) => a.rank - b.rank)
        .map(({ entry }) => entry)
    : sortedApps

  const focusItem = (index: number) => {
    const buttons = listRef.current?.querySelectorAll<HTMLButtonElement>('button[data-app-id]')
    if (!buttons || buttons.length === 0) return
    buttons[Math.max(0, Math.min(index, buttons.length - 1))].focus()
  }

  const handleSearchKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter' && results[0]) {
      event.preventDefault()
      onOpenApp(results[0].app.id)
    } else if (event.key === 'ArrowDown') {
      event.preventDefault()
      focusItem(0)
    } else if (event.key === 'Escape' && query) {
      // The first Escape clears the search; the next one closes the panel.
      event.stopPropagation()
      event.nativeEvent.stopImmediatePropagation()
      setQuery('')
    }
  }

  const handleListKeyDown = (event: ReactKeyboardEvent<HTMLUListElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    const buttons = Array.from(listRef.current?.querySelectorAll<HTMLButtonElement>('button[data-app-id]') ?? [])
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
    if (index < 0) return
    event.preventDefault()
    if (event.key === 'ArrowUp' && index === 0) {
      listRef.current?.closest('[data-all-apps]')?.querySelector<HTMLInputElement>('input')?.focus()
      return
    }
    focusItem(index + (event.key === 'ArrowDown' ? 1 : -1))
  }

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={t('shell.allApps', 'All apps')}
      data-all-apps=""
      data-testid="all-apps-panel"
      className={clsx(
        'flex flex-col text-[color:var(--cp-text)]',
        fullscreen
          ? 'fixed inset-0 z-[80] bg-[color:var(--cp-bg)]'
          : clsx('absolute z-[54] rounded-r-[20px] border-l-0 [animation:cp-taskbar-pop_200ms_var(--cp-ease-emphasis)]', taskbarPopupSurface),
      )}
      style={{
        ...(fullscreen ? { paddingTop: insets.top, paddingBottom: insets.bottom } : {}),
        ...style,
      }}
    >
      <div className={clsx('flex shrink-0 items-center gap-2', fullscreen ? 'px-4 pb-2 pt-3' : 'px-3 pb-2 pt-3')}>
        <label
          className={clsx(
            'flex min-w-0 flex-1 items-center gap-2 rounded-[12px] border border-[color:color-mix(in_srgb,var(--cp-border)_88%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_80%,transparent)] px-3 transition-[border-color] focus-within:border-[color:color-mix(in_srgb,var(--cp-accent)_42%,var(--cp-border))]',
            fullscreen ? 'h-11' : 'h-9',
          )}
        >
          <Search className="size-4 shrink-0 text-[color:var(--cp-muted)]" />
          <input
            // The panel opens on request; the search is where typing goes.
            autoFocus
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={handleSearchKeyDown}
            placeholder={t('shell.searchApps', 'Search apps')}
            aria-label={t('shell.searchApps', 'Search apps')}
            data-testid="all-apps-search"
            className="min-w-0 flex-1 bg-transparent text-[13px] outline-none placeholder:text-[color:var(--cp-muted)] focus-visible:shadow-none [&::-webkit-search-cancel-button]:hidden"
          />
          {query ? (
            <button
              type="button"
              aria-label={t('shell.clearSearch', 'Clear search')}
              onClick={() => setQuery('')}
              className="inline-flex size-5 shrink-0 items-center justify-center rounded-full text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </label>
        {fullscreen ? (
          <button
            type="button"
            aria-label={t('common.close')}
            onClick={onClose}
            className="inline-flex size-11 shrink-0 items-center justify-center rounded-full text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
          >
            <X className="size-5" />
          </button>
        ) : null}
      </div>
      <p className={clsx('shrink-0 pb-1 text-[11px] font-medium text-[color:var(--cp-muted)]', fullscreen ? 'px-5' : 'px-4')}>
        {needle
          ? t('shell.allAppsMatches', '{{count}} matching', { count: results.length })
          : t('shell.allAppsCount', 'All apps · {{count}}', { count: results.length })}
      </p>
      {results.length > 0 ? (
        <ul
          ref={listRef}
          onKeyDown={handleListKeyDown}
          className={clsx('desktop-scrollbar min-h-0 flex-1 overflow-y-auto', fullscreen ? 'px-2 pb-4' : 'px-1.5 pb-3')}
        >
          {results.map(({ app, label, summary }) => (
            <li key={app.id}>
              <button
                type="button"
                data-app-id={app.id}
                data-testid={`all-apps-item-${app.id}`}
                onClick={() => onOpenApp(app.id)}
                onContextMenu={
                  onAppContextMenu
                    ? (event) => {
                        event.preventDefault()
                        onAppContextMenu(app.id, event)
                      }
                    : undefined
                }
                className={clsx(
                  'flex w-full min-w-0 items-center gap-3 rounded-[12px] px-2.5 text-left transition-colors hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] focus-visible:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)]',
                  fullscreen ? 'py-2.5' : 'py-2',
                )}
              >
                <span
                  className="flex shrink-0 items-center justify-center overflow-hidden rounded-[10px]"
                  style={{
                    width: fullscreen ? 40 : 32,
                    height: fullscreen ? 40 : 32,
                    '--icon-size': `${fullscreen ? 40 : 32}px`,
                    ...appIconSurfaceStyle(app.accent),
                  } as CSSProperties}
                >
                  <AppIcon iconKey={app.iconKey} iconUrl={app.iconUrl} fill className="text-white" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className={clsx('block truncate font-medium', fullscreen ? 'text-[15px]' : 'text-[13px]')}>
                    {label}
                  </span>
                  {summary ? (
                    <span className="block truncate text-[11px] text-[color:var(--cp-muted)]">{summary}</span>
                  ) : null}
                </span>
                {pinnedAppIds?.has(app.id) ? (
                  <Pin
                    aria-label={t('shell.pinnedToTaskbar', 'Pinned to taskbar')}
                    className="size-3.5 shrink-0 text-[color:var(--cp-muted)]"
                  />
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="flex-1 px-5 py-6 text-center text-[13px] text-[color:var(--cp-muted)]">
          {t('shell.noAppsMatch', 'No apps match "{{query}}"', { query: query.trim() })}
        </p>
      )}
    </div>
  )
}
