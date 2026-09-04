import clsx from 'clsx'
import type {
  LayoutState,
  SystemPreferencesInput,
  ThemeMode,
  WindowAppearancePreferences,
  WindowLaunch,
} from '../../models/ui'
import { AppContentRenderer } from '../../app/registry'
import type { DesktopAppItem } from '../../app/types'
import {
  WindowDialogProvider,
  resolveWindowDialogPermissions,
} from './dialogs'

export function MobileWindowSheet({
  activityLog,
  app,
  deadZone,
  layoutState,
  locale,
  onSaveSettings,
  runtimeContainer,
  safeAreaBottom = 0,
  themeMode,
  topInset,
  windowAppearance,
  windowId,
  launch,
}: {
  activityLog: string[]
  app?: DesktopAppItem
  windowId?: string
  launch?: WindowLaunch
  deadZone: LayoutState['deadZone']
  layoutState: LayoutState
  locale: string
  onSaveSettings: (values: SystemPreferencesInput) => void
  runtimeContainer: string
  safeAreaBottom?: number
  themeMode: ThemeMode
  topInset: number
  windowAppearance: WindowAppearancePreferences
}) {
  if (!app) {
    return null
  }

  const hasFullBleedContent = app.manifest.contentPadding === 'none'

  return (
    <div className="absolute inset-0 z-40 overflow-hidden bg-[color:color-mix(in_srgb,var(--cp-bg)_94%,var(--cp-surface))]">
      <WindowDialogProvider
        insets={{
          top: topInset,
          right: 0,
          bottom: safeAreaBottom + deadZone.bottom,
          left: 0,
        }}
        permissions={resolveWindowDialogPermissions(app)}
        surface="mobile"
      >
        <div
          className="flex h-full min-h-0 flex-col"
          style={{
            paddingBottom: safeAreaBottom + deadZone.bottom,
          }}
        >
          <div
            className={clsx(
              'desktop-scrollbar min-h-0 flex-1',
              hasFullBleedContent ? 'overflow-hidden p-0' : 'overflow-y-auto p-4',
            )}
            style={
              hasFullBleedContent
                ? { paddingTop: topInset }
                : { paddingTop: topInset > 0 ? topInset + 14 : 14 }
            }
          >
            <AppContentRenderer
              activityLog={activityLog}
              app={app}
              windowId={windowId}
              launch={launch}
              layoutState={layoutState}
              locale={locale}
              onSaveSettings={onSaveSettings}
              runtimeContainer={runtimeContainer}
              themeMode={themeMode}
              windowAppearance={windowAppearance}
            />
          </div>
        </div>
      </WindowDialogProvider>
    </div>
  )
}
