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
        aria-label={resolve(item.label)}
        onClick={() => invoke(item)}
        className={clsx(
          'flex min-h-12 w-full items-center gap-4 px-5 py-2 text-left text-[15px] transition-colors active:bg-[color:var(--files-pressed)] disabled:opacity-40',
          item.danger ? 'text-[color:var(--cp-danger)]' : 'text-[color:var(--cp-text)]',
        )}
      >
        <span
          className={clsx(
            'flex w-5 shrink-0 justify-center',
            !item.danger && 'text-[color:var(--cp-muted)]',
          )}
        >
          {Icon ? <Icon size={20} /> : null}
        </span>
        <span className="min-w-0 flex-1 truncate">{resolve(item.label)}{item.disabledReason && <small className="block whitespace-normal text-[12px] text-[color:var(--cp-muted)]">{t(`filebrowser.commandReason.${item.disabledReason}`, item.disabledReason)}</small>}</span>
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
        className="flex min-h-12 w-full items-center gap-4 px-5 py-2 text-left text-[15px] text-[color:var(--cp-text)] transition-colors active:bg-[color:var(--files-pressed)]"
      >
        <span className="flex w-5 shrink-0 justify-center text-[color:var(--cp-muted)]">
          {Icon ? <Icon size={20} /> : null}
        </span>
        <span className="min-w-0 flex-1 truncate">{resolve(item.label)}</span>
        <ChevronRight size={20} className="shrink-0 text-[color:var(--cp-muted)]" />
      </button>
    )
  }

  return (
    <div className="absolute inset-0 z-50 flex items-end">
      <div className="files-scrim absolute inset-0" onClick={close} />
      <div className="files-sheet relative flex max-h-[70%] w-full flex-col pb-4 pt-2">
        <div className="files-sheet-grip mb-2" />
        {submenu ? (
          <div className="flex items-center gap-1 px-3 py-1">
            <button
              type="button"
              onClick={() => setSubmenu(null)}
              aria-label={t('common.back', 'Back')}
              className="flex h-8 w-8 items-center justify-center rounded-full text-[color:var(--cp-muted)]"
            >
              <ChevronLeft size={20} />
            </button>
            <span className="min-w-0 flex-1 truncate text-[16px] font-medium text-[color:var(--cp-text)]">
              {resolve(submenu.label)}
            </span>
          </div>
        ) : title ? (
          <div className="truncate px-5 pb-2 pt-1 text-[16px] font-medium text-[color:var(--cp-text)]">
            {title}
          </div>
        ) : null}
        <div className="min-h-0 overflow-y-auto pb-1">
          {submenu
            ? submenu.items.map(renderAction)
            : sections.map((section, index) => (
                <div key={index}>
                  {index > 0 ? (
                    <div className="mx-5 my-1 h-px bg-[color:var(--files-hairline)]" />
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
