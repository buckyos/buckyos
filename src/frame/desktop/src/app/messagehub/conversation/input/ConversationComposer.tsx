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
  File,
  FolderOpen,
  ImageIcon,
  Paperclip,
  Send,
  X,
} from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import {
  createAttachmentItem,
  extractTransferFiles,
  filesFromInputList,
  formatAttachmentSize,
  getAttachmentPathKey,
  revokeAttachmentItem,
  type ComposerAttachmentInput,
  type ComposerAttachmentItem,
} from './attachmentDraft'

export interface ConversationComposerSubmitPayload {
  attachments: ComposerAttachmentItem[]
  content: string
}

export interface ConversationComposerHandle {
  addTransferData: (dataTransfer: DataTransfer) => Promise<void>
}

interface ConversationComposerProps {
  placeholder: string
  /** Max height for the entire composer (in px). Used to compute inner constraints. */
  maxHeight?: number
  onSendMessage: (payload: ConversationComposerSubmitPayload) => void
}

const ConversationComposerInner = forwardRef<
  ConversationComposerHandle,
  ConversationComposerProps
>(function ConversationComposer(
  { placeholder, maxHeight, onSendMessage },
  ref,
) {
  const { t } = useI18n()
  const [attachments, setAttachments] = useState<ComposerAttachmentItem[]>([])
  const [inputValue, setInputValue] = useState('')
  const [pickerOpen, setPickerOpen] = useState(false)
  const attachmentsRef = useRef<ComposerAttachmentItem[]>([])
  const composerRef = useRef<HTMLDivElement>(null)
  const messageInputContainerRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const directoryInputRef = useRef<HTMLInputElement>(null)

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
    element.style.height = `${scrollHeight}px`

    // After layout, check if the message container clips the textarea
    requestAnimationFrame(() => {
      const container = messageInputContainerRef.current
      if (container) {
        const isOverflowing = container.scrollHeight > container.clientHeight
        element.style.overflow = isOverflowing ? 'auto' : 'hidden'
      }
    })
  }, [inputValue, maxHeight])

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
    if (!pickerOpen) {
      return
    }

    const handlePointerDown = (event: MouseEvent) => {
      if (!composerRef.current?.contains(event.target as Node)) {
        setPickerOpen(false)
      }
    }

    document.addEventListener('pointerdown', handlePointerDown)

    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
    }
  }, [pickerOpen])

  const appendAttachmentInputs = useCallback((items: ComposerAttachmentInput[]) => {
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

  const handleSend = useCallback(() => {
    const text = inputValue.trim()

    if (!text && attachments.length === 0) {
      return
    }

    onSendMessage({
      attachments,
      content: text,
    })
    setInputValue('')
    clearAttachments()
    inputRef.current?.focus()
  }, [attachments, clearAttachments, inputValue, onSendMessage])

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      handleSend()
    }

    if (event.key === 'Escape') {
      setPickerOpen(false)
    }
  }

  const addTransferData = useCallback(async (dataTransfer: DataTransfer) => {
    const transferFiles = await extractTransferFiles(dataTransfer)
    appendAttachmentInputs(transferFiles)
  }, [appendAttachmentInputs])

  useImperativeHandle(ref, () => ({
    addTransferData,
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

  const hasDraft = hasAttachments || Boolean(inputValue.trim())

  return (
    <div
      ref={composerRef}
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

      {/* Message input area – top, max 50% of composer */}
      <div
        ref={messageInputContainerRef}
        className="flex flex-shrink-0 flex-col px-3 pt-2 pb-1 overflow-hidden"
        style={messageInputMaxHeight != null ? { maxHeight: messageInputMaxHeight } : undefined}
      >
        <div className="relative flex min-w-0 items-end gap-2 px-1 py-1">
          <div className="relative flex-shrink-0">
            <button
              className="p-1 rounded-lg flex-shrink-0"
              style={{ color: 'var(--cp-muted)' }}
              onClick={() => setPickerOpen((previous) => !previous)}
              type="button"
            >
              <Paperclip size={18} />
            </button>

            {pickerOpen ? (
              <div
                className="absolute bottom-full left-0 z-40 mb-2 w-40 rounded-2xl p-1.5 shadow-lg"
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
          </div>

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
            className="block min-h-[22px] flex-1 bg-transparent border-none py-1 text-sm outline-none resize-none"
            style={{
              color: 'var(--cp-text)',
              lineHeight: '1.5',
              overflow: 'hidden',
            }}
          />
          <button
            onClick={handleSend}
            disabled={!hasDraft}
            className="p-1.5 rounded-full flex-shrink-0 transition-colors"
            style={{
              background: hasDraft
                ? 'var(--cp-accent)'
                : 'color-mix(in srgb, var(--cp-text) 10%, transparent)',
              color: hasDraft ? '#fff' : 'var(--cp-muted)',
            }}
            type="button"
          >
            <Send size={16} />
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

export const ConversationComposer = memo(ConversationComposerInner)

const MemoAttachmentCard = memo(function AttachmentCard({
  attachment,
  onRemove,
}: {
  attachment: ComposerAttachmentItem
  onRemove: (id: string) => void
}) {
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
        className="absolute right-1.5 top-1.5 z-10 rounded-full p-1"
        style={{
          background: 'rgba(15, 23, 42, 0.72)',
          color: '#fff',
        }}
        onClick={() => onRemove(attachment.id)}
        type="button"
      >
        <X size={12} />
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
    const caret = selectionStart + nextText.length
    textarea.focus()
    textarea.setSelectionRange(caret, caret)
  })
}
