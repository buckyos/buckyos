import { zodResolver } from '@hookform/resolvers/zod'
import { useMemo, useState } from 'react'
import { Controller, useForm } from 'react-hook-form'
import type { WindowDialogControls } from '../../desktop/windows/dialogs'
import { useI18n } from '../../i18n/provider'
import { isNarrowerAudience, type Translate } from './datamodel/format'
import { audienceSchema, commentInputSchema, type AudienceInput } from './datamodel/inputs'
import type { AudienceSpec } from './datamodel/types'
import { AudiencePicker } from './publish/AudiencePicker'
import { EmojiPickerButton } from './ui/EmojiPickerButton'

function DialogButtons({ onCancel, confirmLabel, onConfirm, danger, disabled, testId }: { onCancel: () => void; confirmLabel: string; onConfirm?: () => void; danger?: boolean; disabled?: boolean; testId?: string }) {
  const { t } = useI18n()
  return (
    <div className="mt-6 flex justify-end gap-2">
      <button type="button" className="hs-btn" onClick={onCancel}>{t('common.cancel', 'Cancel')}</button>
      <button type={onConfirm ? 'button' : 'submit'} data-testid={testId} className={danger ? 'hs-btn is-primary is-danger' : 'hs-btn is-primary'} disabled={disabled} onClick={onConfirm}>
        {confirmLabel}
      </button>
    </div>
  )
}

export function ConfirmBody({ controls, body, notes, confirmLabel, danger, testId }: { controls: WindowDialogControls<boolean>; body: string; notes?: string[]; confirmLabel: string; danger?: boolean; testId?: string }) {
  return (
    <div className="hs-root text-[15px] leading-[22px]">
      <p>{body}</p>
      {notes?.length ? (
        <ul className="mt-3 list-disc space-y-1 pl-5 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>
          {notes.map(note => <li key={note}>{note}</li>)}
        </ul>
      ) : null}
      <DialogButtons onCancel={() => controls.close(false)} onConfirm={() => controls.close(true)} confirmLabel={confirmLabel} danger={danger} testId={testId} />
    </div>
  )
}

export function QuoteBody({ controls, defaultAudience }: { controls: WindowDialogControls<{ text: string; audience: AudienceSpec }>; defaultAudience: AudienceSpec }) {
  const { t } = useI18n()
  const schema = useMemo(() => commentInputSchema.extend({ audience: audienceSchema }), [])
  const form = useForm<{ text: string; audience: AudienceInput }>({ resolver: zodResolver(schema), defaultValues: { text: '', audience: defaultAudience } })
  const textField = form.register('text')
  const error = form.formState.errors.text?.message
  return (
    <form className="hs-root" onSubmit={form.handleSubmit(values => controls.close(values))}>
      <textarea
        {...textField}
        autoFocus
        rows={4}
        className="hs-input resize-none"
        aria-label={t('homestation.quote.placeholder', 'Add your thoughts…')}
        placeholder={t('homestation.quote.placeholder', 'Add your thoughts…')}
      />
      <EmojiPickerButton field={textField} testId="hs-quote-emoji" />
      {error ? <p className="mt-2 text-[13px]" role="alert" style={{ color: 'var(--hs-danger-ink)' }}>{t(error, 'Write something first.')}</p> : null}
      <div className="mt-3">
        <Controller control={form.control} name="audience" render={({ field }) => <AudiencePicker idPrefix="hs-quote" value={field.value} onChange={field.onChange} />} />
      </div>
      <p className="mt-3 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>
        {t('homestation.quote.note', 'A quote is your own post that wraps this exact version. Edits to the original don’t change what you quoted.')}
      </p>
      <DialogButtons onCancel={() => controls.dismiss()} confirmLabel={t('homestation.quote.submit', 'Publish quote')} testId="hs-quote-submit" />
    </form>
  )
}

export function EditBody({ controls, initialText }: { controls: WindowDialogControls<string>; initialText: string }) {
  const { t } = useI18n()
  const form = useForm<{ text: string }>({ resolver: zodResolver(commentInputSchema), defaultValues: { text: initialText } })
  const textField = form.register('text')
  const error = form.formState.errors.text?.message
  return (
    <form className="hs-root" onSubmit={form.handleSubmit(values => controls.close(values.text))}>
      <textarea {...textField} autoFocus rows={5} className="hs-input resize-none" aria-label={t('homestation.edit.label', 'Post text')} />
      <EmojiPickerButton field={textField} testId="hs-edit-emoji" />
      {error ? <p className="mt-2 text-[13px]" role="alert" style={{ color: 'var(--hs-danger-ink)' }}>{t(error, 'Write something first.')}</p> : null}
      <p className="mt-3 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>
        {t('homestation.edit.note', 'Saving publishes a new version with a new object ID and moves your entry to it. Comments and likes stay on the version they were made on, and copies of the old version that others already have can’t be recalled.')}
      </p>
      <DialogButtons onCancel={() => controls.dismiss()} confirmLabel={t('homestation.edit.submit', 'Publish new version')} testId="hs-edit-submit" />
    </form>
  )
}

export function AudienceBody({ controls, current, t, groupsLabel }: { controls: WindowDialogControls<AudienceSpec>; current: AudienceSpec; t: Translate; groupsLabel: (spec: AudienceSpec) => string }) {
  const [value, setValue] = useState<AudienceSpec>(current)
  const narrower = isNarrowerAudience(value, current)
  const valid = audienceSchema.safeParse(value).success
  return (
    <div className="hs-root">
      <p className="mb-3 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.audience.current', 'Current: {{label}}', { label: groupsLabel(current) })}</p>
      <AudiencePicker idPrefix="hs-audience-dialog" value={value} onChange={setValue} />
      {narrower ? (
        <p className="hs-note is-warning mt-3" role="note" data-testid="hs-audience-shrink">
          {t('homestation.audience.shrinkNote', 'Narrowing the audience doesn’t take back copies people already fetched. If they should stop seeing it, withdraw the post instead.')}
        </p>
      ) : (
        <p className="mt-3 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.audience.widenNote', 'People who gain access will see this post the next time they sync your feed. The post itself doesn’t change.')}
        </p>
      )}
      <DialogButtons onCancel={() => controls.dismiss()} onConfirm={() => controls.close(value)} disabled={!valid} confirmLabel={t('homestation.audience.apply', 'Apply')} testId="hs-audience-apply" />
    </div>
  )
}
