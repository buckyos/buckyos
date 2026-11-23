import { attachmentKindOfFile } from '../api/fileKind'
import { releaseAttachmentImport } from '../api/transport'
import { zodResolver } from '@hookform/resolvers/zod'
import { CircleAlert, FileUp, FolderUp, Image, Link2, Loader2, Mic, RotateCw, Video, X } from 'lucide-react'
import { type FormEvent, useCallback, useContext, useEffect, useRef, useState } from 'react'
import { Controller, useForm, useWatch } from 'react-hook-form'
import { useI18n } from '../../../i18n/provider'
import { linkCardSchema, publishInputSchema, type AttachmentInput, type PublishInput } from '../datamodel/inputs'
import type { HomeStationStore } from '../store/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useStoreSelector } from '../store/context'
import { EmojiPickerButton } from '../ui/EmojiPickerButton'
import { useToast } from '../ui/toastContext'
import { Avatar } from '../ui/primitives'
import { AudiencePicker } from './AudiencePicker'
import { reportShareDelivery } from '../../../api/shares'
import type { PublishTask } from '../datamodel/types'
import { ComposePrefillContext } from '../launch'

const selectSettings = (store: HomeStationStore) => store.peekSettings()
const selectHome = (store: HomeStationStore) => store.peekHome()
const selectOwner = (store: HomeStationStore) => store.peekIdentity(store.owner)

function newPublishKey() {
  return `publish-${crypto.randomUUID()}`
}

