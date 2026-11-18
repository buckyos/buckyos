import { Dialog, DialogContent, DialogTitle, IconButton, Tab, Tabs } from '@mui/material'
import clsx from 'clsx'
import { Clock, Info, LayoutGrid, StickyNote, X, type LucideIcon } from 'lucide-react'
import { useId, useMemo, useState, type CSSProperties } from 'react'
import type { DesktopAppItem } from '../app/types'
import { AppIcon } from '../components/DesktopVisuals'
import { appIconSurfaceStyle } from '../components/DesktopVisualTokens'
import { useI18n } from '../i18n/provider'
import { isLauncherApp, type LayoutState } from '../models/ui'
import { useAppText } from '../presentation/appText'
import { widgetCatalog } from './widgets/catalog'

const widgetIcons: Record<string, LucideIcon> = {
  clock: Clock,
  notepad: StickyNote,
  'system-info': Info,
}

type AddTab = 'widgets' | 'apps'

/**
 * "Add…" from the desktop's menu: the widgets and the apps that can go on the desktop, one tab each. What is on the
 * desktop already is shown but cannot be chosen (except a widget the desktop takes more than one of, a note).
 * Choosing one adds it where the menu was opened and closes the dialog.
 */
export function AddToDesktopDialog({
  apps,
  isMobile,
  layout,
  onAddApp,
  onAddWidget,
  onClose,
  open,
}: {
  apps: DesktopAppItem[]
  isMobile: boolean
  layout: LayoutState | null
  onAddApp: (appId: string, label: string) => void
  onAddWidget: (widgetType: string) => void
  onClose: () => void
  open: boolean
}) {
  const { locale, t } = useI18n()
  const appText = useAppText()
  const titleId = useId()
  const [tab, setTab] = useState<AddTab>('widgets')

  const onDesktop = useMemo(() => {
    const items = layout?.pages.flatMap((page) => page.items) ?? []
    return {
      apps: new Set(items.flatMap((item) => (item.type === 'app' ? [item.appId] : []))),
      widgets: new Set(items.flatMap((item) => (item.type === 'widget' ? [item.widgetType] : []))),
    }
  }, [layout])

  const sortedApps = useMemo(() => {
    const collator = new Intl.Collator(locale, { sensitivity: 'base', numeric: true })
    return apps
      .filter(isLauncherApp)
      .map((app) => ({ app, label: appText(app).title }))
      .sort((a, b) => collator.compare(a.label, b.label))
  }, [apps, locale, appText])

  const addedLabel = t('desktop.add.added', 'On the desktop')
  const addableApps = sortedApps.filter(({ app }) => !onDesktop.apps.has(app.id))
  const addedApps = sortedApps.filter(({ app }) => onDesktop.apps.has(app.id))

  return (
    <Dialog
      open={open}
      onClose={onClose}
      fullScreen={isMobile}
      fullWidth
      maxWidth="sm"
      aria-labelledby={titleId}
      data-testid="desktop-add-dialog"
      slotProps={{
        paper: {
          sx: isMobile
            ? { paddingTop: 'env(safe-area-inset-top)', paddingBottom: 'env(safe-area-inset-bottom)' }
            : { height: 'min(560px, calc(100dvh - 64px))' },
        },
      }}
    >
      <DialogTitle id={titleId} className="flex items-center justify-between gap-2 !pb-0">
        <span>{t('desktop.add.title', 'Add to desktop')}</span>
        <IconButton aria-label={t('common.close', 'Close')} onClick={onClose} edge="end" data-testid="desktop-add-close">
          <X className="size-5" />
        </IconButton>
      </DialogTitle>
      <Tabs
        value={tab}
        onChange={(_, next: AddTab) => setTab(next)}
        variant="fullWidth"
        className="shrink-0 border-b border-[color:var(--cp-border)] px-2"
      >
        <Tab value="widgets" label={t('desktop.add.widgets', 'Widgets')} data-testid="desktop-add-tab-widgets" />
        <Tab value="apps" label={t('desktop.add.apps', 'Apps')} data-testid="desktop-add-tab-apps" />
      </Tabs>
      <DialogContent className="desktop-scrollbar !px-3 !py-3">
        {tab === 'widgets' ? (
          <ul className="grid gap-2 sm:grid-cols-2" data-testid="desktop-add-widgets">
            {widgetCatalog.map((definition) => {
              const added = !definition.multiple && onDesktop.widgets.has(definition.type)
              const Icon = widgetIcons[definition.type] ?? LayoutGrid
              return (
                <li key={definition.type}>
                  <button
                    type="button"
                    disabled={added}
                    aria-disabled={added}
                    data-testid={`desktop-add-widget-${definition.type}`}
                    onClick={() => onAddWidget(definition.type)}
                    className={clsx(
                      'flex w-full items-start gap-3 rounded-[12px] border border-[color:var(--cp-border)] p-3 text-left transition-colors',
                      added
                        ? 'cursor-default opacity-55'
                        : 'hover:border-[color:color-mix(in_srgb,var(--cp-accent)_45%,transparent)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent)_6%,transparent)] focus-visible:border-[color:var(--cp-accent)]',
                    )}
                  >
                    <span className="flex size-10 shrink-0 items-center justify-center rounded-[10px] bg-[color:var(--cp-surface-2)] text-[color:var(--cp-text)]">
                      <Icon className="size-5" />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex items-baseline justify-between gap-2">
                        <span className="truncate text-sm font-medium text-[color:var(--cp-text)]">
                          {t(definition.labelKey, definition.label)}
                        </span>
                        <span className="shrink-0 text-xs tabular-nums text-[color:var(--cp-muted)]">
                          {definition.defaultSize.w} × {definition.defaultSize.h}
                        </span>
                      </span>
                      <span className="mt-0.5 block text-xs leading-5 text-[color:var(--cp-muted)]">
                        {added ? addedLabel : t(definition.descriptionKey, definition.description)}
                      </span>
                    </span>
                  </button>
                </li>
              )
            })}
          </ul>
        ) : sortedApps.length > 0 ? (
          <div className="space-y-4">
            {addableApps.length > 0 ? (
              <AppGrid apps={addableApps} isMobile={isMobile} onAddApp={onAddApp} testId="desktop-add-apps" />
            ) : (
              <p className="py-6 text-center text-sm text-[color:var(--cp-muted)]">{t('desktop.add.allAppsAdded', 'Every app is on the desktop')}</p>
            )}
            {addedApps.length > 0 ? (
              <section aria-label={addedLabel}>
                <h3 className="px-1 pb-1 text-xs font-medium text-[color:var(--cp-muted)]">{addedLabel}</h3>
                <AppGrid apps={addedApps} added isMobile={isMobile} testId="desktop-add-apps-added" />
              </section>
            ) : null}
          </div>
        ) : (
          <p className="py-8 text-center text-sm text-[color:var(--cp-muted)]">{t('desktop.add.noApps', 'No apps to add')}</p>
        )}
      </DialogContent>
    </Dialog>
  )
}

