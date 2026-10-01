import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import {
  AtSign,
  File,
  FolderOpen,
  ImageIcon,
  Paperclip,
  Send,
  X,
} from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { displayedContent, messageObjId } from '../history/relations'
import { getMessageSenderName, type MessageObject, type MsgMentions, type MsgRelation } from '../../protocol/msgobj'
import { collectMentions, MENTION_ALL, type MentionCandidate } from './mentions'
import {
  createAttachmentItem,
  extractTransferFiles,
  filesFromInputList,
  formatAttachmentSize,
  getAttachmentPathKey,
  MAX_ATTACHMENT_BYTES,
  revokeAttachmentItem,
  type ComposerAttachmentInput,
  type ComposerAttachmentItem,
} from './attachmentDraft'

/** Keystrokes are coalesced before the draft text is persisted. */
const DRAFT_PERSIST_DELAY_MS = 300
const COMPOSER_FIELD_MIN_HEIGHT = 44
const COMPOSER_FIELD_CHROME = 18

function sameAttachmentInputs(a: readonly ComposerAttachmentInput[], b: readonly ComposerAttachmentInput[]) {
  return a.length === b.length && a.every((item, index) => item.file === b[index].file && item.relativePath === b[index].relativePath)
}

export interface ConversationComposerSubmitPayload {
  attachments: ComposerAttachmentItem[]
  content: string
  /** MsgObject v2 `relates_to` of a reply (`thread`) or an edit of an own message. */
  relatesTo?: MsgRelation
  /** Structured mentions picked in the composer; absent when none. */
  mentions?: MsgMentions
}

/** A message the next send relates to: a reply quotes it, an edit replaces its text. */
export interface ComposerRelation {
  kind: 'reply' | 'edit'
  message: MessageObject
}

export type { MentionCandidate } from './mentions'

export interface ConversationComposerHandle {
  addTransferData: (dataTransfer: DataTransfer) => Promise<void>
  focus: () => void
}

interface ConversationComposerProps {
  placeholder: string
  /** Max height for the entire composer (in px). Used to compute inner constraints. */
  maxHeight?: number
  onSendMessage: (payload: ConversationComposerSubmitPayload) => void | Promise<void>
  initialDraft?: string
  initialAttachments?: ComposerAttachmentInput[]
  onAttachmentsChange?: (attachments: ComposerAttachmentInput[]) => Promise<void> | undefined
  onDraftChange?: (value: string) => Promise<void> | undefined
  relation?: ComposerRelation | null
  onCancelRelation?: () => void
  /** Members offered by the mention picker (group sessions only). */
  mentionCandidates?: MentionCandidate[]
  canMentionAll?: boolean
}


const ConversationComposerInner = forwardRef<
  ConversationComposerHandle,
  ConversationComposerProps
