import { zodResolver } from '@hookform/resolvers/zod'
import clsx from 'clsx'
import { CircleAlert, Globe2, Image, Link2, Loader2, Mic, RotateCw, Send, Video, X } from 'lucide-react'
import { useCallback, useRef, useState } from 'react'
import { Controller, useForm, useWatch } from 'react-hook-form'
import { useI18n } from '../../../i18n/provider'
import { linkCardSchema, publishInputSchema, type AttachmentInput, type PublishInput } from '../datamodel/inputs'
import type { HomeStationStore } from '../store/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useStoreSelector } from '../store/context'
import { useToast } from '../ui/toastContext'
import { AudiencePicker } from './AudiencePicker'

const selectSettings = (store: HomeStationStore) => store.peekSettings()
const selectHome = (store: HomeStationStore) => store.peekHome()

function newPublishKey() {
  return `publish-${crypto.randomUUID()}`
}

function kindOfFile(file: File): AttachmentInput['kind'] {
  if (file.type.startsWith('video/')) return 'video'
  if (file.type.startsWith('audio/')) return 'audio'
  return 'image'
}

export function PublishComposer({ variant = 'full', onPublished }: { variant?: 'full' | 'compact'; onPublished?: () => void }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const toast = useToast()
  const nav = useHsNav()
  const settings = useStoreSelector(selectSettings)
  const home = useStoreSelector(selectHome)
  const [publishKey, setPublishKey] = useState(newPublishKey)
  const [failed, setFailed] = useState(false)
  const [linkOpen, setLinkOpen] = useState(false)
  const [linkUrl, setLinkUrl] = useState('')
  const [linkBusy, setLinkBusy] = useState(false)
  const [linkError, setLinkError] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const compact = variant === 'compact'
  const idPrefix = compact ? 'hs-quick' : 'hs-publish'

  const form = useForm<PublishInput>({
    resolver: zodResolver(publishInputSchema),
    defaultValues: { text: '', attachments: [], link: null, audience: settings.defaultAudience },
  })
  const attachments = useWatch({ control: form.control, name: 'attachments' })
  const link = useWatch({ control: form.control, name: 'link' })
  const text = useWatch({ control: form.control, name: 'text' })
  const audience = useWatch({ control: form.control, name: 'audience' })
  const uploading = attachments.some(attachment => attachment.status === 'uploading')

  const updateAttachment = useCallback((id: string, patch: Partial<AttachmentInput>) => {
    form.setValue('attachments', form.getValues('attachments').map(attachment => (attachment.id === id ? { ...attachment, ...patch } : attachment)), { shouldValidate: form.formState.isSubmitted })
  }, [form])

  const addAttachment = useCallback(async (kind: AttachmentInput['kind'], file: File) => {
    const id = crypto.randomUUID()
    form.setValue('attachments', [...form.getValues('attachments'), { id, kind, name: file.name, status: 'uploading' }])
    try {
      const object = await store.uploadAttachment(file, kind)
      updateAttachment(id, { status: 'uploaded', object })
    } catch {
      updateAttachment(id, { status: 'failed' })
    }
  }, [form, store, updateAttachment])

  const fetchLink = async () => {
    const parsed = linkCardSchema.shape.url.safeParse(linkUrl)
    if (!parsed.success) {
      setLinkError(t('homestation.validation.url', 'Enter a full http(s) URL.'))
      return
    }
    setLinkError(null)
    setLinkBusy(true)
    const preview = await store.fetchLinkPreview(parsed.data)
    setLinkBusy(false)
    form.setValue('link', { url: parsed.data, title: preview.title, summary: preview.summary }, { shouldValidate: form.formState.isSubmitted })
    setLinkOpen(false)
  }

  const finish = () => {
    setFailed(false)
    form.reset({ text: '', attachments: [], link: null, audience: form.getValues('audience'), zoneFeed: form.getValues('zoneFeed') })
    setPublishKey(newPublishKey())
    toast({ text: t('homestation.publish.done', 'Published to your feed. It’s in My publications, not in your reading list.'), tone: 'success', action: { label: t('homestation.toast.viewPublished', 'View'), onClick: () => nav.navigate({ name: 'published' }, { reset: true }) } })
    onPublished?.()
  }

  const submit = form.handleSubmit(async values => {
    const task = await store.publish({ ...values, zoneFeed: values.audience.kind === 'public' && !!values.zoneFeed }, publishKey)
    if (task.stage === 'failed') {
      setFailed(true)
      return
    }
    finish()
  })

  const retry = async () => {
    const task = await store.retryPublish(publishKey)
    if (task.stage === 'failed') return
    finish()
  }

  const errors = form.formState.errors
  const errorKey = errors.text?.message ?? errors.attachments?.message ?? errors.attachments?.root?.message ?? errors.audience?.message ?? (errors.audience as { groupId?: { message?: string }; dids?: { message?: string } } | undefined)?.dids?.message
  const busy = form.formState.isSubmitting

  return (
    <form className={clsx(compact ? '' : 'p-4')} onSubmit={submit} data-testid={`${idPrefix}-form`}>
      <div className="rounded-2xl p-3" style={{ background: 'var(--hs-subtle-bg)' }}>
        <textarea
          {...form.register('text')}
          rows={compact ? 3 : 5}
          className="w-full resize-none bg-transparent text-sm outline-none"
          style={{ minHeight: compact ? 64 : 120 }}
          placeholder={t('homestation.publish.placeholder', 'What’s on your mind?')}
          aria-label={t('homestation.publish.placeholder', 'What’s on your mind?')}
          data-testid={`${idPrefix}-text`}
        />
        {attachments.length > 0 ? (
          <ul className="mb-2 flex flex-wrap gap-1.5" data-testid={`${idPrefix}-attachments`}>
            {attachments.map(attachment => (
              <li key={attachment.id} className="hs-badge" data-status={attachment.status}>
                {attachment.status === 'uploading' ? <Loader2 size={11} className="animate-spin" /> : attachment.status === 'failed' ? <CircleAlert size={11} /> : null}
                {attachment.name}
                {attachment.status === 'uploading' ? ` · ${t('homestation.publish.uploadingShort', 'uploading')}` : attachment.status === 'failed' ? ` · ${t('homestation.publish.uploadFailed', 'upload failed')}` : ''}
                <button type="button" aria-label={t('homestation.publish.removeAttachment', 'Remove')} onClick={() => form.setValue('attachments', form.getValues('attachments').filter(entry => entry.id !== attachment.id))}><X size={11} /></button>
              </li>
            ))}
          </ul>
        ) : null}
        {link ? (
          <div className="mb-2 flex items-start gap-2 rounded-xl border px-3 py-2 text-xs" style={{ borderColor: 'var(--hs-divider)' }} data-testid={`${idPrefix}-link`}>
            <Link2 size={13} className="mt-0.5" />
            <div className="min-w-0 flex-1">
              <input className="w-full bg-transparent text-sm font-semibold outline-none" value={link.title} aria-label={t('homestation.publish.linkTitle', 'Card title')} onChange={event => form.setValue('link', { ...link, title: event.target.value })} />
              <p className="truncate" style={{ color: 'var(--cp-muted)' }}>{link.url}</p>
              <p style={{ color: 'var(--cp-muted)' }}>{t('homestation.publish.linkNote', 'The title and summary are fixed in your post; the URL is only a jump target.')}</p>
            </div>
            <button type="button" aria-label={t('homestation.publish.removeAttachment', 'Remove')} onClick={() => form.setValue('link', null)}><X size={13} /></button>
          </div>
        ) : null}
        {linkOpen ? (
          <div className="mb-2 flex items-center gap-2">
            <input className="hs-input py-1.5 text-sm" value={linkUrl} placeholder="https://" aria-label={t('homestation.publish.linkUrl', 'Link URL')} onChange={event => setLinkUrl(event.target.value)} data-testid={`${idPrefix}-link-url`} />
            <button type="button" className="hs-btn" disabled={linkBusy} onClick={() => void fetchLink()}>{linkBusy ? <Loader2 size={13} className="animate-spin" /> : null}{t('homestation.publish.linkFetch', 'Make card')}</button>
          </div>
        ) : null}
        {linkError ? <p className="mb-2 text-xs" style={{ color: 'var(--cp-danger)' }}>{linkError}</p> : null}
        <div className="flex flex-wrap items-center gap-1 pt-2" style={{ borderTop: '1px solid var(--hs-divider)' }}>
          <input
            ref={fileRef}
            type="file"
            accept="image/*,video/*,audio/*"
            multiple
            className="hidden"
            data-testid={`${idPrefix}-file`}
            onChange={event => {
              for (const file of [...(event.target.files ?? [])]) void addAttachment(kindOfFile(file), file)
              event.target.value = ''
            }}
          />
          <button type="button" className="hs-icon-btn" title={t('homestation.publish.addMedia', 'Add photos or video')} aria-label={t('homestation.publish.addMedia', 'Add photos or video')} onClick={() => fileRef.current?.click()}>
            <Image size={17} />
          </button>
          <button type="button" className="hs-icon-btn" title={t('homestation.publish.addVideo', 'Add video')} aria-label={t('homestation.publish.addVideo', 'Add video')} onClick={() => fileRef.current?.click()}>
            <Video size={17} />
          </button>
          <button type="button" className="hs-icon-btn" title={t('homestation.publish.addVoice', 'Record a voice note (simulated)')} aria-label={t('homestation.publish.addVoice', 'Record a voice note (simulated)')} onClick={() => void addAttachment('audio', new File([new Uint8Array(380_000)], `voice-note-${attachments.length + 1}.m4a`, { type: 'audio/mp4' }))}>
            <Mic size={17} />
          </button>
          <button type="button" className="hs-icon-btn" aria-pressed={linkOpen} title={t('homestation.publish.addLink', 'Import from a URL')} aria-label={t('homestation.publish.addLink', 'Import from a URL')} onClick={() => setLinkOpen(value => !value)}>
            <Link2 size={17} />
          </button>
          <span className="ml-auto text-xs tabular-nums" style={{ color: 'var(--cp-muted)' }}>{text.length}</span>
          <button type="submit" className="hs-btn is-primary" disabled={busy || uploading || failed} data-testid={`${idPrefix}-submit`}>
            {busy ? <Loader2 size={14} className="animate-spin" /> : <Send size={14} />}
            {busy ? t('homestation.publish.publishing', 'Publishing…') : t('homestation.publish.submit', 'Publish')}
          </button>
        </div>
      </div>
      <div className="mt-3">
        <Controller control={form.control} name="audience" render={({ field }) => <AudiencePicker idPrefix={idPrefix} value={field.value} onChange={field.onChange} compact={compact} />} />
      </div>
      {home?.zoneFeed.writer ? (
        <Controller
          control={form.control}
          name="zoneFeed"
          render={({ field }) => (
            <label className="mt-2 flex items-center gap-2 text-xs" style={{ color: audience.kind === 'public' ? undefined : 'var(--cp-muted)' }} data-testid={`${idPrefix}-zone-feed`}>
              <input type="checkbox" checked={audience.kind === 'public' && !!field.value} disabled={audience.kind !== 'public'} onChange={event => field.onChange(event.target.checked)} />
              <Globe2 size={13} />
              <span>
                {t('homestation.publish.zoneFeed', 'Also show on {{zone}}', { zone: home.zoneFeed.name })}
                {audience.kind !== 'public' ? ` · ${t('homestation.publish.zoneFeedPublicOnly', 'public posts only')}` : ''}
              </span>
            </label>
          )}
        />
      ) : null}
      {errorKey ? <p className="mt-2 text-xs" role="alert" style={{ color: 'var(--cp-danger)' }}>{t(errorKey, 'Check the post before publishing.')}</p> : null}
      {failed ? (
        <div className="mt-3 flex flex-wrap items-center gap-2 rounded-xl px-3 py-2 text-xs" role="alert" style={{ background: 'color-mix(in srgb, var(--cp-danger) 12%, transparent)' }} data-testid={`${idPrefix}-failed`}>
          <CircleAlert size={14} />
          <span className="flex-1">{t('homestation.publish.failedBody', 'Not published: your HomeStation couldn’t store the post. Nothing was sent. Retrying reuses the same publish ID, so it won’t create a duplicate.')}</span>
          <button type="button" className="hs-btn" onClick={() => void retry()} data-testid={`${idPrefix}-retry`}><RotateCw size={13} />{t('common.retry', 'Retry')}</button>
        </div>
      ) : null}
      {!compact ? (
        <p className="mt-3 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>
          {t('homestation.publish.hint', 'Your post is signed by you and added to your home feed for its audience. It isn’t delivered to your own reading list.')}
        </p>
      ) : null}
    </form>
  )
}
