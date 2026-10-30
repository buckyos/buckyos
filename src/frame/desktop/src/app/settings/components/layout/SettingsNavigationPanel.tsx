import { settingsSearchTerms } from './search'
import { Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { useViewerIsAdmin } from '../../hooks/use-viewer-is-admin'
import {
  settingsPageDefinitions,
  type SettingsPage,
} from './navigation'

interface SettingsNavigationPanelProps {
  currentPage?: SettingsPage | null
  onNavigate: (page: SettingsPage) => void
}

export function SettingsNavigationPanel({
  currentPage = null,
  onNavigate,
}: SettingsNavigationPanelProps) {
  const { t } = useI18n()
  const [query, setQuery] = useState('')
  const normalizedQuery = query.trim().toLowerCase()
  const isAdmin = useViewerIsAdmin()

  const visibleItems = useMemo(
    () =>
      settingsPageDefinitions.map((item) => ({
        ...item,
        matches: normalizedQuery ? settingsSearchTerms[item.key]
          .filter(([key, fallback]) => [t(key, fallback), fallback].some((text) => text.toLowerCase().includes(normalizedQuery)))
          .map(([key, fallback]) => t(key, fallback)) : [],
      })).filter((item) => {
        if ('adminOnly' in item && item.adminOnly && !isAdmin) {
          return false
        }
        if (!normalizedQuery) {
          return true
        }

        const label = t(item.labelKey, item.label).toLowerCase()
        return label.includes(normalizedQuery) || item.label.toLowerCase().includes(normalizedQuery) || item.matches.length > 0
      }),
    [isAdmin, normalizedQuery, t],
  )

  return (
    <>
      <label
        className="shell-subtle-panel flex items-center gap-2.5 px-3 py-2.5"
        style={{ color: 'var(--cp-muted)' }}
      >
        <Search size={16} />
        <input
          type="search"
          aria-label={t('settings.searchPlaceholder', 'Search settings')}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t('settings.searchPlaceholder', 'Search settings')}
          className="w-full bg-transparent text-sm outline-none placeholder:text-[color:var(--cp-muted)]"
          style={{ color: 'var(--cp-text)' }}
        />
      </label>
      <div className="mt-2 space-y-0.5">
        {visibleItems.length > 0 ? (
          visibleItems.map((item) => {
            const active = currentPage === item.key

            return (
              <button
                key={item.key}
                type="button"
                onClick={() => onNavigate(item.key)}
                aria-current={active ? 'page' : undefined}
                className="flex min-h-11 w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left text-sm transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-[var(--cp-accent)]"
                style={{
                  background: 'transparent',
                  color: 'var(--cp-text)',
                  border: 'none',
                }}
              >
                <div
                  className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl"
                  style={{
                    background: active
                      ? 'color-mix(in srgb, var(--cp-accent) 16%, transparent)'
                      : 'color-mix(in srgb, var(--cp-surface) 84%, transparent)',
                    color: active ? 'var(--cp-accent)' : 'var(--cp-muted)',
                  }}
                >
                  <item.icon size={16} />
                </div>
                <span className="min-w-0">
                  <span className="block font-medium">{t(item.labelKey, item.label)}</span>
                  {item.matches.length > 0 && (
                    <span className="mt-0.5 block text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
                      {item.matches.join(' · ')}
                    </span>
                  )}
                </span>
              </button>
            )
          })
        ) : (
          <div
            className="shell-subtle-panel px-4 py-4 text-sm leading-6"
            style={{ color: 'var(--cp-muted)' }}
          >
            {t('settings.searchEmpty', 'No matching settings')}
          </div>
        )}
      </div>
    </>
  )
}