/** Apps as a grid of icons; the ones already on the desktop are dimmed and cannot be chosen. */
function AppGrid({
  added = false,
  apps,
  isMobile,
  onAddApp,
  testId,
}: {
  added?: boolean
  apps: Array<{ app: DesktopAppItem; label: string }>
  isMobile: boolean
  onAddApp?: (appId: string, label: string) => void
  testId: string
}) {
  return (
    <ul className={clsx('grid gap-1', isMobile ? 'grid-cols-4' : 'grid-cols-5')} data-testid={testId}>
      {apps.map(({ app, label }) => (
        <li key={app.id}>
          <button
            type="button"
            disabled={added}
            title={label}
            data-testid={`desktop-add-app-${app.id}`}
            onClick={() => onAddApp?.(app.id, label)}
            className={clsx(
              'flex w-full flex-col items-center gap-1.5 rounded-[12px] px-1 pb-2 pt-3 text-center transition-colors',
              added
                ? 'cursor-default'
                : 'hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)] focus-visible:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]',
            )}
          >
            <span
              className={clsx('flex shrink-0 items-center justify-center overflow-hidden', added && 'opacity-40')}
              style={{
                width: 44,
                height: 44,
                borderRadius: 12,
                '--icon-size': '44px',
                ...appIconSurfaceStyle(app.accent),
              } as CSSProperties}
            >
              <AppIcon iconKey={app.iconKey} iconUrl={app.iconUrl} fill className="text-white" />
            </span>
            <span
              className={clsx(
                'line-clamp-2 max-w-full break-words text-xs leading-4',
                added ? 'text-[color:var(--cp-muted)]' : 'text-[color:var(--cp-text)]',
              )}
            >
              {label}
            </span>
          </button>
        </li>
      ))}
    </ul>
  )
}
