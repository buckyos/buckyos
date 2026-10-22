import {
  forwardRef,
  memo,
  useCallback,
  useEffect,
  useId,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  AtSign,
  Circle,
  CircleDot,
  File,
  FolderOpen,
  ImageIcon,
  ListChecks,
  Mic,
  Paperclip,
  Send,
  Smile,
  Square,
  X,
} from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { displayedContent, effectiveContent, messageObjId } from '../history/relations'
import { agentAskQuestionText, buildAgentAskAnswer, getMessageSenderName, type MachineContent, type MessageObject, type MsgMentions, type MsgRelation, type VoiceNote } from '../../protocol/msgobj'
import { formatVoiceDuration, voiceRecordingSupported } from '../media/voice'
import { EmojiPanel } from './EmojiPanel'
import { VoiceRecorder } from './VoiceRecorder'
import { isStopCommand, mergeMentions, type AnswerInputState } from './inputState'
import { applyMention, collectMentions, matchMentionCandidates, mentionQueryAt, type MentionCandidate, type MentionQuery } from './mentions'
import {
  createAttachmentItem,
  extractTransferFiles,
  filesFromInputList,
  formatAttachmentSize,
  getAttachmentKey,
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
  return a.length === b.length && a.every((item, index) => item.file === b[index].file && item.relativePath === b[index].relativePath && item.voice === b[index].voice)
}

const attachmentInput = ({ file, relativePath, voice }: ComposerAttachmentInput): ComposerAttachmentInput => ({ file, relativePath, ...(voice ? { voice } : {}) })

export interface ConversationComposerSubmitPayload {
  attachments: ComposerAttachmentItem[]
  content: string
  /** MsgObject v2 `relates_to` of a reply (`thread`) or an edit of an own message. */
  relatesTo?: MsgRelation
  /** Structured mentions picked in the composer; absent when none. */
  mentions?: MsgMentions
  /** `content.machine`: an answer that picked an option (`agent_ask.answer`). */
  machine?: MachineContent
  /** The only attachment is a voice note recorded in the composer. */
  voice?: VoiceNote
  localId?: string
  submittedAt?: number
}

/** The question the conversation's input state locks the composer to, and who asked it. */
export interface ComposerAnswer extends AnswerInputState {
  askerName: string
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
  /** Submitted payloads not sent yet, the one being sent included; called whenever that list changes. */
  onQueueChange?: (queued: ConversationComposerSubmitPayload[]) => void
  initialDraft?: string
  initialAttachments?: ComposerAttachmentInput[]
  onAttachmentsChange?: (attachments: ComposerAttachmentInput[]) => Promise<void> | undefined
  onDraftChange?: (value: string) => Promise<void> | undefined
  relation?: ComposerRelation | null
  onCancelRelation?: () => void
  /**
   * Set by the conversation, not by the user (`inputState.ts`): everything
   * sent answers this question, there is no way to cancel it, and a choice
   * question replaces the input with its options. `/stop` still goes out as
   * the command it is.
   */
  answer?: ComposerAnswer | null
  /** Members offered while typing `@` and by the @ button (group sessions only). */
  mentionCandidates?: MentionCandidate[]
  canMentionAll?: boolean
  /** false: text only, no files, folders, pasted or dropped files or voice notes (Session Only guests, v1). */
  allowFiles?: boolean
}


const ConversationComposerInner = forwardRef<
  ConversationComposerHandle,
  ConversationComposerProps
