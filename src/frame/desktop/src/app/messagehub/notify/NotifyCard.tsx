import { useContext, useState, type CSSProperties, type ReactNode } from 'react'
import { Bell, Circle, CircleDot, ExternalLink, MessageCircleQuestion, RefreshCw } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { friendlyDidName } from '../api/projection'
import { ConversationMessageActionsContext } from '../conversation/history/actions'
import { messageObjId } from '../conversation/history/relations'
import type { MessageObject, NotifyMessageData, NotifyStatus } from '../protocol/msgobj'
import { committedApp, isNotifyClosed, promptOptions, promptTextInput, resultOption, resultText, type NotifyAnswer, type NotifyView } from './model'
import { getNotifyWatch, useNotifyEntry } from './index'

/** Links an App may put on a notification: web pages and paths of this zone. */
function safeLink(link: string | undefined): string | undefined {
  if (!link) return undefined
  if (link.startsWith('/') && !link.startsWith('//')) return link
  try { return ['http:', 'https:'].includes(new URL(link).protocol) ? link : undefined } catch { return undefined }
}

function time(ms: number | undefined): string {
  return ms ? new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : ''
}

const statusTone: Record<NotifyStatus, string> = {
  open: 'var(--cp-accent)',
  read: 'var(--cp-muted)',
  answered: 'var(--cp-success)',
  dismissed: 'var(--cp-muted)',
  expired: 'var(--cp-warning)',
  canceled: 'var(--cp-muted)',
}

const knownReasons = new Set(['ask_timeout', 'turn_stopped', 'goal_dropped', 'expired'])

/**
 * A System Notify message (`buckyos.notify`): the record in a category
 * session or a copy in a conversation. Its state is the notification's, read
 * with `notify.get` and shared by every card of it; the message only gives
 * what to show before that read arrives. Handling it never touches the
 * composer of the conversation it is in.
 */
