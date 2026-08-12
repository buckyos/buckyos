import { ChevronLeft } from 'lucide-react'
import clsx from 'clsx'
import type { CSSProperties } from 'react'
import { AppIcon } from '../components/DesktopVisuals'
import { useI18n } from '../i18n/provider'
import { useMobileNavState } from './windows/MobileNavContext'

interface StandaloneAppTitleBarProps {
  titleKey: string
  summaryKey: string
  iconKey: string
  accent: string
  className?: string
  onBack?: () => void
}

export function StandaloneAppTitleBar({
  titleKey,
  summaryKey,
  iconKey,
  accent,
  className,
  onBack,
}: StandaloneAppTitleBarProps) {
  const { t } = useI18n()
  const mobileNav = useMobileNavState()
  const handleBack = mobileNav.canGoBack ? mobileNav.goBack : onBack

  return (
    <header
      className={clsx('shrink-0 border-b backdrop-blur-xl', className)}
      style={{
        paddingTop: 'env(safe-area-inset-top, 0px)',
        borderColor: 'var(--cp-border)',
        backgroundColor: `color-mix(in srgb, ${accent} 14%, var(--cp-surface-2))`,
      }}
    >
      <div className="relative flex h-[58px] items-center justify-between gap-3 px-3 text-[color:var(--cp-text)]">
        <div className="flex min-w-0 items-center gap-2">
          {handleBack ? (
            <button
              type="button"
              aria-label={t('common.back', 'Back')}
              onClick={handleBack}
              className="inline-flex h-9 w-9 items-center justify-center rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] shadow-[0_10px_24px_color-mix(in_srgb,var(--cp-shadow)_12%,transparent)] transition-transform duration-150 ease-[var(--cp-ease-emphasis)] active:scale-[0.96]"
            >
              <ChevronLeft size={20} />
            </button>
          ) : (
            <div
              className="flex h-9 w-9 items-center justify-center rounded-full border bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] shadow-[0_10px_24px_color-mix(in_srgb,var(--cp-shadow)_12%,transparent)]"
              style={
                {
                  '--icon-size': '36px',
                  borderColor: 'color-mix(in srgb, var(--cp-border) 82%, transparent)',
                } as CSSProperties
              }
            >
              <AppIcon iconKey={iconKey} className="text-[color:var(--cp-text)]" />
            </div>
          )}
        </div>

        <div className="absolute left-1/2 top-1/2 flex min-w-0 max-w-[58vw] -translate-x-1/2 -translate-y-1/2 flex-col items-center justify-center text-center">
          <p className="truncate font-display text-sm font-semibold text-[color:var(--cp-text)]">
            {mobileNav.titleOverride?.title ?? t(titleKey)}
          </p>
          <p className="line-clamp-1 text-xs text-[color:var(--cp-muted)]">
            {mobileNav.titleOverride?.subtitle ?? t(summaryKey)}
          </p>
        </div>
        <div className="h-9 w-9 shrink-0" />
      </div>
    </header>
  )
}
