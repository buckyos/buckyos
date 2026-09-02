/**
 * Single-field name/title dialog driven by react-hook-form + the Zod input
 * schemas (UI_DATAMODEL.md §3) — replaces the prototype's window.prompt()
 * paths for Collection/group creation and rename flows.
 */

import { zodResolver } from '@hookform/resolvers/zod'
import { useEffect } from 'react'
import { useForm } from 'react-hook-form'
import { z } from 'zod'
import { useI18n } from '../../../i18n/provider'
import { validationFallback } from '../data/schemas'

export interface NamePromptRequest {
  /** Dialog heading, already localized. */
  title: string
  /** Field label/placeholder, already localized. */
  label: string
  /** Submit button text, already localized. */
  submitLabel: string
  /** Edit-state refill: the current title/name exactly as displayed. */
  defaultValue?: string
  /** Field-level schema (entryNameSchema / collectionTitleSchema / …). */
  schema: z.ZodType<string, string>
  onSubmit: (value: string) => void | Promise<void>
}

interface FormValues {
  value: string
}

export function NamePromptDialog({
  request,
  onClose,
}: {
  request: NamePromptRequest | null
  onClose: () => void
}) {
  const { t } = useI18n()

  const formSchema = request ? z.object({ value: request.schema }) : z.object({ value: z.string() })
  const {
    register,
    handleSubmit,
    reset,
    setError,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({
    resolver: zodResolver(formSchema),
    defaultValues: { value: request?.defaultValue ?? '' },
  })

  useEffect(() => {
    reset({ value: request?.defaultValue ?? '' })
  }, [request, reset])

  if (!request) return null

  const submit = handleSubmit(async ({ value }) => {
    try {
      await request.onSubmit(value.trim())
      onClose()
    } catch (err) {
      setError('root', {
        message: err instanceof Error ? err.message : String(err),
      })
    }
  })

  const fieldError = errors.value?.message
  const rootError = errors.root?.message

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/40" onClick={onClose} />
      <form
        onSubmit={(event) => void submit(event)}
        className="relative w-[min(92vw,380px)] rounded-[20px] border border-[color:var(--cp-border)] p-4 shadow-[0_18px_48px_rgba(0,0,0,0.28)]"
        style={{ background: 'var(--cp-surface)' }}
        data-testid="name-prompt-dialog"
      >
        <div className="mb-3 text-[15px] font-semibold text-[color:var(--cp-text)]">
          {request.title}
        </div>
        <label className="block">
          <span className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-[color:var(--cp-muted)]">
            {request.label}
          </span>
          <input
            type="text"
            autoFocus
            {...register('value')}
            className="w-full rounded-[12px] border border-[color:var(--cp-border)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] px-3 py-2 text-sm outline-none focus:border-[color:var(--cp-accent)]"
            style={{ color: 'var(--cp-text)' }}
          />
        </label>
        {fieldError ? (
          <p className="mt-1.5 text-[11px] text-[color:var(--cp-warning)]" role="alert">
            {t(fieldError, validationFallback[fieldError] ?? fieldError)}
          </p>
        ) : null}
        {rootError ? (
          <p className="mt-1.5 text-[11px] text-[color:var(--cp-warning)]" role="alert">
            {rootError}
          </p>
        ) : null}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-full border border-[color:var(--cp-border)] px-4 py-1.5 text-sm text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            type="submit"
            disabled={isSubmitting}
            className="rounded-full px-4 py-1.5 text-sm font-semibold text-white disabled:opacity-60"
            style={{ background: 'var(--cp-accent)' }}
          >
            {request.submitLabel}
          </button>
        </div>
      </form>
    </div>
  )
}