>(function ConversationComposer(
  { placeholder, maxHeight, onSendMessage, initialDraft = '', initialAttachments = [], onAttachmentsChange, onDraftChange, relation = null, onCancelRelation, mentionCandidates = [], canMentionAll = false },
  ref,
) {
  const { t } = useI18n()
  const [attachments, setAttachments] = useState<ComposerAttachmentItem[]>(() => initialAttachments.map(createAttachmentItem))
  const [inputValue, setInputValue] = useState(initialDraft)
  const [mentionOpen, setMentionOpen] = useState(false)
  const [picked, setPicked] = useState<MentionCandidate[]>([])
  const [mentionAll, setMentionAll] = useState(false)
  const [pendingSends, setPendingSends] = useState(0)
  const [oversizeNames, setOversizeNames] = useState<string[]>([])
  const [sendError, setSendError] = useState<string | false>(false)
  // Each submit snapshots the draft into this queue and clears the input at
  // once, so typing continues while earlier messages are still being sent.
  // Sends run one at a time to keep their order; a failure stops the queue
  // and puts the failed and still-queued drafts back into the composer.
  const sendQueue = useRef<ConversationComposerSubmitPayload[]>([])
  const draining = useRef(false)
  const mounted = useRef(false)
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  const sendCallback = useRef(onSendMessage)
  useEffect(() => { sendCallback.current = onSendMessage }, [onSendMessage])
  const attachmentsCallback = useRef(onAttachmentsChange)
  useEffect(() => { attachmentsCallback.current = onAttachmentsChange }, [onAttachmentsChange])
  const draftCallback = useRef(onDraftChange)
  useEffect(() => { draftCallback.current = onDraftChange }, [onDraftChange])
  // Every persisted write clones the whole local record, so the draft text is
  // debounced (flushed on unmount and whenever attachments are written) and a
  // value that is already stored is never written again.
  const persistedDraft = useRef(initialDraft)
  const persistedAttachments = useRef<ComposerAttachmentInput[]>(initialAttachments)
  const latestDraft = useRef(initialDraft)
  const draftTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const flushDraft = useCallback(() => {
    if (draftTimer.current) { clearTimeout(draftTimer.current); draftTimer.current = null }
    const value = latestDraft.current
    if (value === persistedDraft.current || !draftCallback.current) return
    persistedDraft.current = value
    void draftCallback.current(value)?.catch(() => setSendError('true'))
  }, [])
  useEffect(() => {
    latestDraft.current = inputValue
    if (inputValue === persistedDraft.current) return
    if (draftTimer.current) clearTimeout(draftTimer.current)
    draftTimer.current = setTimeout(flushDraft, DRAFT_PERSIST_DELAY_MS)
  }, [inputValue, flushDraft])
  useEffect(() => flushDraft, [flushDraft])
  useEffect(() => {
    const inputs = attachments.map(({ file, relativePath }) => ({ file, relativePath }))
    if (sameAttachmentInputs(inputs, persistedAttachments.current)) return
    persistedAttachments.current = inputs
    flushDraft()
    if (attachmentsCallback.current) void attachmentsCallback.current(inputs)?.catch(() => setSendError('true'))
  }, [attachments, flushDraft])
  const [pickerOpen, setPickerOpen] = useState(false)
  const attachmentsRef = useRef<ComposerAttachmentItem[]>([])
  const composerRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const directoryInputRef = useRef<HTMLInputElement>(null)
  // An edit starts from the message's current text; the draft the user had is
  // restored when the edit is cancelled or sent (state adjusted on prop change).
  const [appliedRelation, setAppliedRelation] = useState<ComposerRelation | null>(null)
  const [draftBeforeEdit, setDraftBeforeEdit] = useState<string | null>(null)
  if (relation !== appliedRelation) {
    setAppliedRelation(relation)
    if (relation?.kind === 'edit') {
      if (draftBeforeEdit === null) setDraftBeforeEdit(inputValue)
      setInputValue(displayedContent(relation.message))
    } else if (draftBeforeEdit !== null) {
      setInputValue(draftBeforeEdit)
      setDraftBeforeEdit(null)
    }
  }
  useEffect(() => { if (relation) inputRef.current?.focus() }, [relation])

  // Message input area max is 50% of the composer's max height
  const messageInputMaxHeight = maxHeight != null ? Math.floor(maxHeight / 2) : undefined

  // Auto-resize textarea to fit content; scrollbar appears when it exceeds max
  useLayoutEffect(() => {
    const element = inputRef.current
    if (!element) {
      return
    }

    element.style.height = '0px'
    const scrollHeight = element.scrollHeight
    const limit = messageInputMaxHeight != null ? Math.max(COMPOSER_FIELD_MIN_HEIGHT, messageInputMaxHeight - COMPOSER_FIELD_CHROME) : scrollHeight
    element.style.height = `${Math.min(scrollHeight, limit)}px`
    element.style.overflowY = scrollHeight > limit ? 'auto' : 'hidden'
  }, [inputValue, messageInputMaxHeight])

  useEffect(() => {
    const element = directoryInputRef.current
    if (!element) {
      return
    }

    element.setAttribute('webkitdirectory', '')
    element.setAttribute('directory', '')
  }, [])

  useEffect(() => {
    attachmentsRef.current = attachments
  }, [attachments])

  const hasAttachments = attachments.length > 0

  useEffect(() => {
    return () => {
      attachmentsRef.current.forEach(revokeAttachmentItem)
    }
  }, [])

  useEffect(() => {
    if (!pickerOpen && !mentionOpen) {
      return
    }

    const handlePointerDown = (event: MouseEvent) => {
      if (!composerRef.current?.contains(event.target as Node)) {
        setPickerOpen(false)
        setMentionOpen(false)
      }
    }

    document.addEventListener('pointerdown', handlePointerDown)

    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
    }
  }, [pickerOpen, mentionOpen])

  const appendAttachmentInputs = useCallback((selected: ComposerAttachmentInput[]) => {
    const items = selected.filter(item => item.file.size <= MAX_ATTACHMENT_BYTES)
    setOversizeNames(selected.filter(item => item.file.size > MAX_ATTACHMENT_BYTES).map(item => item.relativePath || item.file.name))
    if (items.length === 0) {
      return
    }

    setAttachments((previous) => {
      const existingKeys = new Set(previous.map(getAttachmentPathKey))
      const nextInputs: ComposerAttachmentInput[] = []

      items.forEach((item) => {
        const pathKey = getAttachmentPathKey(item)

        if (existingKeys.has(pathKey)) {
          return
        }

        existingKeys.add(pathKey)
        nextInputs.push(item)
      })

      if (nextInputs.length === 0) {
        return previous
      }

      return [
        ...previous,
        ...nextInputs.map(createAttachmentItem),
      ]
    })
    setPickerOpen(false)
    inputRef.current?.focus()
  }, [])

  const clearAttachments = useCallback(() => {
    setAttachments((previous) => {
      previous.forEach(revokeAttachmentItem)
      return []
    })
  }, [])

  const handleRemoveAttachment = useCallback((id: string) => {
    setAttachments((previous) => {
      const target = previous.find((item) => item.id === id)

      if (target) {
        revokeAttachmentItem(target)
      }

      return previous.filter((item) => item.id !== id)
    })
  }, [])

  const restoreFailedSends = useCallback((failed: ConversationComposerSubmitPayload[]) => {
    const text = failed.map(item => item.content).filter(Boolean).join('\n')
    const items = failed.flatMap(item => item.attachments)
    if (!mounted.current) {
      const draft = [text, latestDraft.current].filter(Boolean).join('\n')
      const inputs = [...items, ...attachmentsRef.current].map(({ file, relativePath }) => ({ file, relativePath }))
      items.forEach(revokeAttachmentItem)
      latestDraft.current = draft
      persistedDraft.current = draft
      persistedAttachments.current = inputs
      void draftCallback.current?.(draft)?.catch(() => undefined)
      void attachmentsCallback.current?.(inputs)?.catch(() => undefined)
      return
    }
    setInputValue(current => [text, current].filter(Boolean).join('\n'))
    setAttachments(current => {
      const keys = new Set(current.map(getAttachmentPathKey))
      return [...items.filter(item => !keys.has(getAttachmentPathKey(item))), ...current]
    })
  }, [])

  const drainSendQueue = useCallback(async () => {
    if (draining.current) return
    draining.current = true
    try {
      while (sendQueue.current.length > 0) {
        const next = sendQueue.current[0]
        try {
          await sendCallback.current(next)
        } catch (error) {
          const failed = sendQueue.current.splice(0)
          restoreFailedSends(failed)
          if (mounted.current) setSendError(error instanceof Error && error.message ? error.message : 'true')
          break
        }
        sendQueue.current.shift()
        next.attachments.forEach(revokeAttachmentItem)
        if (mounted.current) setPendingSends(sendQueue.current.length)
      }
    } finally {
      draining.current = false
      if (mounted.current) setPendingSends(sendQueue.current.length)
    }
  }, [restoreFailedSends])

  const handleSend = useCallback(() => {
    const text = inputValue.trim()

    if (!text && attachments.length === 0) {
      return
    }

    const target = relation ? messageObjId(relation.message) : undefined
    const relatesTo: MsgRelation | undefined = relation && target ? { rel: relation.kind === 'edit' ? 'edit' : 'thread', target } : undefined
    const mentions = collectMentions(text, picked, mentionAll)
    sendQueue.current.push({ attachments: relation?.kind === 'edit' ? [] : attachments, content: text, ...(relatesTo ? { relatesTo } : {}), ...(mentions ? { mentions } : {}) })
    if (relation) onCancelRelation?.()
    setPicked([]); setMentionAll(false)
    setPendingSends(sendQueue.current.length)
    setSendError(false)
    setOversizeNames([])
    setInputValue('')
    latestDraft.current = ''
    flushDraft()
    setAttachments([])
    inputRef.current?.focus()
    void drainSendQueue()
  }, [attachments, drainSendQueue, flushDraft, inputValue, relation, onCancelRelation, picked, mentionAll])

  const insertMention = useCallback((candidate: MentionCandidate | 'all') => {
    const text = candidate === 'all' ? `${MENTION_ALL} ` : `@${candidate.name} `
    if (candidate === 'all') setMentionAll(true)
    else setPicked(previous => previous.some(item => item.did === candidate.did) ? previous : [...previous, candidate])
    insertTextAtSelection(text, inputValue, setInputValue, inputRef.current)
    setMentionOpen(false)
  }, [inputValue])

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      if (!event.nativeEvent.isComposing) handleSend()
    }

    if (event.key === 'Escape') {
      setPickerOpen(false)
      setMentionOpen(false)
      if (relation) onCancelRelation?.()
    }
  }

  const addTransferData = useCallback(async (dataTransfer: DataTransfer) => {
    const transferFiles = await extractTransferFiles(dataTransfer)
    appendAttachmentInputs(transferFiles)
  }, [appendAttachmentInputs])

  useImperativeHandle(ref, () => ({
    addTransferData,
    focus: () => inputRef.current?.focus(),
  }), [addTransferData])

  const handlePaste = async (
    event: React.ClipboardEvent<HTMLTextAreaElement>,
  ) => {
    const clipboardData = event.clipboardData
    const hasFileItems = Array.from(clipboardData.items).some(
      (item) => item.kind === 'file',
    )

    if (!hasFileItems) {
      return
    }

    event.preventDefault()

    const plainText = clipboardData.getData('text/plain')
    if (plainText) {
      insertTextAtSelection(plainText, inputValue, setInputValue, inputRef.current)
    }

    await addTransferData(clipboardData)
  }

  const handleFileInputChange = (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    appendAttachmentInputs(filesFromInputList(event.target.files))
    event.target.value = ''
  }

  const hasDraft = relation?.kind === 'edit' ? Boolean(inputValue.trim()) : hasAttachments || Boolean(inputValue.trim())
  const showMentions = mentionCandidates.length > 0 || canMentionAll

  return (
    <div
      ref={composerRef}
      data-testid="message-composer"
      aria-busy={pendingSends > 0}
      onKeyDown={(event) => { if (event.key === 'Escape') { setPickerOpen(false); setMentionOpen(false) } }}
      className="relative z-20 flex min-h-0 flex-shrink-0 flex-col"
      style={{
        borderTop: '1px solid var(--cp-border)',
        background: 'var(--cp-surface)',
        ...(maxHeight != null ? { maxHeight } : {}),
      }}
    >
      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={handleFileInputChange}
      />
      <input
        ref={directoryInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={handleFileInputChange}
      />

      {pendingSends > 0 && <p role="status" className="sr-only">{t('messagehub.sendingQueue', undefined, { count: pendingSends })}</p>}
      {oversizeNames.length > 0 && <p role="alert" className="px-3 pt-2 text-xs text-[color:var(--cp-danger)]" data-testid="attachment-too-large">{t('messagehub.attachmentTooLarge', undefined, { name: oversizeNames.join(', '), limit: formatAttachmentSize(MAX_ATTACHMENT_BYTES) })}</p>}
      {sendError && <p role="alert" className="px-3 pt-2 text-xs text-[color:var(--cp-danger)]">{t('messagehub.sendFailed')}{typeof sendError === 'string' && sendError !== 'true' ? ` ${describeSendError(sendError, t)}` : ''}</p>}
      {/* Anchored to the composer root: the message input area is overflow-hidden
          and would clip a menu popping upward from inside it. */}
      {pickerOpen ? (
        <div
          className="absolute bottom-full left-4 z-40 mb-2 w-40 rounded-2xl p-1.5 shadow-lg"
          style={{
            background: 'color-mix(in srgb, var(--cp-surface) 96%, white)',
            border: '1px solid var(--cp-border)',
          }}
        >
          <button
            className="flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-sm"
            style={{ color: 'var(--cp-text)' }}
            onClick={() => fileInputRef.current?.click()}
            type="button"
          >
            <File size={16} />
            {t('messagehub.pickFile', 'Choose file')}
          </button>
          <button
            className="flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-sm"
            style={{ color: 'var(--cp-text)' }}
            onClick={() => directoryInputRef.current?.click()}
            type="button"
          >
            <FolderOpen size={16} />
            {t('messagehub.pickFolder', 'Choose folder')}
          </button>
        </div>
      ) : null}

      {mentionOpen ? (
        <div role="listbox" aria-label={t('messagehub.mention.pick')} data-testid="mention-picker" className="shell-scrollbar absolute bottom-full left-4 z-40 mb-2 max-h-56 w-56 overflow-y-auto rounded-2xl p-1.5 shadow-lg" style={{ background: 'color-mix(in srgb, var(--cp-surface) 96%, white)', border: '1px solid var(--cp-border)' }}>
          {canMentionAll ? <button type="button" role="option" aria-selected={false} className="flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-sm font-medium" style={{ color: 'var(--cp-text)' }} onClick={() => insertMention('all')}><AtSign size={14} />{t('messagehub.mention.all')}</button> : null}
          {mentionCandidates.map(candidate => <button key={candidate.did} type="button" role="option" aria-selected={false} className="flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-sm" style={{ color: 'var(--cp-text)' }} onClick={() => insertMention(candidate)}><span className="truncate">{candidate.name}</span></button>)}
        </div>
      ) : null}

      {relation ? (
        <div className="flex items-center gap-2 border-b px-3 py-1.5 text-[12px]" style={{ borderColor: 'var(--cp-border)' }} data-testid="composer-relation" data-kind={relation.kind}>
          <span className="shrink-0 font-semibold" style={{ color: 'var(--cp-accent)' }}>{t(relation.kind === 'edit' ? 'messagehub.message.editing' : 'messagehub.message.replyingTo', undefined, { name: getMessageSenderName(relation.message) })}</span>
          <span className="min-w-0 flex-1 truncate" style={{ color: 'var(--cp-muted)' }}>{displayedContent(relation.message)}</span>
          <button type="button" className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full" aria-label={t('messagehub.cancel')} title={t('messagehub.cancel')} onClick={() => onCancelRelation?.()} style={{ color: 'var(--cp-muted)' }}><X size={14} /></button>
        </div>
      ) : null}

      {/* Message input area – top, max 50% of composer */}
      <div className="flex flex-shrink-0 flex-col px-2 py-2 md:px-3">
        <div
          className="relative flex min-w-0 items-end gap-0.5 rounded-[22px] border px-0.5 transition-[border-color,box-shadow] focus-within:border-[color:color-mix(in_srgb,var(--cp-accent)_55%,var(--cp-border))] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--cp-accent)_14%,transparent)]"
          style={{ background: 'var(--cp-message-canvas)', borderColor: 'var(--cp-border)' }}
        >
          <button
            className="flex min-h-11 min-w-11 flex-shrink-0 items-center justify-center rounded-full"
            style={{ color: 'var(--cp-muted)' }}
            onClick={() => setPickerOpen((previous) => !previous)}
            type="button"
            aria-label={t('messagehub.addAttachment', 'Add attachment')}
            aria-haspopup="menu"
            aria-expanded={pickerOpen}
            title={t('messagehub.addAttachment', 'Add attachment')}
          >
            <Paperclip size={18} />
          </button>
          {showMentions ? (
            <button
              className="flex min-h-11 min-w-11 flex-shrink-0 items-center justify-center rounded-full"
              style={{ color: mentionOpen ? 'var(--cp-accent)' : 'var(--cp-muted)' }}
              onClick={() => setMentionOpen((previous) => !previous)}
              type="button"
              aria-label={t('messagehub.mention.add')}
              aria-haspopup="listbox"
              aria-expanded={mentionOpen}
              title={t('messagehub.mention.add')}
              data-testid="mention-button"
            >
              <AtSign size={18} />
            </button>
          ) : null}

          <textarea
            ref={inputRef}
            value={inputValue}
            onChange={(event) => setInputValue(event.target.value)}
            onKeyDown={handleKeyDown}
            onPaste={(event) => {
              void handlePaste(event)
            }}
            placeholder={placeholder}
            rows={1}
            aria-label={placeholder}
            className="block min-h-11 min-w-0 flex-1 resize-none border-none bg-transparent px-1 py-[10px] text-[16px] leading-6 outline-none md:text-[15px]"
            style={{
              color: 'var(--cp-text)',
              overflowY: 'hidden',
            }}
          />
          <button
            onClick={handleSend}
            disabled={!hasDraft}
            className="group flex min-h-11 min-w-11 flex-shrink-0 items-center justify-center disabled:cursor-not-allowed"
            type="button"
            aria-label={t('messagehub.send', 'Send')}
            title={t('messagehub.send', 'Send')}
          >
            <span
              className="flex h-8 w-8 items-center justify-center rounded-full transition-colors"
              style={{
                background: hasDraft
                  ? 'var(--cp-message-self-bg)'
                  : 'color-mix(in srgb, var(--cp-text) 8%, transparent)',
                color: hasDraft ? 'var(--cp-message-self-text)' : 'color-mix(in srgb, var(--cp-muted) 70%, transparent)',
              }}
            >
              <Send size={16} />
            </span>
          </button>
        </div>
      </div>

      {/* Attachment area – bottom, fills remaining space */}
      {hasAttachments ? (
        <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-2">
          <div
            className="flex items-center justify-between gap-3 px-1"
          >
            <div className="min-w-0">
              <p
                className="text-[11px] font-medium"
                style={{ color: 'var(--cp-text)' }}
              >
                {t('messagehub.attachmentsReady', 'Will send ({{count}}) items', {
                  count: attachments.length,
                })}
              </p>
            </div>
            <button
              className="rounded-md px-1.5 py-0.5 text-[11px]"
              style={{
                color: 'var(--cp-muted)',
              }}
              onClick={clearAttachments}
              type="button"
            >
              {t('messagehub.clearAttachments', 'Clear')}
            </button>
          </div>

          <div className="pt-1.5">
            <div className="relative z-0 grid grid-cols-2 gap-1.5 px-1 pb-1 sm:grid-cols-3">
              {attachments.map((attachment) => (
                <MemoAttachmentCard
                  key={attachment.id}
                  attachment={attachment}
                  onRemove={handleRemoveAttachment}
                />
              ))}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  )
})

ConversationComposerInner.displayName = 'ConversationComposer'

/**
 * Surface the backend's real rejection / unknown-result reason next to the
 * generic retry hint. Reasons are prefixed by the store (`rejected: …`,
 * `result_unknown: …`); other messages map to i18n keys when known.
 */
function describeSendError(message: string, t: (key: string, fallback?: string, variables?: Record<string, string | number>) => string): string {
  const [prefix, ...rest] = message.split(':')
  const detail = rest.join(':').trim()
  switch (prefix.trim()) {
    case 'rejected': return t('messagehub.sendRejected', undefined, { reason: detail || 'unknown' })
    case 'result_unknown': return t('messagehub.sendResultUnknown')
    case 'attachment_upload_unavailable': return t('messagehub.attachmentUploadUnavailable')
    case 'attachment_upload_failed': return t('messagehub.attachmentUploadFailed')
    case 'attachment_too_large': return t('messagehub.attachmentTooLarge', undefined, { name: detail, limit: formatAttachmentSize(MAX_ATTACHMENT_BYTES) })
    case 'permission_denied': return t('messagehub.reason.permission_denied')
    default: return ''
  }
}

export const ConversationComposer = memo(ConversationComposerInner)

const MemoAttachmentCard = memo(function AttachmentCard({
  attachment,
  onRemove,
}: {
  attachment: ComposerAttachmentItem
  onRemove: (id: string) => void
}) {
  const { t } = useI18n()
  const displayPath = attachment.relativePath || attachment.file.name
  const metaLine = `${attachment.file.name} · ${formatAttachmentSize(attachment.file.size)}`

  return (
    <div
      className="relative overflow-hidden rounded-[18px]"
      style={{
        border: '1px solid color-mix(in srgb, var(--cp-border) 72%, transparent)',
        background: 'color-mix(in srgb, var(--cp-surface) 95%, white)',
      }}
    >
      <button
        className="absolute right-0 top-0 z-10 flex h-8 w-8 items-center justify-center"
        onClick={() => onRemove(attachment.id)}
        type="button"
        aria-label={t('messagehub.removeAttachment', 'Remove {{name}}', { name: displayPath })}
        title={t('messagehub.removeAttachment', 'Remove {{name}}', { name: displayPath })}
      >
        <span className="rounded-full p-1" style={{ background: 'rgba(15, 23, 42, 0.72)', color: '#fff' }}>
          <X size={12} />
        </span>
      </button>

      {attachment.previewUrl || attachment.kind === 'image' ? (
        <div className="relative h-24 overflow-hidden">
          {attachment.previewUrl ? (
            <img
              alt={attachment.file.name}
              className="h-full w-full object-cover"
              src={attachment.previewUrl}
            />
          ) : (
            <div
              className="flex h-full w-full items-center justify-center"
              style={{
                background: 'color-mix(in srgb, var(--cp-accent) 14%, transparent)',
              }}
            >
              <ImageIcon size={24} style={{ color: 'var(--cp-muted)' }} />
            </div>
          )}

          <div
            className="absolute inset-x-0 bottom-0 px-2.5 py-2"
            style={{
              background: 'linear-gradient(180deg, transparent, rgba(15, 23, 42, 0.82))',
            }}
          >
            <p
              className="truncate text-[11px] font-medium text-white"
              title={displayPath}
            >
              {metaLine}
            </p>
          </div>
        </div>
      ) : (
        <div className="flex items-center gap-2.5 px-2.5 py-2.5">
          <FileGlyph filename={attachment.file.name} />
          <div className="min-w-0 flex-1">
            <p
              className="truncate text-[11px] font-medium"
              style={{ color: 'var(--cp-text)' }}
              title={displayPath}
            >
              {metaLine}
            </p>
          </div>
        </div>
      )}
    </div>
  )
})

MemoAttachmentCard.displayName = 'AttachmentCard'

function FileGlyph({ filename }: { filename: string }) {
  const extension = getFileExtension(filename)

  return (
    <div
      className="relative h-12 w-10 flex-shrink-0 overflow-hidden rounded-[12px]"
      style={{
        background: 'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 86%, white), color-mix(in srgb, var(--cp-text) 4%, transparent))',
        border: '1px solid color-mix(in srgb, var(--cp-border) 92%, transparent)',
        boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.65)',
      }}
    >
      <div
        className="absolute right-0 top-0 h-4 w-4"
        style={{
          background: 'linear-gradient(135deg, color-mix(in srgb, var(--cp-border) 72%, white) 0%, color-mix(in srgb, var(--cp-surface) 96%, white) 100%)',
          clipPath: 'polygon(0 0, 100% 0, 100% 100%)',
        }}
      />
      <div className="flex h-full flex-col justify-between px-2 py-2">
        <File size={18} style={{ color: 'var(--cp-muted)' }} />
        <span
          className="truncate text-[9px] font-semibold uppercase tracking-[0.08em]"
          style={{ color: 'var(--cp-accent)' }}
        >
          {extension}
        </span>
      </div>
    </div>
  )
}

function getFileExtension(filename: string): string {
  const normalized = filename.split('/').pop() ?? filename
  const parts = normalized.split('.')

  if (parts.length < 2) {
    return 'FILE'
  }

  return parts.at(-1)?.slice(0, 4).toUpperCase() || 'FILE'
}

function insertTextAtSelection(
  nextText: string,
  currentValue: string,
  setValue: (value: string) => void,
  textarea: HTMLTextAreaElement | null,
) {
  if (!textarea) {
    setValue(`${currentValue}${nextText}`)
    return
  }

  const selectionStart = textarea.selectionStart ?? currentValue.length
  const selectionEnd = textarea.selectionEnd ?? currentValue.length
  const updatedValue = [
    currentValue.slice(0, selectionStart),
    nextText,
    currentValue.slice(selectionEnd),
  ].join('')

  setValue(updatedValue)

  requestAnimationFrame(() => {
    // Typing may already have continued before this frame; never move the
    // caret back behind text the user has entered since the insert.
    if (textarea.value !== updatedValue) return
    const caret = selectionStart + nextText.length
    textarea.focus()
    textarea.setSelectionRange(caret, caret)
  })
}