>(function ConversationComposer(
  { placeholder, maxHeight, onSendMessage, onQueueChange, initialDraft = '', initialAttachments = [], onAttachmentsChange, onDraftChange, relation = null, onCancelRelation, answer = null, mentionCandidates = [], canMentionAll = false, allowFiles = true },
  ref,
) {
  const { t } = useI18n()
  const [attachments, setAttachments] = useState<ComposerAttachmentItem[]>(() => initialAttachments.map(createAttachmentItem))
  const [inputValue, setInputValue] = useState(initialDraft)
  // The `@word` at the caret drives the suggestions; Escape dismisses them
  // for that `@` until the caret leaves it.
  const [mentionQuery, setMentionQuery] = useState<MentionQuery | null>(null)
  const [dismissedMention, setDismissedMention] = useState<number | null>(null)
  const [activeMention, setActiveMention] = useState({ key: '', index: 0 })
  const mentionListId = useId()
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
  const queueCallback = useRef(onQueueChange)
  useEffect(() => { queueCallback.current = onQueueChange }, [onQueueChange])
  const reportQueue = useCallback(() => queueCallback.current?.([...sendQueue.current]), [])
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
    const inputs = attachments.map(attachmentInput)
    if (sameAttachmentInputs(inputs, persistedAttachments.current)) return
    persistedAttachments.current = inputs
    flushDraft()
    if (attachmentsCallback.current) void attachmentsCallback.current(inputs)?.catch(() => setSendError('true'))
  }, [attachments, flushDraft])
  const [pickerOpen, setPickerOpen] = useState(false)
  const [emojiOpen, setEmojiOpen] = useState(false)
  const [recording, setRecording] = useState(false)
  const [canRecord] = useState(voiceRecordingSupported)
  const [canPickFolder] = useState(() => 'webkitdirectory' in document.createElement('input') && !window.matchMedia?.('(pointer: coarse)').matches)
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
    setMentionQuery(null)
    if (relation?.kind === 'edit') {
      if (draftBeforeEdit === null) setDraftBeforeEdit(inputValue)
      setInputValue(displayedContent(relation.message))
    } else if (draftBeforeEdit !== null) {
      setInputValue(draftBeforeEdit)
      setDraftBeforeEdit(null)
    }
  }
  useEffect(() => { if (relation) inputRef.current?.focus() }, [relation])

  // A choice question shows its options instead of the input until the user
  // picks "Other…" (allowed only when the question accepts free text).
  const answerKey = answer ? messageObjId(answer.question) ?? answer.ask.call_id : null
  const [freeTextFor, setFreeTextFor] = useState<string | null>(null)
  const choice = answer?.ask.input.kind === 'choice' && freeTextFor !== answerKey ? answer : null
  const showInput = !choice
  const questionText = answer ? agentAskQuestionText(effectiveContent(answer.question).content ?? '', answer.ask) : ''
  const [directSend, setDirectSend] = useState<string | null>(null)

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
  }, [inputValue, messageInputMaxHeight, showInput])

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

  const mentionOptions = useMemo(() => mentionQuery ? matchMentionCandidates(mentionCandidates, mentionQuery.query, canMentionAll) : [], [mentionQuery, mentionCandidates, canMentionAll])
  const mentionOpen = mentionQuery !== null && mentionQuery.start !== dismissedMention && mentionOptions.length > 0
  const mentionKey = mentionQuery ? `${mentionQuery.start}:${mentionQuery.query}` : ''
  const activeMentionIndex = activeMention.key === mentionKey ? Math.min(activeMention.index, mentionOptions.length - 1) : 0
  const mentionOptionId = (index: number) => `${mentionListId}-option-${index}`

  const trackMention = useCallback((text: string, element: HTMLTextAreaElement) => {
    const next = element.selectionStart === element.selectionEnd ? mentionQueryAt(text, element.selectionStart) : null
    setMentionQuery(previous => previous?.start === next?.start && previous?.end === next?.end && previous?.query === next?.query ? previous : next)
    if (!next) setDismissedMention(null)
  }, [])

  useEffect(() => {
    if (mentionOpen) document.getElementById(mentionOptionId(activeMentionIndex))?.scrollIntoView({ block: 'nearest' })
  }, [mentionOpen, activeMentionIndex]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!pickerOpen && !mentionOpen && !emojiOpen) {
      return
    }

    const handlePointerDown = (event: MouseEvent) => {
      if (!composerRef.current?.contains(event.target as Node)) {
        setPickerOpen(false)
        setMentionQuery(null)
        setEmojiOpen(false)
      }
    }

    document.addEventListener('pointerdown', handlePointerDown)

    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
    }
  }, [pickerOpen, mentionOpen, emojiOpen])

  const appendAttachmentInputs = useCallback((selected: ComposerAttachmentInput[]) => {
    const items = selected.filter(item => item.file.size <= MAX_ATTACHMENT_BYTES)
    setOversizeNames(selected.filter(item => item.file.size > MAX_ATTACHMENT_BYTES).map(item => item.relativePath || item.file.name))
    if (items.length === 0) {
      return
    }

    setAttachments((previous) => {
      const existingKeys = new Set(previous.map(getAttachmentKey))
      const nextInputs: ComposerAttachmentInput[] = []

      items.forEach((item) => {
        const itemKey = getAttachmentKey(item)

        if (existingKeys.has(itemKey)) {
          return
        }

        existingKeys.add(itemKey)
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
      const inputs = [...items, ...attachmentsRef.current].map(attachmentInput)
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
      const keys = new Set(current.map(getAttachmentKey))
      return [...items.filter(item => !keys.has(getAttachmentKey(item))), ...current]
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
          reportQueue()
          restoreFailedSends(failed)
          if (mounted.current) setSendError(error instanceof Error && error.message ? error.message : 'true')
          break
        }
        sendQueue.current.shift()
        reportQueue()
        next.attachments.forEach(revokeAttachmentItem)
        if (mounted.current) setPendingSends(sendQueue.current.length)
      }
    } finally {
      draining.current = false
      if (mounted.current) setPendingSends(sendQueue.current.length)
    }
  }, [reportQueue, restoreFailedSends])

  /** Queues one message (the reply, edit or answer in progress applies to it) and starts sending. */
  const enqueue = useCallback((text: string, items: ComposerAttachmentItem[]) => {
    const target = relation ? messageObjId(relation.message) : undefined
    // In the answer state everything typed answers the question, except a
    // bare `/stop`, which the Agent takes as a command on its own.
    const reply = answer && !(isStopCommand(text) && items.length === 0) ? buildAgentAskAnswer(answer.question, answer.ask, { text }) : undefined
    const relatesTo: MsgRelation | undefined = answer ? reply?.relates_to : relation && target ? { rel: relation.kind === 'edit' ? 'edit' : 'thread', target } : undefined
    const mentions = mergeMentions(collectMentions(text, picked, mentionAll), answer?.mentions)
    const editing = relation?.kind === 'edit'
    const voice = !editing && items.length === 1 ? items[0].voice : undefined
    sendQueue.current.push({ attachments: editing ? [] : items, content: text, ...(voice ? { voice } : {}), ...(relatesTo ? { relatesTo } : {}), ...(mentions ? { mentions } : {}), localId: crypto.randomUUID(), submittedAt: Date.now() })
    reportQueue()
    if (relation) onCancelRelation?.()
    setPicked([]); setMentionAll(false); setMentionQuery(null)
    setPendingSends(sendQueue.current.length)
    setSendError(false)
    void drainSendQueue()
  }, [drainSendQueue, relation, onCancelRelation, picked, mentionAll, answer, reportQueue])

  const handleSend = useCallback(() => {
    const text = inputValue.trim()

    if (!text && attachments.length === 0) {
      return
    }

    enqueue(text, attachments)
    setOversizeNames([])
    setInputValue('')
    latestDraft.current = ''
    flushDraft()
    setAttachments([])
    inputRef.current?.focus()
  }, [attachments, enqueue, flushDraft, inputValue])

  const sendVoice = useCallback((file: File, voice: VoiceNote) => {
    enqueue('', [createAttachmentItem({ file, voice })])
  }, [enqueue])

  const handleRecordingChange = useCallback((active: boolean) => {
    setRecording(active)
    if (active) { setEmojiOpen(false); setPickerOpen(false) }
  }, [])

  /** Puts the emoji at the caret; the input keeps its focus (and a phone its keyboard state). */
  const insertEmoji = useCallback((emoji: string) => {
    const textarea = inputRef.current
    const start = textarea?.selectionStart ?? inputValue.length
    const end = textarea?.selectionEnd ?? inputValue.length
    const text = `${inputValue.slice(0, start)}${emoji}${inputValue.slice(end)}`
    const caret = start + emoji.length
    setInputValue(text)
    requestAnimationFrame(() => {
      if (textarea?.value === text) textarea.setSelectionRange(caret, caret)
    })
  }, [inputValue])

  const closeEmoji = useCallback(() => {
    setEmojiOpen(false)
    inputRef.current?.focus()
  }, [])

  /**
   * A picked option or the stop button: sent at once, apart from the typed
   * draft, which stays as it is. `key` marks what is being sent.
   */
  const sendDirect = useCallback(async (key: string, payload: ConversationComposerSubmitPayload) => {
    setDirectSend(key)
    setSendError(false)
    try {
      await sendCallback.current(payload)
    } catch (error) {
      if (mounted.current) setSendError(error instanceof Error && error.message ? error.message : 'true')
    } finally {
      if (mounted.current) setDirectSend(null)
    }
  }, [])

  const pickOption = useCallback((option: string) => {
    if (!answer) return
    const reply = buildAgentAskAnswer(answer.question, answer.ask, { option })
    if (!reply) return
    void sendDirect(`option:${option}`, { attachments: [], content: reply.text, relatesTo: reply.relates_to, ...(reply.machine ? { machine: reply.machine } : {}), ...(answer.mentions ? { mentions: answer.mentions } : {}) })
  }, [answer, sendDirect])

  const sendStop = useCallback(() => {
    void sendDirect('stop', { attachments: [], content: '/stop', ...(answer?.mentions ? { mentions: answer.mentions } : {}) })
  }, [answer, sendDirect])

  /** Replaces the `@word` at the caret with the picked mention. */
  const pickMention = useCallback((option: MentionCandidate | 'all') => {
    if (!mentionQuery) return
    const { text, caret } = applyMention(inputValue, mentionQuery, option)
    if (option === 'all') setMentionAll(true)
    else setPicked(previous => previous.some(item => item.did === option.did) ? previous : [...previous, option])
    setInputValue(text)
    setMentionQuery(null)
    placeCaret(inputRef.current, text, caret)
  }, [inputValue, mentionQuery])

  /** The @ button starts a mention at the caret, or closes the open suggestions. */
  const toggleMentions = useCallback(() => {
    const textarea = inputRef.current
    if (mentionQuery && mentionOptions.length > 0) {
      setDismissedMention(mentionOpen ? mentionQuery.start : null)
      textarea?.focus()
      return
    }
    const start = textarea?.selectionStart ?? inputValue.length
    const end = textarea?.selectionEnd ?? inputValue.length
    const marker = start > 0 && !/\s/.test(inputValue[start - 1]) ? ' @' : '@'
    const text = `${inputValue.slice(0, start)}${marker}${inputValue.slice(end)}`
    const caret = start + marker.length
    setInputValue(text)
    setDismissedMention(null)
    setMentionQuery(mentionQueryAt(text, caret))
    placeCaret(textarea, text, caret)
  }, [inputValue, mentionOpen, mentionOptions.length, mentionQuery])

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (mentionOpen && mentionQuery && !event.nativeEvent.isComposing) {
      const count = mentionOptions.length
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault()
        setActiveMention({ key: mentionKey, index: (activeMentionIndex + (event.key === 'ArrowDown' ? 1 : count - 1)) % count })
        return
      }
      if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') {
        event.preventDefault()
        pickMention(mentionOptions[activeMentionIndex])
        return
      }
      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        setDismissedMention(mentionQuery.start)
        return
      }
    }

    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      if (!event.nativeEvent.isComposing) handleSend()
    }

    if (event.key === 'Escape') {
      setPickerOpen(false)
      if (emojiOpen) setEmojiOpen(false)
      else if (relation) onCancelRelation?.()
    }
  }

  const addTransferData = useCallback(async (dataTransfer: DataTransfer) => {
    if (!allowFiles) return
    const transferFiles = await extractTransferFiles(dataTransfer)
    appendAttachmentInputs(transferFiles)
  }, [appendAttachmentInputs, allowFiles])

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

    // Without files the browser pastes the text as usual.
    if (!hasFileItems || !allowFiles) {
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
  // An empty input offers a voice note in place of the send button (Telegram);
  // a recording keeps it until the note is sent or discarded.
  const showVoice = allowFiles && canRecord && showInput && (recording || (!hasDraft && relation?.kind !== 'edit'))
  const showMentions = mentionCandidates.length > 0 || canMentionAll
  const answerBusy = directSend !== null || pendingSends > 0
  const stopButton = answer ? (
    <button type="button" className="mh-answer-action" disabled={answerBusy} onClick={sendStop} title={t('messagehub.answer.stopHint')} data-testid="answer-stop">
      <Square size={11} aria-hidden />{t('messagehub.answer.stop')}
    </button>
  ) : null

  return (
    <div
      ref={composerRef}
      data-testid="message-composer"
      aria-busy={pendingSends > 0}
      onKeyDown={(event) => { if (event.key === 'Escape') { setPickerOpen(false); setEmojiOpen(false) } }}
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

      {emojiOpen && showInput ? (
        <div className="absolute bottom-full left-2 right-2 z-40 mb-2 flex justify-end sm:left-auto sm:right-3">
          <EmojiPanel onPick={insertEmoji} onClose={closeEmoji} />
        </div>
      ) : null}

      {mentionOpen ? (
        <div id={mentionListId} role="listbox" aria-label={t('messagehub.mention.pick')} data-testid="mention-picker" className="shell-scrollbar absolute bottom-full left-4 z-40 mb-2 max-h-56 w-60 max-w-[calc(100%-2rem)] overflow-y-auto rounded-2xl p-1.5 shadow-lg" style={{ background: 'color-mix(in srgb, var(--cp-surface) 96%, white)', border: '1px solid var(--cp-border)' }}>
          {mentionOptions.map((option, index) => {
            const active = index === activeMentionIndex
            return (
              <div
                key={option === 'all' ? 'all' : option.did}
                id={mentionOptionId(index)}
                role="option"
                aria-selected={active}
                className={`flex min-h-10 w-full cursor-pointer items-center gap-2 rounded-xl px-3 text-left text-sm ${option === 'all' ? 'font-medium' : ''}`}
                style={{ color: 'var(--cp-text)', background: active ? 'color-mix(in srgb, var(--cp-accent) 12%, transparent)' : undefined }}
                // Keep the caret in the input: picking replaces the `@word` around it.
                onMouseDown={(event) => event.preventDefault()}
                onMouseEnter={() => setActiveMention({ key: mentionKey, index })}
                onClick={() => pickMention(option)}
              >
                {option === 'all' ? <><AtSign size={14} aria-hidden />{t('messagehub.mention.all')}</> : <span className="truncate">{option.name}</span>}
              </div>
            )
          })}
        </div>
      ) : null}

      {answer && !choice ? (
        <div className="flex items-center gap-2 border-b px-3 py-1 text-[12px]" style={{ borderColor: 'var(--cp-border)' }} data-testid="composer-answer" data-kind="text">
          <span className="shrink-0 font-semibold" style={{ color: 'var(--cp-accent)' }}>{t('messagehub.answer.banner', undefined, { name: answer.askerName })}</span>
          <span className="min-w-0 flex-1 truncate" style={{ color: 'var(--cp-muted)' }} title={questionText}>{questionText}</span>
          {answer.ask.input.kind === 'choice' ? <button type="button" className="mh-answer-action" onClick={() => setFreeTextFor(null)} data-testid="answer-back-to-options"><ListChecks size={13} aria-hidden />{t('messagehub.answer.backToOptions')}</button> : null}
          {stopButton}
        </div>
      ) : null}

      {choice ? (
        <div className="flex flex-col gap-2 px-3 pb-3 pt-2" data-testid="composer-answer" data-kind="choice">
          <div className="flex items-center gap-2 text-[12px]">
            <span className="min-w-0 flex-1 truncate font-semibold" style={{ color: 'var(--cp-accent)' }}>{t('messagehub.answer.banner', undefined, { name: choice.askerName })}</span>
            {stopButton}
          </div>
          {questionText ? <p className="max-h-24 overflow-y-auto whitespace-pre-wrap break-words text-[14px] leading-[1.45]" style={{ color: 'var(--cp-text)' }} data-testid="answer-question">{questionText}</p> : null}
          <div role="group" aria-label={t('messagehub.answer.options')} className="flex max-h-60 flex-col gap-1.5 overflow-y-auto">
            {choice.ask.input.options.map(option => {
              const sending = directSend === `option:${option.id}`
              return (
                <button key={option.id} type="button" className="mh-answer-option" aria-pressed={sending} disabled={answerBusy} onClick={() => pickOption(option.id)} data-testid="answer-option" data-option={option.id}>
                  {sending ? <CircleDot size={16} aria-hidden style={{ color: 'var(--cp-accent)' }} /> : <Circle size={16} aria-hidden style={{ color: 'var(--cp-muted)' }} />}
                  <span className="min-w-0 flex-1 break-words">{option.label}</span>
                </button>
              )
            })}
            {choice.ask.input.allow_text ? (
              <button type="button" className="mh-answer-option" style={{ color: 'var(--cp-muted)' }} disabled={answerBusy} onClick={() => { setFreeTextFor(answerKey); requestAnimationFrame(() => inputRef.current?.focus()) }} data-testid="answer-other">
                <Circle size={16} aria-hidden />
                <span className="min-w-0 flex-1">{t('messagehub.answer.other')}</span>
              </button>
            ) : null}
          </div>
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
      {showInput ? <div className="flex flex-shrink-0 flex-col px-2 py-2 md:px-3">
        <div
          className="relative flex min-w-0 items-end gap-0.5 rounded-[22px] border px-0.5 transition-[border-color,box-shadow] focus-within:border-[color:color-mix(in_srgb,var(--cp-accent)_55%,var(--cp-border))] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--cp-accent)_14%,transparent)]"
          style={{ background: 'var(--cp-message-canvas)', borderColor: 'var(--cp-border)' }}
        >
          {allowFiles ? (
            <button
              className="flex min-h-11 min-w-11 flex-shrink-0 items-center justify-center rounded-full"
              style={{ color: 'var(--cp-muted)' }}
              onClick={() => { if (canPickFolder) setPickerOpen((previous) => !previous); else fileInputRef.current?.click() }}
              type="button"
              aria-label={t('messagehub.addAttachment', 'Add attachment')}
              aria-haspopup={canPickFolder ? 'menu' : undefined}
              aria-expanded={canPickFolder ? pickerOpen : undefined}
              title={t('messagehub.addAttachment', 'Add attachment')}
            >
              <Paperclip size={18} />
            </button>
          ) : null}
          {showMentions ? (
            <button
              className="flex min-h-11 min-w-11 flex-shrink-0 items-center justify-center rounded-full"
              style={{ color: mentionOpen ? 'var(--cp-accent)' : 'var(--cp-muted)' }}
              onMouseDown={(event) => event.preventDefault()}
              onClick={toggleMentions}
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
            onChange={(event) => { setInputValue(event.target.value); trackMention(event.target.value, event.target) }}
            onSelect={(event) => trackMention(event.currentTarget.value, event.currentTarget)}
            onKeyDown={handleKeyDown}
            onPaste={(event) => {
              void handlePaste(event)
            }}
            placeholder={answer ? t('messagehub.answer.placeholder') : placeholder}
            rows={1}
            aria-label={answer ? t('messagehub.answer.placeholder') : placeholder}
            aria-autocomplete={showMentions ? 'list' : undefined}
            aria-controls={mentionOpen ? mentionListId : undefined}
            aria-activedescendant={mentionOpen ? mentionOptionId(activeMentionIndex) : undefined}
            className="block min-h-11 min-w-0 flex-1 resize-none border-none bg-transparent px-1 py-[10px] text-[16px] leading-6 outline-none md:text-[15px]"
            style={{
              color: 'var(--cp-text)',
              overflowY: 'hidden',
            }}
          />
          <button
            className="flex min-h-11 min-w-11 flex-shrink-0 items-center justify-center rounded-full"
            style={{ color: emojiOpen ? 'var(--cp-accent)' : 'var(--cp-muted)' }}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              setPickerOpen(false)
              setEmojiOpen(previous => !previous)
              if (window.matchMedia?.('(pointer: coarse)').matches) inputRef.current?.blur()
            }}
            type="button"
            aria-label={t('messagehub.emoji.open')}
            aria-haspopup="dialog"
            aria-expanded={emojiOpen}
            title={t('messagehub.emoji.open')}
            data-testid="emoji-button"
          >
            <Smile size={19} />
          </button>
          {showVoice ? <VoiceRecorder onSend={sendVoice} onActiveChange={handleRecordingChange} /> : (
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
          )}
        </div>
      </div> : null}

      {/* Attachment area – bottom, fills remaining space */}
      {hasAttachments && showInput ? (
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
    case 'rejected':
      // A Limited account has no personal inbox: point to group sessions instead.
      if (detail.startsWith('recipient-has-no-inbox')) return t('messagehub.sendNoInbox')
      return t('messagehub.sendRejected', undefined, { reason: detail || 'unknown' })
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
      ) : attachment.voice ? (
        <div className="flex items-center gap-2.5 px-2.5 py-2.5" data-testid="voice-draft">
          <span className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-full" style={{ background: 'var(--cp-accent)', color: 'var(--cp-message-self-text)' }}>
            <Mic size={18} aria-hidden />
          </span>
          <p className="min-w-0 flex-1 truncate text-[11px] font-medium" style={{ color: 'var(--cp-text)' }} title={displayPath}>
            {t('messagehub.voice.label')} · {formatVoiceDuration(attachment.voice.duration_ms)}
          </p>
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

/** Moves the caret once React has rendered `text`, unless typing has changed it since. */
function placeCaret(textarea: HTMLTextAreaElement | null, text: string, caret: number) {
  requestAnimationFrame(() => {
    if (!textarea || textarea.value !== text) return
    textarea.focus()
    textarea.setSelectionRange(caret, caret)
  })
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
