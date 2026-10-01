import type { WindowDialogApi } from '../../desktop/windows/dialogs'
import { DialogFocus, hubButtonClass } from './SessionDialogs'

type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

/** Confirmation for destructive group actions; resolves true when confirmed. */
export async function confirmGroupAction(dialog: WindowDialogApi, t: Translate, options: { title: string; body: string; confirm: string }) {
  const trigger = document.activeElement
  const result = await dialog.open<boolean>({ title: options.title, size: 'sm', dismissible: false, renderBody: controls => <DialogFocus onCancel={() => controls.close(false)}>
    <p className="text-sm">{options.body}</p>
    <div className="mt-4 flex justify-end gap-2">
      <button data-autofocus type="button" className={hubButtonClass} onClick={() => controls.close(false)}>{t('messagehub.cancel')}</button>
      <button type="button" className={`${hubButtonClass} font-semibold text-[color:var(--cp-danger)]`} onClick={() => controls.close(true)}>{options.confirm}</button>
    </div>
  </DialogFocus> })
  if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus()
  return result === true
}