export function NotifyCard({ message, data, className, style, footer }: { message: MessageObject; data: NotifyMessageData; className: string; style: CSSProperties; footer: ReactNode }) {
  const { t } = useI18n()
  const { displayName, handleNotify } = useContext(ConversationMessageActionsContext)
  const entry = useNotifyEntry(data.notify_id)
  const watch = getNotifyWatch()
  const me = watch.me()
  const [pending, setPending] = useState<string | null>(null)
  const [error, setError] = useState('')
  const [lost, setLost] = useState(false)
  const [text, setText] = useState('')
  const view: NotifyView | undefined = entry?.view
  const kind = view?.kind ?? data.kind
  const prompt = view?.prompt ?? data.prompt
  const source = view?.source ?? data.source
  const level = view?.level ?? data.level
  const status = view?.status
  const closed = status ? isNotifyClosed(status) : false
  const options = promptOptions(prompt)
  const textInput = kind === 'input' ? promptTextInput(prompt) : undefined
  const nameOf = (did: string) => displayName?.(did) ?? friendlyDidName(did, true)
  const sourceName = source.agent_did ? nameOf(source.agent_did) : source.app_id ?? t('messagehub.notify.source.system')
  const context = source.goal_id ? t('messagehub.notify.source.goal', undefined, { name: source.operation ?? source.goal_id })
    : source.session_id ? t('messagehub.notify.source.work', undefined, { name: source.operation ?? source.session_id })
    : source.operation ?? ''
  const link = safeLink(source.link)
  const canAct = handleNotify === true && entry?.status === 'ready' && !!status && !closed

  const run = (key: string, operation: () => Promise<'committed' | 'already_closed' | void>) => {
    if (pending) return
    setPending(key); setError('')
    void operation().then(outcome => {
      if (outcome === 'already_closed') setLost(true)
      else if (key === 'text') setText('')
    }, failure => setError(failure instanceof Error && ['unknown_option', 'text_not_accepted', 'empty'].includes(failure.message) ? t(`messagehub.notify.invalid.${failure.message}`) : t('messagehub.notify.failed'))).finally(() => setPending(null))
  }
  const answer = (key: string, value: NotifyAnswer) => run(key, () => watch.answer(data.notify_id, value))
  const who = (name: string | undefined) => !name ? '' : name === me ? t('messagehub.you') : name
  const optionLabel = (value: string | undefined) => value ? options.find(option => option.value === value)?.label ?? value : undefined

  const result = (() => {
    if (!view || !status) return null
    const at = time(view.result?.submitted_at ?? view.closed_at_ms)
    const by = who(view.result?.submitted_by)
    const proxy = view.result && source.agent_did && committedApp(view.result) && committedApp(view.result) === source.app_id ? nameOf(source.agent_did) : undefined
    const meta = [by ? t(proxy ? 'messagehub.notify.result.byProxy' : 'messagehub.notify.result.by', undefined, { name: by, agent: proxy ?? '' }) : '', at].filter(Boolean).join(' · ')
    if (status === 'answered') {
      const picked = optionLabel(resultOption(view.result))
      const typed = resultText(view.result)
      return <>
        <p className="text-[14px] font-medium" data-testid="notify-answer">{t('messagehub.notify.result.answer', undefined, { answer: [picked, typed].filter(Boolean).join(' · ') })}</p>
        {meta ? <p className="text-[12px]" style={{ color: 'var(--cp-muted)' }} data-testid="notify-result-meta">{meta}</p> : null}
      </>
    }
    if (status === 'read') return view.acked_by ? <p className="text-[12px]" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.notify.result.ackedBy', undefined, { name: nameOf(view.acked_by) })}</p> : null
    if (status === 'open') return null
    const reason = view.closed_reason && status === 'canceled' ? (knownReasons.has(view.closed_reason) ? t(`messagehub.notify.reason.${view.closed_reason}`) : view.closed_reason) : ''
    return <p className="text-[13px]" style={{ color: 'var(--cp-muted)' }}>{[t(`messagehub.notify.result.${status}`), reason, meta].filter(Boolean).join(' · ')}</p>
  })()

  return (
    <div
      className={`mh-bubble mh-notify ${className}`}
      style={style}
      data-testid="notify-card"
      data-notify-id={data.notify_id}
      data-kind={kind}
      data-home={data.home}
      data-status={status ?? entry?.status ?? 'loading'}
      data-message-id={messageObjId(message)}
    >
      <div className="flex items-center gap-2">
        <span className="mh-notify-icon flex h-8 w-8 shrink-0 items-center justify-center rounded-full" aria-hidden>{kind === 'input' ? <MessageCircleQuestion size={16} /> : <Bell size={16} />}</span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[12px] font-medium" style={{ color: 'var(--cp-muted)' }} data-testid="notify-source" title={source.agent_did ?? source.app_id}>{sourceName}{context ? ` · ${context}` : ''}</p>
          <div className="mt-0.5 flex flex-wrap items-center gap-1">
            <span className="mh-notify-chip" data-level={level} data-testid="notify-level" title={t('messagehub.notify.levelHint')}>{t(`messagehub.notify.level.${level}`)}</span>
            <span className="mh-notify-chip" style={{ '--mh-chip': status ? statusTone[status] : 'var(--cp-muted)' } as CSSProperties} data-testid="notify-status">{status ? t(`messagehub.notify.status.${kind === 'notice' && status === 'open' ? 'unread' : status}`) : t(entry?.status === 'loading' || !entry ? 'messagehub.notify.status.loading' : `messagehub.notify.state.${entry.status}`)}</span>
          </div>
        </div>
      </div>
      {prompt.title ? <p className="mt-2 break-words text-[15px] font-semibold leading-snug" data-testid="notify-title">{prompt.title}</p> : null}
      {prompt.message ? <p className="mt-1 whitespace-pre-wrap break-words text-[14px] leading-[1.45]" data-testid="notify-message">{prompt.message}</p> : null}
      {link ? <a className="mt-1.5 inline-flex items-center gap-1 text-[13px] underline underline-offset-2" style={{ color: 'var(--cp-accent)' }} href={link} target={link.startsWith('/') ? undefined : '_blank'} rel="noreferrer" data-testid="notify-link"><ExternalLink size={13} aria-hidden />{t('messagehub.notify.openLink')}</a> : null}

      {canAct && kind === 'input' ? (
        <div className="mt-3 flex flex-col gap-1.5" data-testid="notify-controls" data-no-details>
          {options.length > 0 ? <div role="group" aria-label={t('messagehub.notify.options')} className="flex flex-col gap-1.5">
            {options.map(option => (
              <button key={option.value} type="button" className="mh-answer-option" aria-pressed={pending === `option:${option.value}`} disabled={!!pending} onClick={() => answer(`option:${option.value}`, { option: option.value })} data-testid="notify-option" data-option={option.value}>
                {pending === `option:${option.value}` ? <CircleDot size={16} aria-hidden style={{ color: 'var(--cp-accent)' }} /> : <Circle size={16} aria-hidden style={{ color: 'var(--cp-muted)' }} />}
                <span className="min-w-0 flex-1 break-words">{option.label}</span>
              </button>
            ))}
          </div> : null}
          {textInput ? <form className="flex flex-col gap-1.5" onSubmit={event => { event.preventDefault(); if (text.trim()) answer('text', { text }) }}>
            <label className="sr-only" htmlFor={`notify-text-${data.notify_id}-${messageObjId(message)}`}>{textInput.label ?? t('messagehub.notify.textLabel')}</label>
            <textarea id={`notify-text-${data.notify_id}-${messageObjId(message)}`} className="mh-notify-input" rows={textInput.multiline === false ? 1 : 2} value={text} placeholder={textInput.placeholder ?? t(options.length ? 'messagehub.notify.textOther' : 'messagehub.notify.textPlaceholder')} disabled={!!pending} onChange={event => setText(event.target.value)} onKeyDown={event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && text.trim()) { event.preventDefault(); answer('text', { text }) } }} data-testid="notify-text" />
            <div className="flex justify-end"><button type="submit" className="mh-notify-primary" disabled={!!pending || !text.trim()} data-testid="notify-submit">{t(pending === 'text' ? 'messagehub.notify.submitting' : 'messagehub.notify.submit')}</button></div>
          </form> : null}
          <div className="flex justify-end"><button type="button" className="mh-answer-action" disabled={!!pending} onClick={() => answer('dismiss', { dismiss: true })} data-testid="notify-dismiss">{t('messagehub.notify.dismiss')}</button></div>
        </div>
      ) : null}
      {canAct && kind === 'notice' ? (
        <div className="mt-3 flex flex-wrap items-center justify-end gap-2" data-testid="notify-controls" data-no-details>
          <button type="button" className="mh-answer-action" disabled={!!pending} onClick={() => run('dismiss', () => watch.dismiss(data.notify_id))} data-testid="notify-dismiss">{t('messagehub.notify.dismiss')}</button>
          {status === 'open' ? <button type="button" className="mh-notify-primary" disabled={!!pending} onClick={() => run('ack', () => watch.ack(data.notify_id))} data-testid="notify-ack">{t('messagehub.notify.ack')}</button> : null}
        </div>
      ) : null}

      {result ? <div className="mh-notify-result mt-3 rounded-xl px-3 py-2" data-testid="notify-result">{result}</div> : null}
      {lost ? <p role="status" className="mt-2 text-[12px]" style={{ color: 'var(--cp-muted)' }} data-testid="notify-conflict">{t('messagehub.notify.handledElsewhere')}</p> : null}
      {entry?.stale ? <p className="mt-2 text-[12px]" style={{ color: 'var(--cp-muted)' }}>{t('messagehub.notify.stale')}</p> : null}
      {entry && entry.status !== 'ready' && entry.status !== 'loading' && !entry.view ? (
        <div className="mt-2 flex items-center gap-2 text-[12px]" style={{ color: 'var(--cp-muted)' }}>
          {entry.status === 'error' ? <button type="button" className="mh-answer-action" onClick={() => void watch.refresh(data.notify_id)} data-testid="notify-retry"><RefreshCw size={12} aria-hidden />{t('messagehub.retry')}</button> : null}
        </div>
      ) : null}
      {error ? <p role="alert" className="mt-2 text-[12px]" style={{ color: 'var(--cp-danger)' }} data-testid="notify-error">{error}</p> : null}
      {footer}
    </div>
  )
}
