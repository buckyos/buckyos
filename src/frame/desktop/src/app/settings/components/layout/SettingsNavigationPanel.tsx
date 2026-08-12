import { Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
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

  const visibleItems = useMemo(
    () =>
      settingsPageDefinitions.filter((item) => {
        if (!normalizedQuery) {
          return true
        }

        const label = t(item.labelKey, item.label).toLowerCase()
        return label.includes(normalizedQuery)
      }),
    [normalizedQuery, t],
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
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t('settings.searchPlaceholder', 'Search settings')}
          className="w-full bg-transparent text-sm outline-none placeholder:text-[color:var(--cp-muted)]"
          style={{ color: 'var(--cp-text)' }}
        />
      </label>
      <div className="mt-4 space-y-1">
        {visibleItems.length > 0 ? (
          visibleItems.map((item) => {
            const active = currentPage === item.key

            return (
              <button
                key={item.key}
                type="button"
                onClick={() => onNavigate(item.key)}
                className="flex w-full items-center gap-3 rounded-[18px] px-4 py-3 text-left text-sm transition-colors"
                style={{
                  background: active
                    ? 'color-mix(in srgb, var(--cp-accent-soft) 14%, var(--cp-surface-2))'
                    : 'transparent',
                  color: active ? 'var(--cp-text)' : 'var(--cp-muted)',
                  border: active
                    ? '1px solid color-mix(in srgb, var(--cp-accent) 22%, var(--cp-border))'
                    : '1px solid transparent',
                }}
              >
                <div
                  className="flex h-9 w-9 items-center justify-center rounded-[14px]"
                  style={{
                    background: active
                      ? 'color-mix(in srgb, var(--cp-accent) 16%, transparent)'
                      : 'color-mix(in srgb, var(--cp-surface) 84%, transparent)',
                    color: active ? 'var(--cp-accent)' : 'var(--cp-muted)',
                  }}
                >
                  <item.icon size={16} />
                </div>
                <span className="font-medium">{t(item.labelKey, item.label)}</span>
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
