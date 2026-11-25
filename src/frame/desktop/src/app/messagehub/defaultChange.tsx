import { DialogFocus, hubButtonClass, hubPrimaryButtonClass } from '../../components/conversation/DialogFocus'
import type { WindowDialogApi } from '../../desktop/windows/dialogs'
import { isChannelSession } from './sessionModel'
import type { MessageHubStore } from './store'
import type { MessageHubContext, Session } from './types'

type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

/**
 * Before an Agent's default moves away from a channel (Telegram …): the owner is told that the channel stops receiving
 * the Agent's notifications. Resolves to whether the default is to change; without a channel to leave it does.
 */
export async function confirmDefaultChange(dialog: WindowDialogApi, t: Translate, store: MessageHubStore, context: MessageHubContext, entityId: string, next?: Session): Promise<boolean> {
  await store.loadConversations(context, entityId)
  const left = store.defaultSession(context, entityId)
  if (!left || left.id === next?.id || !isChannelSession(context, left)) return true
  const names = { channel: left.binding.kind === 'tunnel' ? left.binding.connectionName : left.title, name: store.findEntity(context, entityId)?.name ?? entityId }
  const trigger = document.activeElement
  const result = await dialog.open<boolean>({ title: t('messagehub.defaultSession.leaveTitle'), size: 'sm', dismissible: false, renderBody: controls => <DialogFocus onCancel={() => controls.close(false)}><div data-testid="leave-channel">
    <p className="text-sm">{t('messagehub.defaultSession.leave', undefined, names)}</p>
    {!next && <p className="mt-2 text-sm text-[color:var(--cp-muted)]">{t('messagehub.defaultSession.leaveCreate', undefined, names)}</p>}
    <div className="mt-4 flex justify-end gap-2">
      <button data-autofocus type="button" className={hubButtonClass} onClick={() => controls.close(false)}>{t('messagehub.defaultSession.keep', undefined, names)}</button>
      <button type="button" className={hubPrimaryButtonClass} onClick={() => controls.close(true)}>{t('messagehub.defaultSession.change')}</button>
    </div>
  </div></DialogFocus> })
  if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus()
  return result === true
}
