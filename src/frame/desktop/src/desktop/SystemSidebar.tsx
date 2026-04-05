import clsx from 'clsx'
import {
  House,
  ShieldAlert,
  ShieldCheck,
  ShieldX,
} from 'lucide-react'
import { AppIcon } from '../components/DesktopVisuals'
import { useI18n } from '../i18n/provider'
import type { LayoutState, SystemSidebarDataModel } from '../models/ui'
import {
  connectionLabel,
  type ConnectionState,
} from './shell'

const SYSTEM_SIDEBAR_WIDTH = Math.round(390 * 0.37)

export function SystemSidebar({
  connectionState,
  deadZone,
  onClose,
  onOpenApp,
  onReturnDesktop,
  open,
  runtimeContainer,
  safeAreaTop = 0,
  safeAreaBottom = 0,
  uiModel,
}: {
  connectionState: ConnectionState
  deadZone: LayoutState['deadZone']
  onClose: () => void
  onOpenApp: (appId: string) => void
  onReturnDesktop: () => void
  open: boolean
  runtimeContainer: string
  safeAreaTop?: number
  safeAreaBottom?: number
  uiModel: SystemSidebarDataModel
}) {
  const { t } = useI18n()
  const connectionIcon =
    connectionState === 'online'
      ? ShieldCheck
      : connectionState === 'degraded'
        ? ShieldAlert
        : ShieldX
  const ConnectionIcon = connectionIcon
  const hasSwitchApps = uiModel.switchApps.length > 0

  const itemClassName =
    'flex w-full min-w-0 items-center gap-2.5 py-1.5 text-left text-[13px] leading-5 transition-opacity duration-150 ease-[var(--cp-ease-emphasis)] hover:opacity-100 active:scale-[0.99]'

  const renderAppRow = (
    app: SystemSidebarDataModel['switchApps'][number] | SystemSidebarDataModel['systemApps'][number],
  ) => {
    const isCurrent = uiModel.currentAppId === app.appId

    return (
      <button
        key={app.appId}
        type="button"
        onClick={() => onOpenApp(app.appId)}
        className={clsx(
          itemClassName,
          isCurrent
            ? 'font-medium text-[color:var(--cp-text)] opacity-100'
            : 'text-[color:var(--cp-muted)] opacity-90',
        )}
      >
        <AppIcon
          iconKey={app.iconKey}
          className={clsx(
            'size-4 shrink-0 sm:size-4',
            isCurrent ? 'text-[color:var(--cp-text)]' : 'text-[color:var(--cp-muted)]',
          )}
        />
        <span className="truncate">{t(app.labelKey)}</span>
      </button>
    )
  }

  return (
    <div
      className={clsx(
        'absolute inset-0 z-[60] transition-opacity duration-200 ease-[var(--cp-ease-emphasis)]',
        open ? 'pointer-events-auto opacity-100' : 'pointer-events-none opacity-0',
      )}
      aria-hidden={!open}
    >
      <button
        type="button"
        aria-label={t('common.cancel')}
        onClick={onClose}
        className="absolute inset-0 bg-[color:color-mix(in_srgb,var(--cp-shadow)_32%,transparent)] backdrop-blur-[2px]"
      />
      <aside
        className={clsx(
          'absolute inset-y-0 left-0 border-r border-[color:color-mix(in_srgb,var(--cp-border)_88%,transparent)] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface)_98%,transparent),color-mix(in_srgb,var(--cp-surface-2)_96%,transparent))] shadow-[0_18px_56px_color-mix(in_srgb,var(--cp-shadow)_18%,transparent)] backdrop-blur-xl transition-transform duration-250 ease-[var(--cp-ease-emphasis)]',
          open ? 'translate-x-0' : '-translate-x-full',
        )}
        style={{ width: SYSTEM_SIDEBAR_WIDTH }}
      >
        <div
          className="desktop-scrollbar flex h-full flex-col gap-3 overflow-y-auto px-3 pb-4 pt-3"
          style={{
            paddingTop: safeAreaTop + deadZone.top + 10,
            paddingBottom: safeAreaBottom + deadZone.bottom + 12,
          }}
        >

          <section className="rounded-[8px] border border-[color:color-mix(in_srgb,var(--cp-border)_66%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_82%,transparent)] px-3 py-2.5">
            <p className="truncate font-display text-[15px] font-semibold text-[color:var(--cp-text)]">
              waterflier
            </p>
            <p className="mt-0.5 truncate text-[12px] text-[color:var(--cp-muted)]">
              {t(`runtime.${runtimeContainer}`, runtimeContainer)}
            </p>
            <div className="mt-2 flex min-w-0 items-center gap-2 text-[11px] text-[color:var(--cp-muted)]">
              <ConnectionIcon className="size-3.5 shrink-0" />
              <span className="truncate">{connectionLabel(connectionState, t)}</span>
            </div>
          </section>

          <div className="h-px bg-[color:color-mix(in_srgb,var(--cp-border)_72%,transparent)]" />

          <button
            type="button"
            onClick={onReturnDesktop}
            className={clsx(itemClassName, 'font-medium text-[color:var(--cp-text)]')}
          >
            <House className="size-4 shrink-0 text-[color:var(--cp-muted)]" />
            <span className="truncate">
              {t('shell.returnDesktop', 'Desktop')}
            </span>
          </button>

          {hasSwitchApps ? (
            <>
              <div className="h-px bg-[color:color-mix(in_srgb,var(--cp-border)_72%,transparent)]" />
              <div className="flex flex-col">
                {uiModel.switchApps.map(renderAppRow)}
              </div>
            </>
          ) : null}

          <div className="h-px bg-[color:color-mix(in_srgb,var(--cp-border)_72%,transparent)]" />

          <div className="flex flex-col">
            {uiModel.systemApps.map(renderAppRow)}
          </div>
        </div>
      </aside>
    </div>
  )
}