export function PublishComposer({ variant = 'full', onPublished }: { variant?: 'full' | 'compact'; onPublished?: () => void }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const toast = useToast()
  const nav = useHsNav()
  const settings = useStoreSelector(selectSettings)
  const home = useStoreSelector(selectHome)
  const owner = useStoreSelector(selectOwner)
  const [publishKey, setPublishKey] = useState(newPublishKey)
  const [failed, setFailed] = useState(false)
  const [linkOpen, setLinkOpen] = useState(false)
  const [linkUrl, setLinkUrl] = useState('')
  const [linkBusy, setLinkBusy] = useState(false)
  const [linkError, setLinkError] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const genericRef = useRef<HTMLInputElement>(null)
  const folderRef = useRef<HTMLInputElement>(null)
  const uploads = useRef(new Map<string, { file: File | readonly File[]; kind: AttachmentInput['kind']; controller: AbortController }>())
  useEffect(() => { const jobs = uploads.current; return () => jobs.forEach(job => { job.controller.abort(); releaseAttachmentImport(job.file) }) }, [])
  const compact = variant === 'compact'
  const idPrefix = compact ? 'hs-quick' : 'hs-publish'

  // A post handed over by the system share dialog pre-fills the full composer once.
  const prefillSource = useContext(ComposePrefillContext)
  const [prefill] = useState(() => (compact ? null : prefillSource.peek()))
  const form = useForm<PublishInput>({
    resolver: zodResolver(publishInputSchema),
    defaultValues: {
      text: prefill?.text ?? '',
      attachments: (prefill?.attachments ?? []).map(attachment => ({ ...attachment, id: crypto.randomUUID() })),
      link: prefill?.link ? { url: prefill.link.url, title: prefill.link.title, summary: prefill.link.summary ?? '' } : null,
      audience: settings.defaultAudience,
    },
  })
  const [sharedId, setSharedId] = useState(prefill?.shareId ?? null)
  const textField = form.register('text')
  const attachments = useWatch({ control: form.control, name: 'attachments' })
  const link = useWatch({ control: form.control, name: 'link' })
  const text = useWatch({ control: form.control, name: 'text' })
  const audience = useWatch({ control: form.control, name: 'audience' })
  const uploading = attachments.some(attachment => attachment.status === 'uploading')

  const updateAttachment = useCallback((id: string, patch: Partial<AttachmentInput>) => {
    form.setValue('attachments', form.getValues('attachments').map(attachment => (attachment.id === id ? { ...attachment, ...patch } : attachment)), { shouldValidate: form.formState.isSubmitted })
  }, [form])

  const runUpload = useCallback(async (id: string, kind: AttachmentInput['kind'], file: File | readonly File[]) => {
    const controller = new AbortController()
    uploads.current.set(id, { file, kind, controller })
    updateAttachment(id, { status: 'uploading', error: undefined, progress: 0 })
    try {
      const object = await store.uploadAttachment(file, kind, { signal: controller.signal, onProgress: (loaded, total) => updateAttachment(id, { progress: total ? Math.min(99, Math.round(loaded / total * 100)) : 0 }) })
      updateAttachment(id, { status: 'uploaded', object, progress: 100 })
    } catch (error) {
      updateAttachment(id, { status: controller.signal.aborted ? 'cancelled' : 'failed', error: error instanceof Error ? error.message : String(error) })
    }
  }, [store, updateAttachment])

  const addAttachment = useCallback(async (kind: AttachmentInput['kind'], file: File | readonly File[]) => {
    const id = crypto.randomUUID()
    const name = Array.isArray(file) ? file[0]?.webkitRelativePath.split('/')[0] || 'folder' : (file as File).name
    form.setValue('attachments', [...form.getValues('attachments'), { id, kind, name, status: 'uploading' }])
    await runUpload(id, kind, file)
  }, [form, runUpload])

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

  const finish = (task?: PublishTask) => {
    if (sharedId) {
      reportShareDelivery(sharedId, 'confirmed', { businessRef: { entry: task?.entry ?? null, obj_id: task?.objId ?? null } })
      setSharedId(null)
      prefillSource.clear()
    }
    uploads.current.forEach(job => releaseAttachmentImport(job.file))
    uploads.current.clear()
    setFailed(false)
    form.reset({ text: '', attachments: [], link: null, audience: form.getValues('audience'), zoneFeed: form.getValues('zoneFeed') })
    setPublishKey(newPublishKey())
    toast({ text: t('homestation.publish.done', 'Published to your feed. It’s in My publications, not in your reading list.'), tone: 'success', action: { label: t('homestation.toast.viewPublished', 'View'), onClick: () => nav.navigate({ name: 'published' }, { reset: true }) } })
    onPublished?.()
  }

  const publish = async (values: PublishInput) => {
    const task = await store.publish({ ...values, zoneFeed: values.audience.kind === 'public' && !!values.zoneFeed }, publishKey)
    if (task.stage === 'failed') {
      setFailed(true)
      return
    }
    finish(task)
  }
  const submit = (event: FormEvent<HTMLFormElement>) => form.handleSubmit(publish)(event)

  const retry = async () => {
    const task = await store.retryPublish(publishKey)
    if (task.stage === 'failed') return
    finish(task)
  }

  const errors = form.formState.errors
  const errorKey = errors.text?.message ?? errors.attachments?.message ?? errors.attachments?.root?.message ?? errors.audience?.message ?? (errors.audience as { groupId?: { message?: string }; dids?: { message?: string } } | undefined)?.dids?.message
  const busy = form.formState.isSubmitting

  const fileInputs = (
    <>
      <input
        ref={fileRef}
        type="file"
        accept="image/*,video/*,audio/*,.heic,.heif,.mov"
        multiple
        className="hidden"
        data-testid={`${idPrefix}-file`}
        onChange={event => {
          for (const file of [...(event.target.files ?? [])]) void attachmentKindOfFile(file).then(kind => addAttachment(kind, file))
          event.target.value = ''
        }}
      />
      <input ref={genericRef} type="file" multiple className="hidden" data-testid={`${idPrefix}-generic-file`} onChange={event => { for (const file of Array.from(event.target.files ?? [])) void addAttachment('file', file); event.target.value = '' }} />
      <input ref={element => { folderRef.current = element; element?.setAttribute('webkitdirectory', '') }} type="file" multiple className="hidden" data-testid={`${idPrefix}-folder`} onChange={event => { const files = Array.from(event.target.files ?? []); if (files.length) void addAttachment('dir', files); else toast({ text: t('homestation.publish.folderEmpty', 'The folder is empty or this browser can’t read it'), tone: 'warning' }); event.target.value = '' }} />
    </>
  )

  const tools = (
    <div className="-ml-2 flex flex-wrap items-center gap-1">
      <EmojiPickerButton field={textField} disabled={busy} testId={`${idPrefix}-emoji`} />
      <button type="button" className="hs-icon-btn" title={t('homestation.publish.addMedia', 'Add photos or video')} aria-label={t('homestation.publish.addMedia', 'Add photos or video')} onClick={() => fileRef.current?.click()}>
        <Image size={18} />
      </button>
      <button type="button" className="hs-icon-btn" title={t('homestation.publish.addVideo', 'Add video')} aria-label={t('homestation.publish.addVideo', 'Add video')} onClick={() => fileRef.current?.click()}>
        <Video size={18} />
      </button>
      <button type="button" className="hs-icon-btn" title={t('homestation.publish.addVoice', 'Record a voice note (simulated)')} aria-label={t('homestation.publish.addVoice', 'Record a voice note (simulated)')} onClick={() => void addAttachment('audio', new File([new Uint8Array(380_000)], `voice-note-${attachments.length + 1}.m4a`, { type: 'audio/mp4' }))}>
        <Mic size={18} />
      </button>
      <button type="button" className="hs-icon-btn" aria-pressed={linkOpen} title={t('homestation.publish.addLink', 'Import from a URL')} aria-label={t('homestation.publish.addLink', 'Import from a URL')} onClick={() => setLinkOpen(value => !value)}>
        <Link2 size={18} />
      </button>
      <button type="button" className="hs-icon-btn" title={t('homestation.publish.addFiles', 'Upload files')} aria-label={t('homestation.publish.addFiles', 'Upload files')} onClick={() => genericRef.current?.click()}><FileUp size={18} /></button>
      <button type="button" className="hs-icon-btn" title={t('homestation.publish.addFolder', 'Upload folder')} aria-label={t('homestation.publish.addFolder', 'Upload folder')} disabled={!('webkitdirectory' in document.createElement('input'))} onClick={() => folderRef.current?.click()}><FolderUp size={18} /></button>
    </div>
  )

  const extras = (
    <>
      {attachments.length > 0 ? (
        <ul className="mt-3 space-y-2" data-testid={`${idPrefix}-attachments`}>
          {attachments.map(attachment => (
            <li key={attachment.id} className="flex min-w-0 items-center gap-2 rounded-xl px-3 py-2 text-[13px] leading-5" style={{ border: '1px solid var(--hs-line-strong)' }} data-status={attachment.status}>
              {attachment.status === 'uploading' ? <Loader2 size={14} className="flex-shrink-0 animate-spin" style={{ color: 'var(--cp-muted)' }} /> : attachment.status === 'failed' ? <CircleAlert size={14} className="flex-shrink-0" style={{ color: 'var(--hs-danger-ink)' }} /> : null}
              <span className="min-w-0 flex-1 truncate">
                {attachment.name}
                <span style={{ color: 'var(--cp-muted)' }}>{attachment.status === 'uploading' ? ` · ${attachment.progress ?? 0}%` : attachment.error ? ` · ${attachment.error}` : ''}</span>
              </span>
              {attachment.status === 'uploading' ? <button type="button" className="hs-link" onClick={() => uploads.current.get(attachment.id)?.controller.abort()}>{t('common.cancel', 'Cancel')}</button> : null}
              {attachment.status === 'failed' || attachment.status === 'cancelled' ? <button type="button" className="hs-link" onClick={() => { const job = uploads.current.get(attachment.id); if (job) void runUpload(attachment.id, job.kind, job.file) }}><RotateCw size={14} />{t('common.retry', 'Retry')}</button> : null}
              <button type="button" className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-md hover:bg-[var(--hs-hover-bg)]" style={{ color: 'var(--cp-muted)' }} aria-label={t('homestation.publish.removeAttachment', 'Remove')} onClick={() => { const job = uploads.current.get(attachment.id); if (job) { job.controller.abort(); releaseAttachmentImport(job.file) } uploads.current.delete(attachment.id); form.setValue('attachments', form.getValues('attachments').filter(entry => entry.id !== attachment.id)) }}><X size={14} /></button>
            </li>
          ))}
        </ul>
      ) : null}
      {link ? (
        <div className="mt-3 flex items-start gap-3 rounded-xl px-4 py-3" style={{ border: '1px solid var(--hs-line-strong)' }} data-testid={`${idPrefix}-link`}>
          <Link2 size={16} className="mt-[3px] flex-shrink-0" style={{ color: 'var(--cp-muted)' }} />
          <div className="min-w-0 flex-1 text-[13px] leading-5">
            <input className="w-full bg-transparent text-[15px] font-semibold leading-[22px] outline-none" value={link.title} aria-label={t('homestation.publish.linkTitle', 'Card title')} onChange={event => form.setValue('link', { ...link, title: event.target.value })} />
            <p className="truncate" style={{ color: 'var(--cp-muted)' }}>{link.url}</p>
            <p className="mt-1" style={{ color: 'var(--cp-muted)' }}>{t('homestation.publish.linkNote', 'The title and summary are fixed in your post; the URL is only a jump target.')}</p>
          </div>
          <button type="button" className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-md hover:bg-[var(--hs-hover-bg)]" style={{ color: 'var(--cp-muted)' }} aria-label={t('homestation.publish.removeAttachment', 'Remove')} onClick={() => form.setValue('link', null)}><X size={14} /></button>
        </div>
      ) : null}
      {linkOpen ? (
        <div className="mt-3 flex items-center gap-2">
          <input className="hs-input" value={linkUrl} placeholder="https://" aria-label={t('homestation.publish.linkUrl', 'Link URL')} onChange={event => setLinkUrl(event.target.value)} data-testid={`${idPrefix}-link-url`} />
          <button type="button" className="hs-btn" disabled={linkBusy} onClick={() => void fetchLink()}>{linkBusy ? <Loader2 size={16} className="animate-spin" /> : null}{t('homestation.publish.linkFetch', 'Make card')}</button>
        </div>
      ) : null}
      {linkError ? <p className="mt-2 text-[13px]" style={{ color: 'var(--hs-danger-ink)' }}>{linkError}</p> : null}
    </>
  )

  const submitButton = (
    <button type="submit" className="hs-btn is-primary" disabled={busy || uploading || failed} data-testid={`${idPrefix}-submit`}>
      {busy ? <Loader2 size={16} className="animate-spin" /> : null}
      {busy ? t('homestation.publish.publishing', 'Publishing…') : t('homestation.publish.submit', 'Publish')}
    </button>
  )

  const audienceBlock = (
    <>
      <Controller control={form.control} name="audience" render={({ field }) => <AudiencePicker idPrefix={idPrefix} value={field.value} onChange={field.onChange} compact={compact} />} />
      {home?.zoneFeed.writer ? (
        <Controller
          control={form.control}
          name="zoneFeed"
          render={({ field }) => (
            <label className="mt-3 flex items-center gap-2 text-[13px] leading-5" style={{ color: audience.kind === 'public' ? undefined : 'var(--cp-muted)' }} data-testid={`${idPrefix}-zone-feed`}>
              <input type="checkbox" checked={audience.kind === 'public' && !!field.value} disabled={audience.kind !== 'public'} onChange={event => field.onChange(event.target.checked)} />
              <span>
                {t('homestation.publish.zoneFeed', 'Also show on {{zone}}', { zone: home.zoneFeed.name })}
                {audience.kind !== 'public' ? ` · ${t('homestation.publish.zoneFeedPublicOnly', 'public posts only')}` : ''}
              </span>
            </label>
          )}
        />
      ) : null}
    </>
  )

  const feedback = (
    <>
      {errorKey ? <p className="mt-3 text-[13px]" role="alert" style={{ color: 'var(--hs-danger-ink)' }}>{t(errorKey, 'Check the post before publishing.')}</p> : null}
      {failed ? (
        <div className="hs-note is-danger mt-3 flex-wrap items-center" role="alert" data-testid={`${idPrefix}-failed`}>
          <CircleAlert size={16} />
          <span className="min-w-0 flex-1">{t('homestation.publish.failedBody', 'Not published: your HomeStation couldn’t store the post. Nothing was sent. Retrying reuses the same publish ID, so it won’t create a duplicate.')}</span>
          <button type="button" className="hs-btn is-small" onClick={() => void retry()} data-testid={`${idPrefix}-retry`}><RotateCw size={14} />{t('common.retry', 'Retry')}</button>
        </div>
      ) : null}
    </>
  )

  if (compact) {
    return (
      <form onSubmit={submit} data-testid={`${idPrefix}-form`}>
        <div className="rounded-xl px-3 pt-2" style={{ border: '1px solid var(--hs-line-strong)' }}>
          <textarea
            {...textField}
            rows={3}
            className="w-full resize-none bg-transparent py-1 text-[15px] leading-[22px] outline-none placeholder:text-[color:var(--cp-muted)]"
            placeholder={t('homestation.publish.placeholder', 'What’s new?')}
            aria-label={t('homestation.publish.placeholder', 'What’s new?')}
            data-testid={`${idPrefix}-text`}
          />
          {extras}
          {fileInputs}
          <div className="py-2">{tools}</div>
        </div>
        <div className="mt-3">{audienceBlock}</div>
        <div className="mt-3 flex items-center justify-end gap-3">
          <span className="text-[13px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>{text.length}</span>
          {submitButton}
        </div>
        {feedback}
      </form>
    )
  }

  return (
    <form className={nav.isDesktop ? 'px-6 py-5' : 'px-4 py-4'} onSubmit={submit} data-testid={`${idPrefix}-form`}>
      <div className="flex gap-3">
        <Avatar identity={owner} size={36} />
        <div className="min-w-0 flex-1">
          <p className="text-[15px] font-semibold leading-[22px]">{owner.name}</p>
          <textarea
            {...textField}
            rows={5}
            className="w-full resize-none bg-transparent text-[15px] leading-[22px] outline-none placeholder:text-[color:var(--cp-muted)]"
            style={{ minHeight: 110 }}
            placeholder={t('homestation.publish.placeholder', 'What’s new?')}
            aria-label={t('homestation.publish.placeholder', 'What’s new?')}
            data-testid={`${idPrefix}-text`}
          />
          {extras}
          {fileInputs}
          <div className="mt-2">{tools}</div>
        </div>
      </div>
      <div className="mt-4 pt-4" style={{ borderTop: '1px solid var(--hs-line)' }}>
        {audienceBlock}
        <div className="mt-4 flex items-center gap-3">
          <p className="min-w-0 flex-1 text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.publish.hint', 'Your post is signed by you and added to your home feed for its audience. It isn’t delivered to your own reading list.')}</p>
          <span className="text-[13px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>{text.length}</span>
          {submitButton}
        </div>
        {feedback}
      </div>
    </form>
  )
}
