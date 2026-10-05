import { useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import { recordMeta } from './api/reader'
import { MessageMarkdown } from './conversation/history/MessageMarkdown'
import { findMessage } from './conversation/history/locate'
import { messageObjId, messageRelations } from './conversation/history/relations'
import type { ConversationMessageReader } from './conversation/history/types'
import { CopyValue, TaskDetailsSection } from './conversation/tasks/MessageTask'
import { AgentWorklog } from './conversation/worklog/AgentWorklog'
import { messageHubMessagePath } from './launch'
import { getMessageDeliveryStatus, messageAgentTaskId, type DID, type MessageObject, type MsgContent } from './protocol/msgobj'
import { useMessageHubStore } from './store'
import type { MessageHubContext } from './types'

const sectionTitleClass = 'text-xs font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]'

function ContentView({ content, testId }: { content: MsgContent; testId: string }) {
  const refs = content.refs ?? []
  const text = content.content ?? ''
  return (
    <div className="rounded-xl bg-[color:color-mix(in_srgb,var(--cp-text)_5%,transparent)] px-3 py-2 text-[13px]" data-testid={testId} data-format={content.format ?? 'text/plain'}>
      {content.format === 'text/markdown' ? <MessageMarkdown text={text} /> : text ? <p className="whitespace-pre-wrap break-words">{text}</p> : null}
      {refs.length > 0 ? <ul className="mt-1 space-y-0.5 text-xs text-[color:var(--cp-muted)]">{refs.map((ref, index) => <li key={index} className="break-all">{ref.label ?? (ref.target.type === 'data_obj' ? ref.target.uri_hint ?? ref.target.obj_id : ref.target.did)} · {ref.role}</li>)}</ul> : null}
      <p className="mt-1 text-[11px] text-[color:var(--cp-muted)]">{content.format ?? 'text/plain'}</p>
    </div>
  )
}

/**
 * Details of one message, addressed by its anchor ObjId. Agent replies open
 * the Turn worklog; the message tab keeps content, edits, delivery and task
 * metadata. Messages outside the loaded history are located in older pages.
 */
export function MessageDetails({ context, entityId, sessionId, messageId, reader, displayName, onClose }: { context: MessageHubContext; entityId: string; sessionId: string; messageId: string; reader: ConversationMessageReader; displayName: (did: DID) => string; onClose: () => void }) {
  const { t } = useI18n()
  const store = useMessageHubStore()
  const [found, setFound] = useState<{ id: string; message: MessageObject | null } | null>(null)
  const located = useRef('')
  const [tab, setTab] = useState<'worklog' | 'message'>('worklog')
  useEffect(() => {
    let cancelled = false
    void findMessage(reader, messageId).then(async result => {
      if (cancelled) return
      if (result) { setFound({ id: messageId, message: result.message }); return }
      if (located.current === messageId) { setFound({ id: messageId, message: null }); return }
      located.current = messageId
      // Loading older pages replaces the reader, which runs this lookup again.
      const loaded = await store.locateMessage(context, sessionId, messageId).catch(() => false)
      if (!cancelled && !loaded) setFound({ id: messageId, message: null })
    })
    return () => { cancelled = true }
  }, [reader, messageId, store, sessionId, context.ownerDid, context.viewerDid, context.mode]) // eslint-disable-line react-hooks/exhaustive-deps
  const current = found?.id === messageId ? found : null
  const message = current?.message
  const relations = message ? messageRelations(message) : undefined
  const taskId = message && !relations?.redacted ? messageAgentTaskId(message) : undefined
  const record = message ? recordMeta(message) : undefined
  const deliveryStatus = message ? getMessageDeliveryStatus(message) : undefined
  const link = typeof window === 'undefined' ? '' : `${window.location.origin}${messageHubMessagePath({ entityId, sessionId, messageId }, context)}`
  return (
    <section className="flex h-full flex-col bg-[color:var(--cp-surface)]" data-testid="message-details-pane" data-message-id={messageId}>
      <header className="flex shrink-0 items-center justify-between border-b border-[color:var(--cp-border)] py-1 pl-4 pr-1"><h2 className="text-sm font-semibold">{t('messagehub.messageDetails.title')}</h2><button type="button" className="flex min-h-11 min-w-11 items-center justify-center" aria-label={t('messagehub.close')} title={t('messagehub.close')} onClick={onClose}><X size={18} /></button></header>
      {taskId ? <div className="flex shrink-0 gap-5 border-b border-[color:var(--cp-border)] px-4" role="tablist" aria-label={t('messagehub.messageDetails.title')}>
        {(['worklog', 'message'] as const).map(value => <button key={value} role="tab" aria-selected={tab === value} className={`border-b-2 py-3 text-xs font-medium ${tab === value ? 'border-[color:var(--cp-accent)] text-[color:var(--cp-accent)]' : 'border-transparent text-[color:var(--cp-muted)]'}`} onClick={() => setTab(value)} data-testid={`detail-tab-${value}`}>{t(`messagehub.worklog.tab.${value}`)}</button>)}
      </div> : null}
      {taskId && tab === 'worklog' ? <AgentWorklog key={taskId} taskId={taskId} /> : <div className="shell-scrollbar min-h-0 flex-1 space-y-6 overflow-y-auto p-4 text-sm">
        {!current ? <p role="status" className="text-[13px] text-[color:var(--cp-muted)]">{t('messagehub.messageDetails.locating')}</p> : null}
        {current && !message ? <p role="alert" className="text-[13px] text-[color:var(--cp-muted)]" data-testid="message-details-missing">{t('messagehub.messageDetails.notFound')}</p> : null}
        {message ? <>
          <section className="space-y-2">
            <h3 className="break-words text-lg font-semibold leading-snug">{displayName(message.from)}</h3>
            <p className="text-[13px] text-[color:var(--cp-muted)]"><time>{new Date(message.created_at_ms).toLocaleString()}</time></p>
            <CopyValue label={t('messagehub.messageDetails.messageId')} value={messageObjId(message) ?? messageId} testId="detail-message-id" />
            <CopyValue label={t('messagehub.messageDetails.link')} value={link} testId="detail-message-link" />
          </section>
          {relations?.redacted ? <p className="text-[13px] italic text-[color:var(--cp-muted)]" data-testid="detail-redacted">{t(relations.redacted.by === message.from ? 'messagehub.message.redacted' : 'messagehub.message.deletedBy', undefined, { name: displayName(relations.redacted.by) })}</p> : <>
            <section className="space-y-2">
              <h3 className={sectionTitleClass}>{t('messagehub.messageDetails.effective')}</h3>
              <ContentView content={relations?.edited?.content ?? message.content} testId="detail-effective" />
            </section>
            <section className="space-y-2">
              <h3 className={sectionTitleClass}>{t('messagehub.messageDetails.original')}</h3>
              {relations?.edited ? <ContentView content={message.content} testId="detail-original" /> : <p className="text-[13px] text-[color:var(--cp-muted)]" data-testid="detail-original-same">{t('messagehub.messageDetails.notEdited')}</p>}
            </section>
            {relations?.edited ? <section className="space-y-2" data-testid="detail-edits">
              <h3 className={sectionTitleClass}>{t('messagehub.messageDetails.edits', undefined, { count: relations.edited.edits.length })}</h3>
              <ol className="space-y-2">{relations.edited.edits.map((edit, index) => <li key={edit.id ?? index} className="text-[13px]">
                <p className="text-xs text-[color:var(--cp-muted)]"><time>{new Date(edit.at).toLocaleString()}</time>{edit.at === relations.edited?.at ? ` · ${t('messagehub.messageDetails.inEffect')}` : ''}</p>
                <p className="line-clamp-3 whitespace-pre-wrap break-words">{edit.content.content || (edit.content.refs ?? []).map(ref => ref.label).filter(Boolean).join(', ')}</p>
                {edit.id ? <p className="truncate text-[11px] text-[color:var(--cp-muted)]" title={edit.id}>{edit.id}</p> : null}
              </li>)}</ol>
            </section> : null}
          </>}
          <section className="space-y-2" data-testid="detail-delivery">
            <h3 className={sectionTitleClass}>{t('messagehub.messageDetails.delivery')}</h3>
            <ul className="space-y-1 text-[13px]">
              {record ? <li>{record.boxKind} · {t(record.direction === 'in' ? 'messagehub.messageDetails.received' : 'messagehub.messageDetails.sent')}{record.recipientState ? ` · ${record.recipientState}` : ''}</li> : null}
              {record?.delivery?.overall ? <li>{t('messagehub.messageDetails.overall')}: {record.delivery.overall}</li> : null}
              {(record?.delivery?.per_target ?? []).map(target => <li key={target.target_did} className="break-all text-xs text-[color:var(--cp-muted)]">{target.target_did} · {target.state}{target.last_error ? ` · ${target.last_error.message}` : ''}</li>)}
              {!record?.delivery?.overall && deliveryStatus ? <li>{t(`messagehub.deliveryStatus.${deliveryStatus}`)}</li> : null}
              {!record && !deliveryStatus ? <li className="text-[color:var(--cp-muted)]">{t('messagehub.messageDetails.noDelivery')}</li> : null}
            </ul>
          </section>
          {relations?.redacted ? null : <TaskDetailsSection message={message} />}
        </> : null}
      </div>}
    </section>
  )
}
