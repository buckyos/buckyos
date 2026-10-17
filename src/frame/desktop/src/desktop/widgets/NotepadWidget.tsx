import { zodResolver } from '@hookform/resolvers/zod'
import { Button } from '@mui/material'
import clsx from 'clsx'
import { useEffect, useId, useRef, useState } from 'react'
import { useForm, useWatch } from 'react-hook-form'
import { useI18n } from '../../i18n/provider'
import { NOTE_MAX_LENGTH, noteInputSchema, type NoteInput } from '../../models/ui'
import type { DesktopWidgetProps } from './types'

export function NotepadWidget({ item, onSaveNote }: DesktopWidgetProps) {
  const { t } = useI18n()
  const [isEditing, setIsEditing] = useState(false)
  const contentId = useId()
  const textareaRef = useRef<HTMLTextAreaElement | null>(null)
  // The `.widget` file's `text` (models/desktopFolder.ts).
  const value = String(item.config.text ?? '')
  const form = useForm<NoteInput>({
    resolver: zodResolver(noteInputSchema),
    defaultValues: {
      content: value,
    },
  })
  const { ref: registerTextareaRef, ...textareaField } = form.register('content')
  const noteValue =
    useWatch({
      control: form.control,
      name: 'content',
    }) ?? ''
  const trimmedLength = noteValue.trim().length
  const remaining = NOTE_MAX_LENGTH - trimmedLength
  const previewContent = noteValue.trim()

  useEffect(() => {
    form.reset({ content: value })
  }, [form, value])

  useEffect(() => {
    if (!isEditing) {
      return
    }

    textareaRef.current?.focus()
  }, [isEditing])

  if (!isEditing) {
    return (
      <button
        type="button"
        data-testid={`notepad-preview-${item.id}`}
        // A short name; the note itself is the description (it mentions app names, which other buttons carry).
        aria-label={t('widgets.editNote', 'Edit note')}
        aria-describedby={contentId}
        onClick={() => setIsEditing(true)}
        className="flex h-full w-full flex-col rounded-[22px] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface-3)_86%,transparent),color-mix(in_srgb,var(--cp-surface-2)_96%,transparent))] p-4 text-left"
      >
        <div className="min-h-0 flex-1 overflow-hidden">
          <p
            id={contentId}
            className={clsx(
              'whitespace-pre-line text-[13px] leading-5',
              previewContent
                ? 'text-[color:var(--cp-text)]'
                : 'text-[color:var(--cp-muted)]',
            )}
          >
            {previewContent || t('widgets.notesPlaceholder')}
          </p>
        </div>
      </button>
    )
  }

  return (
    <form
      onSubmit={form.handleSubmit((values) => {
        if (values.content !== value) {
          onSaveNote(item.id, values.content)
        }
        form.reset({ content: values.content })
        setIsEditing(false)
      })}
      className="flex h-full flex-col rounded-[22px] bg-[linear-gradient(180deg,color-mix(in_srgb,var(--cp-surface-3)_86%,transparent),color-mix(in_srgb,var(--cp-surface-2)_96%,transparent))] p-4"
    >
      <textarea
        {...textareaField}
        ref={(node) => {
          registerTextareaRef(node)
          textareaRef.current = node
        }}
        data-testid={`notepad-editor-${item.id}`}
        aria-invalid={form.formState.isSubmitted && !form.formState.isValid}
        className="widget-interactive min-h-0 flex-1 resize-none rounded-[18px] border border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface)_96%,transparent)] p-3 text-sm leading-6 text-[color:var(--cp-text)] shadow-[inset_0_1px_0_color-mix(in_srgb,white_35%,transparent)] outline-none placeholder:text-[color:var(--cp-muted)] focus:border-[color:var(--cp-accent)]"
        placeholder={t('widgets.notesPlaceholder')}
      />
      <div className="mt-3 flex items-center justify-between gap-3">
        <span className="text-[11px] font-medium text-[color:var(--cp-muted)]">
          {Math.max(remaining, 0)}/{NOTE_MAX_LENGTH}
        </span>
        <Button
          type="submit"
          variant="contained"
          size="small"
          data-testid={`notepad-save-${item.id}`}
          className="widget-interactive"
          disabled={!form.formState.isValid}
        >
          {t('common.save')}
        </Button>
      </div>
    </form>
  )
}
