/* ── Forms of user Collections (= user tags): name a new one, rename, choose one ── */

import { zodResolver } from '@hookform/resolvers/zod'
import { Button } from '@mui/material'
import { FolderOpen } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { z } from 'zod'
import { useI18n } from '../../../i18n/provider'
import { userTagSchema } from '../../../components/network/relation'

export function CollectionNameForm({
  initial = '',
  existing,
  submitLabel,
  hint,
  onSubmit,
  onCancel,
}: {
  initial?: string
  existing: string[]
  submitLabel: string
  hint?: string
  onSubmit: (name: string) => void
  onCancel: () => void
}) {
  const { t } = useI18n()
  const schema = z.object({
    name: userTagSchema.refine(value => !existing.includes(value), 'myNetwork.collection.error.exists'),
  })
  const form = useForm<z.infer<typeof schema>>({ resolver: zodResolver(schema), defaultValues: { name: initial } })
  const error = form.formState.errors.name?.message
  return (
    <form className="space-y-3" onSubmit={form.handleSubmit(values => onSubmit(values.name))} data-testid="collection-name-form">
      <label className="block text-sm text-[color:var(--cp-text)]">
        {t('myNetwork.collection.name')}
        <input
          {...form.register('name')}
          autoFocus
          maxLength={80}
          aria-invalid={Boolean(error)}
          className="mt-1 min-h-10 w-full rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-bg)] px-3 text-sm text-[color:var(--cp-text)]"
        />
      </label>
      {error ? <p role="alert" className="text-xs text-[color:var(--cp-danger)]">{t(error)}</p> : null}
      {hint ? <p className="text-xs text-[color:var(--cp-muted)]">{hint}</p> : null}
      <div className="flex justify-end gap-2">
        <Button variant="text" onClick={onCancel}>{t('network.cancel')}</Button>
        <Button type="submit" variant="contained">{submitLabel}</Button>
      </div>
    </form>
  )
}

export function CollectionChooser({ collections, onPick, onCancel }: { collections: string[]; onPick: (tag: string) => void; onCancel: () => void }) {
  const { t } = useI18n()
  return (
    <div className="space-y-3" data-testid="collection-chooser">
      {collections.length ? (
        <div className="space-y-1">
          {collections.map(tag => (
            <button key={tag} type="button" onClick={() => onPick(tag)} className="flex min-h-10 w-full items-center gap-2 rounded-xl border border-[color:var(--cp-border)] px-3 text-left text-sm text-[color:var(--cp-text)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,var(--cp-surface))]">
              <FolderOpen size={14} className="text-[color:var(--cp-accent)]" />{tag}
            </button>
          ))}
        </div>
      ) : null}
      <div className="border-t border-[color:color-mix(in_srgb,var(--cp-border)_74%,transparent)] pt-3">
        <CollectionNameForm existing={collections} submitLabel={t('myNetwork.collection.createAndAdd')} onSubmit={onPick} onCancel={onCancel} />
      </div>
    </div>
  )
}
