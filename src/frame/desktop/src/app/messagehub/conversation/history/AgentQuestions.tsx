import { useContext } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { agentAskQuestionText, type DID, type MessageObject, type MsgContent } from '../../protocol/msgobj'
import { useQaExpanded } from '../tasks'
import { ConversationMessageActionsContext } from './actions'
import { messageObjId, messageRelations, type MessageThreadEntry } from './relations'

function entryText(entry: MessageThreadEntry): string {
  const content: MsgContent = entry.content
  const text = (content.content ?? '').trim()
  if (entry.agent_ask) return agentAskQuestionText(text, entry.agent_ask)
  const names = (content.refs ?? []).map(ref => ref.label ?? (ref.target.type === 'data_obj' ? ref.target.obj_id : ref.target.did))
  return [text, ...names].filter(Boolean).join('\n')
}

/** The option the answer right after the question at `index` picked. */
function pickedOption(entries: readonly MessageThreadEntry[], index: number): string | undefined {
  const next = entries[index + 1]
  return next && !next.agent_ask ? next.option : undefined
}

/**
 * The questions an Agent asked during the Turn of this bubble and their
 * answers (`ui_relations.thread`). Open while the Turn has not replied yet,
 * closed to "N Q&A messages" once its final edit arrived; opening or closing
 * it by hand is remembered per viewer.
 */
export function AgentQuestions({ message, selfDid }: { message: MessageObject; selfDid?: DID }) {
  const { t } = useI18n()
  const { displayName } = useContext(ConversationMessageActionsContext)
  const relations = messageRelations(message)
  const entries = relations?.thread ?? []
  const [expanded, setExpanded] = useQaExpanded(selfDid ?? '', messageObjId(message) ?? '', !relations?.edited)
  if (entries.length === 0) return null
  const name = (entry: MessageThreadEntry) => entry.from === selfDid ? t('messagehub.you') : displayName?.(entry.from) ?? entry.senderName ?? entry.from
  return (
    <div className="mh-qa mt-2 rounded-xl px-2.5 py-1.5" data-testid="agent-qa" data-expanded={expanded} data-no-details>
      <button type="button" className="mh-qa-toggle -mx-1 flex min-h-8 items-center gap-1 rounded-lg px-1 text-[13px] font-medium" aria-expanded={expanded} onClick={() => setExpanded(!expanded)} data-testid="agent-qa-toggle">
        {expanded ? <ChevronDown size={16} aria-hidden /> : <ChevronRight size={16} aria-hidden />}
        {t('messagehub.qa.count', undefined, { count: entries.length })}
      </button>
      {expanded ? (
        <ol className="mt-1 space-y-2 border-t pb-1 pt-2" style={{ borderColor: 'color-mix(in srgb, currentColor 14%, transparent)' }} aria-label={t('messagehub.qa.list')} data-testid="agent-qa-list">
          {entries.map((entry, index) => {
            const options = entry.agent_ask?.input.options ?? []
            const picked = options.length > 0 ? pickedOption(entries, index) : undefined
            return (
              <li key={entry.id || index} className={entry.agent_ask ? '' : 'mh-qa-answer pl-2'} data-testid="agent-qa-item" data-kind={entry.agent_ask ? 'question' : 'answer'} data-qa-id={entry.id}>
                <p className="text-[12px] font-semibold opacity-80">{name(entry)}</p>
                {entry.redacted
                  ? <p className="text-[13px] italic opacity-70">{t('messagehub.message.redacted')}</p>
                  : <p className="whitespace-pre-wrap break-words text-[14px] leading-[1.45]">{entryText(entry)}</p>}
                {options.length > 0 && !entry.redacted ? (
                  <ul className="mt-1 flex flex-wrap gap-1" data-testid="agent-qa-options">
                    {options.map(option => <li key={option.id} className="mh-qa-option rounded-full px-2 text-[12px] leading-[22px]" data-picked={option.id === picked || undefined}>{option.label}</li>)}
                  </ul>
                ) : null}
              </li>
            )
          })}
        </ol>
      ) : null}
    </div>
  )
}
