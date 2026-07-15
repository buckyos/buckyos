import { NavLink } from 'react-router-dom'
import { getNavigationItems } from './navigation'
import { useI18n } from '../i18n/provider'
import { useProviderMetadataStore } from '../state/useProviderMetadataStore'

export function MobileNav() {
  const { t } = useI18n()
  const { serviceRole } = useProviderMetadataStore()
  const items = getNavigationItems(serviceRole).slice(0, 5)

  return (
    <nav className="grid grid-cols-5 border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] md:hidden">
      {items.map((item) => {
        const Icon = item.icon
        return (
          <NavLink
            className={({ isActive }) =>
              `flex min-h-14 flex-col items-center justify-center gap-1 px-1 text-[10px] font-semibold ${isActive ? 'text-[color:var(--cp-accent)]' : 'text-[color:var(--cp-muted)]'}`
            }
            key={item.id}
            to={item.path}
          >
            <Icon size={17} />
            <span className="max-w-full truncate">{t(item.labelKey, item.id)}</span>
          </NavLink>
        )
      })}
    </nav>
  )
}
