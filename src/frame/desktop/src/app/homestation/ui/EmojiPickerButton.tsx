import { Smile } from 'lucide-react'
import { useState } from 'react'
import type { UseFormRegisterReturn } from 'react-hook-form'
import { EmojiPanel } from '../../../components/conversation/input/EmojiPanel'
import { useI18n } from '../../../i18n/provider'
import { Popover } from './Popover'

interface EmojiPickerButtonProps {
  field: Pick<UseFormRegisterReturn, 'name' | 'onChange'>
  disabled?: boolean
  testId?: string
}

export function EmojiPickerButton({ field, disabled, testId }: EmojiPickerButtonProps) {
  const { t } = useI18n()
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null)
  const [focusSearch, setFocusSearch] = useState(false)
  const control = anchor?.form?.elements.namedItem(field.name)
  const textarea = control instanceof HTMLTextAreaElement ? control : null
  const close = () => setAnchor(null)
  const label = t('messagehub.emoji.open')

  const insert = (emoji: string) => {
    if (!textarea) return
    textarea.setRangeText(emoji, textarea.selectionStart, textarea.selectionEnd, 'end')
    void field.onChange({ type: 'change', target: textarea })
    if (!window.matchMedia('(pointer: coarse)').matches) textarea.focus({ preventScroll: true })
  }

  return (
    <>
      <button
        type="button"
        className="hs-icon-btn"
        aria-label={label}
        title={label}
        aria-haspopup="dialog"
        aria-expanded={!!anchor}
        data-testid={testId}
        disabled={disabled}
        onMouseDown={event => event.preventDefault()}
        onClick={event => {
          if (anchor) return close()
          setFocusSearch(event.detail === 0)
          if (window.matchMedia('(pointer: coarse)').matches && document.activeElement instanceof HTMLTextAreaElement) document.activeElement.blur()
          setAnchor(event.currentTarget)
        }}
      >
        <Smile size={18} aria-hidden />
      </button>
      <Popover
        anchor={anchor}
        boundary={anchor?.closest('.hs-shell-desktop, .hs-shell-mobile, .shell-window') ?? anchor?.ownerDocument.documentElement}
        open={!!anchor}
        onClose={close}
        align="start"
        width={352}
        role="presentation"
        className="!border-0 [&>.mh-emoji-panel]:max-h-[inherit] [&>.mh-emoji-panel]:shadow-none"
        autoFocus={focusSearch}
        returnFocusTo={textarea}
      >
        <EmojiPanel onPick={insert} onClose={close} />
      </Popover>
    </>
  )
}
