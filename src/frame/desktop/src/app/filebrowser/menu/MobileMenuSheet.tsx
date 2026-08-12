/**
 * Mobile renderer for the file context-menu model: a bottom action sheet.
 * Renders the same `FileMenuSection[]` data the desktop popup uses; submenus
 * drill in with a back header instead of cascading popups. Tapping the
 * backdrop dismisses the sheet.
 */

import clsx from 'clsx'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { MENU_ICONS } from './icons'
import type {
  FileMenuAction,
  FileMenuLabel,
  FileMenuSection,
  FileMenuSubmenu,
} from './types'

export interface MobileMenuSheetProps {
  open: boolean
  /** Optional sheet header (item name / selection summary). */
  title?: string
  sections: FileMenuSection[]
  onInvoke: (action: FileMenuAction) => void
  onClose: () => void
}

export function MobileMenuSheet({
  open,
  title,
  sections,
  onInvoke,
  onClose,
}: MobileMenuSheetProps) {
  const { t } = useI18n()
  const [submenu, setSubmenu] = useState<FileMenuSubmenu | null>(null)

  if (!open) return null

  const resolve = (label: FileMenuLabel) => t(label.key, label.fallback, label.vars)

  // All dismissals run through here so a re-opened sheet starts at the root level.
  const close = () => {
    setSubmenu(null)
    onClose()
  }

  const invoke = (action: FileMenuAction) => {
    close()
    onInvoke(action)
  }

  const renderAction = (item: FileMenuAction) => {
    const Icon = item.icon ? MENU_ICONS[item.icon] : null
    return (
      <button
        key={item.id}
        type="button"
        disabled={item.disabled}
        onClick={() => invoke(item)}
        className={clsx(
          'flex w-full items-center gap-3 px-5 py-3 text-left text-[14px] transition active:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)] disabled:opacity-40',
          item.danger ? 'text-[color:var(--cp-danger)]' : 'text-[color:var(--cp-text)]',
        )}
      >
        <span
          className={clsx(
            'flex w-5 shrink-0 justify-center',
            !item.danger && 'text-[color:var(--cp-muted)]',
          )}
        >
          {Icon ? <Icon size={17} /> : null}
        </span>
        <span className="min-w-0 flex-1 truncate">{resolve(item.label)}</span>
      </button>
    )
  }

  const renderSubmenu = (item: FileMenuSubmenu) => {
    const Icon = item.icon ? MENU_ICONS[item.icon] : null
    return (
      <button
        key={item.id}
        type="button"
        onClick={() => setSubmenu(item)}
        className="flex w-full items-center gap-3 px-5 py-3 text-left text-[14px] text-[color:var(--cp-text)] transition active:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)]"
      >
        <span className="flex w-5 shrink-0 justify-center text-[color:var(--cp-muted)]">
          {Icon ? <Icon size={17} /> : null}
        </span>
        <span className="min-w-0 flex-1 truncate">{resolve(item.label)}</span>
        <ChevronRight size={16} className="shrink-0 opacity-60" />
      </button>
    )
  }

  return (
    <div className="absolute inset-0 z-50 flex items-end">
      <div className="absolute inset-0 bg-black/50" onClick={close} />
      <div
        className="relative flex max-h-[70%] w-full flex-col rounded-t-[28px] border-t border-[color:var(--cp-border)] pb-4 pt-2"
        style={{ background: 'var(--cp-surface)' }}
      >
        <div className="mx-auto mb-1 h-1 w-10 rounded-full bg-[color:var(--cp-border)]" />
        {submenu ? (
          <div className="flex items-center gap-1 px-3 py-1">
            <button
              type="button"
              onClick={() => setSubmenu(null)}
              aria-label={t('common.back', 'Back')}
              className="flex h-8 w-8 items-center justify-center rounded-full text-[color:var(--cp-muted)]"
            >
              <ChevronLeft size={18} />
            </button>
            <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-[color:var(--cp-text)]">
              {resolve(submenu.label)}
            </span>
          </div>
        ) : title ? (
          <div className="truncate px-5 py-1 text-[13px] font-semibold text-[color:var(--cp-text)]">
            {title}
          </div>
        ) : null}
        <div className="min-h-0 overflow-y-auto pb-1">
          {submenu
            ? submenu.items.map(renderAction)
            : sections.map((section, index) => (
                <div key={index}>
                  {index > 0 ? (
                    <div className="mx-5 my-1 h-px bg-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)]" />
                  ) : null}
                  {section.map((item) =>
                    item.type === 'action' ? renderAction(item) : renderSubmenu(item),
                  )}
                </div>
              ))}
        </div>
      </div>
    </div>
  )
}
