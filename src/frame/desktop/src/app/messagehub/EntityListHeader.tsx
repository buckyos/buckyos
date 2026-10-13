import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Check, Loader2, MoreVertical, Pencil, Plus } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { HubAvatar } from './HubAvatar'
import type { EntityType } from './types'

/** Bare icons: the title bar leaves its width to the name. */
const headerButtonClass = 'flex h-11 w-9 shrink-0 items-center justify-center rounded-lg text-[color:var(--cp-muted)] transition-colors hover:text-[color:var(--cp-text)] disabled:opacity-40 disabled:hover:text-[color:var(--cp-muted)] md:h-8 md:w-8'

export interface HeaderMenuItem {
  key: string
  label: string
  icon: ReactNode
  onSelect: () => void
  disabled?: boolean
  testId?: string
}

/** Whose messages the list shows: the viewer, or the observed owner. */
export interface HubIdentity {
  name: string
  avatar?: string
  type: EntityType
}

function HeaderMenu({ label, icon, items, testId }: { label: string; icon: ReactNode; items: HeaderMenuItem[]; testId: string }) {
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    if (!open) return
    root.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus()
    const close = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false) }
    document.addEventListener('pointerdown', close)
    return () => document.removeEventListener('pointerdown', close)
  }, [open])
  if (items.length === 0) return null
  const moveFocus = (step: number) => {
    const entries = [...(root.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? [])]
    const index = entries.indexOf(document.activeElement as HTMLElement)
    entries[(index + step + entries.length) % entries.length]?.focus()
  }
  return (
    <div
      ref={root}
      className="relative shrink-0"
      onKeyDown={(event) => {
        if (!open) return
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setOpen(false); trigger.current?.focus() }
        else if (event.key === 'ArrowDown') { event.preventDefault(); moveFocus(1) }
        else if (event.key === 'ArrowUp') { event.preventDefault(); moveFocus(-1) }
      }}
    >
      <button
        ref={trigger}
        type="button"
        disabled={items.every(item => item.disabled)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={label}
        title={label}
        onClick={() => setOpen(value => !value)}
        className={headerButtonClass}
        data-testid={testId}
      >
        {icon}
      </button>
      {open ? (
        <div
          role="menu"
          aria-label={label}
          className="absolute right-0 top-full z-30 mt-1 min-w-48 overflow-hidden rounded-xl py-1 shadow-lg"
          style={{ background: 'var(--cp-surface-opaque, var(--cp-surface))', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
        >
          {items.map(item => (
            <button
              key={item.key}
              type="button"
              role="menuitem"
              disabled={item.disabled}
              data-testid={item.testId}
              className="flex min-h-11 w-full items-center gap-2.5 whitespace-nowrap px-3 text-left text-[13px] hover:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)] disabled:opacity-40 md:min-h-9"
              onClick={() => { setOpen(false); trigger.current?.focus(); item.onSelect() }}
            >
              <span className="flex w-4 shrink-0 justify-center" style={{ color: 'var(--cp-muted)' }} aria-hidden>{item.icon}</span>
              {item.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

/**
 * The list's title bar: edit on the left; the identity with its unread total in the middle,
 * replaced by the sync status while that is not online; new entries (+) and everything else (⋮) on the right.
 */
export function EntityListHeader({
  identity,
  unreadTotal,
  status,
  editing,
  selectedCount,
  canEdit,
  onToggleEditing,
  createItems,
  moreItems,
}: {
  identity: HubIdentity
  unreadTotal: number
  status?: string
  editing: boolean
  selectedCount: number
  canEdit: boolean
  onToggleEditing: () => void
  createItems: HeaderMenuItem[]
  moreItems: HeaderMenuItem[]
}) {
  const { t } = useI18n()
  const editLabel = t(editing ? 'messagehub.header.done' : 'messagehub.header.edit')
  const unreadLabel = t('messagehub.header.unread', undefined, { count: unreadTotal })
  return (
    <div className="mb-2 grid grid-cols-[1fr_minmax(0,max-content)_1fr] items-center gap-1" data-testid="entity-list-header">
      <h1 className="sr-only">{t('messagehub.title', 'MessageHub')}</h1>
      <div className="flex justify-start">
        <button
          type="button"
          disabled={!canEdit && !editing}
          aria-pressed={editing}
          aria-label={editLabel}
          title={editLabel}
          onClick={onToggleEditing}
          className={`${headerButtonClass} -ml-2 md:-ml-1.5`}
          style={editing ? { color: 'var(--cp-accent)' } : undefined}
          data-testid="entity-list-edit"
        >
          {editing ? <Check size={17} /> : <Pencil size={16} />}
        </button>
      </div>
      <div className="flex min-w-0 items-center justify-center gap-2" data-testid="hub-identity">
        <HubAvatar type={identity.type} name={identity.name} avatar={identity.avatar} size={28} />
        {editing ? (
          <span className="min-w-0 truncate text-[15px] font-semibold" style={{ color: 'var(--cp-text)' }} aria-live="polite">
            {selectedCount > 0 ? t('messagehub.header.selected', undefined, { count: selectedCount }) : t('messagehub.header.selectChats')}
          </span>
        ) : status ? (
          <span className="flex min-w-0 items-center gap-1.5 text-[15px] font-medium" style={{ color: 'var(--cp-muted)' }} data-testid="hub-sync-status" aria-hidden>
            <Loader2 size={14} className="shrink-0 animate-spin" />
            <span className="min-w-0 truncate">{status}</span>
          </span>
        ) : (
          <>
            <span className="min-w-0 truncate text-[15px] font-semibold" style={{ color: 'var(--cp-text)' }} title={identity.name} data-testid="hub-identity-name">{identity.name}</span>
            {unreadTotal > 0 ? (
              <span
                className="flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full px-1.5 text-[11px] font-semibold tabular-nums"
                style={{ background: 'var(--cp-accent)', color: 'var(--cp-surface-opaque)' }}
                role="img"
                aria-label={unreadLabel}
                title={unreadLabel}
                data-testid="hub-unread-total"
              >
                {unreadTotal}
              </span>
            ) : null}
          </>
        )}
        <span className="sr-only" role="status">{editing ? '' : status ?? ''}</span>
      </div>
      <div className="-mr-2 flex items-center justify-end md:-mr-1.5">
        <HeaderMenu label={t('messagehub.header.create')} icon={<Plus size={18} />} items={createItems} testId="hub-create-menu" />
        <HeaderMenu label={t('messagehub.header.more')} icon={<MoreVertical size={17} />} items={moreItems} testId="hub-more-menu" />
      </div>
    </div>
  )
}
