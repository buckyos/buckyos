import { NavLink } from 'react-router-dom'
import { getNavigationItems } from './navigation'
import { useI18n } from '../i18n/provider'
import { useProviderMetadataStore } from '../state/useProviderMetadataStore'

export function Sidebar() {
  const { t } = useI18n()
  const { serviceRole } = useProviderMetadataStore()
  const items = getNavigationItems(serviceRole)

  return (
    <aside className="hidden w-60 shrink-0 border-r border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface)_82%,transparent)] p-3 md:block">
      <nav className="space-y-1">
        {items.map((item) => {
          const Icon = item.icon
          return (
            <NavLink
              className={({ isActive }) =>
                `flex items-center gap-3 rounded-md px-3 py-2.5 text-sm font-semibold ${isActive ? 'bg-[color:color-mix(in_srgb,var(--cp-accent)_14%,transparent)] text-[color:var(--cp-accent)]' : 'text-[color:var(--cp-muted)] hover:bg-[color:var(--cp-surface-2)]'}`
              }
              key={item.id}
              to={item.path}
            >
              <Icon size={17} />
              {t(item.labelKey, item.id)}
            </NavLink>
          )
        })}
      </nav>
    </aside>
  )
}
