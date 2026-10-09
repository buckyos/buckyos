import { zodResolver } from '@hookform/resolvers/zod'
import { ChevronDown, ChevronRight, History, Lock, MessageCircle, Send } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useForm } from 'react-hook-form'
import useSWR from 'swr'
import { useI18n } from '../../../i18n/provider'
import { VerificationBadge } from '../card/parts'
import { formatCount, formatDateTime, formatTimeAgo, viewLabel } from '../datamodel/format'
import { commentInputSchema, type CommentInput } from '../datamodel/inputs'
import type { CardView, CommentView, StatsViewKey } from '../datamodel/types'
import type { CommentTypeFilter } from '../mock/store'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useNow, useStoreRevalidate } from '../store/context'
import { Avatar, EmptyState, ErrorState, ListSkeleton } from '../ui/primitives'
import { useToast } from '../ui/toastContext'

const COLLECTOR_VIEW: StatsViewKey = 'collector:open-index'

function CommentRow({ comment, view }: { comment: CommentView; view: StatsViewKey }) {
  const { t } = useI18n()
  const now = useNow()
  const nav = useHsNav()
  const [open, setOpen] = useState(false)
  const text = comment.item.object.content?.text
  const kindLabel = comment.commentType === 'like' ? t('homestation.action.liked', 'liked') : comment.commentType === 'repost' ? t('homestation.action.reposted', 'reposted') : comment.commentType === 'quote' ? t('homestation.action.quoted', 'quoted') : null
  const pathLabel = (kind: CommentView['sourcePaths'][number]['kind'], label: string) => ({
    author_list: t('homestation.comments.pathAuthor', 'Author’s list ({{name}})', { name: label }),
    collector: t('homestation.comments.pathCollector', 'Collector {{name}}', { name: label }),
    push: t('homestation.comments.pathPush', 'Delivered to your HomeStation by {{name}}', { name: label }),
    participant: t('homestation.comments.pathSelf', 'Published by you'),
  })[kind]
  return (
    <li className="py-3" style={{ borderBottom: '1px solid var(--hs-divider)' }} data-testid="hs-comment-row" data-old-version={comment.onOldVersion ? 'true' : 'false'}>
      <div className="flex items-start gap-2.5">
        <Avatar identity={comment.item.publisher} size={30} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[13px]">
            <span className="font-semibold">{comment.item.publisher.name}</span>
            <VerificationBadge verification={comment.item.verification} />
            {kindLabel ? <span style={{ color: 'var(--cp-muted)' }}>{kindLabel}</span> : null}
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>· {formatTimeAgo(t, comment.item.createdAt, now)}</span>
            {comment.onOldVersion ? <span className="hs-badge is-warning" data-testid="hs-old-version"><History size={10} />{t('homestation.comments.oldVersion', 'On an earlier version (v{{n}})', { n: comment.targetVersion })}</span> : null}
            {comment.item.audience.restricted ? <span className="hs-badge"><Lock size={10} />{t('homestation.comments.authorOnly', 'Visible to the author only')}</span> : null}
          </div>
          {text ? <p className="mt-0.5 whitespace-pre-line text-sm leading-relaxed">{text}</p> : null}
          {view === 'local' && !comment.listedByAuthor ? (
            <p className="mt-1 text-[11px]" style={{ color: 'var(--cp-muted)' }} data-testid="hs-not-in-author-view">{t('homestation.comments.notInAuthorView', 'The author’s view doesn’t list this comment right now.')}</p>
          ) : null}
          <button type="button" className="mt-1 inline-flex items-center gap-1 text-[11px]" style={{ color: 'var(--cp-muted)' }} aria-expanded={open} onClick={() => setOpen(value => !value)}>
            {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
            {t('homestation.comments.where', 'Where this came from')}
          </button>
          {open ? (
            <ul className="mt-1 space-y-0.5 text-[11px]" style={{ color: 'var(--cp-muted)' }} data-testid="hs-comment-paths">
              {comment.sourcePaths.map(path => <li key={`${path.kind}-${path.label}`}>· {pathLabel(path.kind, path.label)}</li>)}
              {comment.onOldVersion ? (
                <li>
                  · <button type="button" className="underline" onClick={() => nav.openDetail(comment.targetObjId)}>{t('homestation.comments.openVersion', 'Open the version it was written on')}</button>
                </li>
              ) : null}
            </ul>
          ) : null}
        </div>
      </div>
    </li>
  )
}

function CommentComposer({ view }: { view: CardView }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const toast = useToast()
  const nav = useHsNav()
  const form = useForm<CommentInput>({ resolver: zodResolver(commentInputSchema), defaultValues: { text: '' } })
  const restricted = view.item.audience.restricted
  const error = form.formState.errors.text?.message
  const submit = form.handleSubmit(async values => {
    const result = await store.comment(view.item.objId, values.text)
    form.reset({ text: '' })
    toast({
      text: result.audience.kind === 'dids'
        ? t('homestation.comments.postedAuthorOnly', 'Comment published to the author only. It won’t appear on your public homepage.')
        : t('homestation.comments.posted', 'Comment published. It’s in My publications.'),
      tone: 'success',
      action: { label: t('homestation.toast.viewPublished', 'View'), onClick: () => nav.navigate({ name: 'published' }, { reset: true }) },
    })
  })
  return (
    <form className="mt-3" onSubmit={submit} data-testid="hs-comment-form">
      {restricted ? (
        <p className="mb-2 flex items-start gap-1.5 rounded-xl px-3 py-2 text-xs leading-5" style={{ background: 'var(--hs-pressed-bg)' }} data-testid="hs-comment-restricted">
          <Lock size={13} className="mt-0.5 flex-shrink-0" />
          {t('homestation.comments.restrictedNote', 'This post has a restricted audience. Your comment goes only to its author and stays off your public homepage.')}
        </p>
      ) : null}
      <div className="flex items-end gap-2">
        <textarea
          {...form.register('text')}
          rows={2}
          className="hs-input resize-none"
          placeholder={t('homestation.comments.placeholder', 'Write a comment — it’s published as your own post')}
          aria-label={t('homestation.comments.placeholder', 'Write a comment — it’s published as your own post')}
        />
        <button type="submit" className="hs-btn is-primary" disabled={form.formState.isSubmitting} data-testid="hs-comment-submit">
          <Send size={14} />
          {t('homestation.comments.submit', 'Comment')}
        </button>
      </div>
      {error ? <p className="mt-1 text-xs" role="alert" style={{ color: 'var(--cp-danger)' }}>{t(error, 'Write something first.')}</p> : null}
    </form>
  )
}

export function CommentSection({ view: card }: { view: CardView }) {
  const { t, locale } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const owner = nav.perspective === 'owner'
  const [view, setView] = useState<StatsViewKey>(owner ? 'local' : 'author')
  const [type, setType] = useState<CommentTypeFilter>('text')
  const objId = card.item.objId
  const swr = useSWR(['hs-comments', store.id, objId, view, type], () => store.listComments(objId, { view, type }), { revalidateOnFocus: false, shouldRetryOnError: false })
  const { mutate } = swr
  const revalidate = useCallback(() => void mutate(), [mutate])
  useStoreRevalidate(['comments'], revalidate)
  const data = swr.data
  const current = data?.comments.filter(comment => !comment.onOldVersion) ?? []
  const older = data?.comments.filter(comment => comment.onOldVersion) ?? []
  const views: StatsViewKey[] = owner ? ['local', 'author', COLLECTOR_VIEW] : ['author']
  const types: { id: CommentTypeFilter; label: string }[] = [
    { id: 'text', label: t('homestation.comments.typeText', 'Comments') },
    { id: 'like', label: t('homestation.comments.typeLike', 'Likes') },
    { id: 'repost', label: t('homestation.comments.typeRepost', 'Reposts') },
    { id: 'quote', label: t('homestation.comments.typeQuote', 'Quotes') },
  ]
  const stats = data?.stats

  return (
    <section className="mt-6" aria-label={t('homestation.comments.title', 'Comments and interactions')} data-testid="hs-comments">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="flex items-center gap-1.5 text-sm font-semibold"><MessageCircle size={15} />{t('homestation.comments.title', 'Comments and interactions')}</h3>
        <div className="flex flex-wrap gap-1" role="radiogroup" aria-label={t('homestation.comments.viewLabel', 'Comment view')}>
          {views.map(entry => (
            <button key={entry} type="button" role="radio" aria-checked={view === entry} className="hs-chip" data-testid={`hs-comment-view-${entry.split(':')[0]}`} onClick={() => setView(entry)}>
              {viewLabel(t, entry, 'Open Index')}
            </button>
          ))}
        </div>
      </div>
      <p className="mt-1.5 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }}>
        {view === 'local'
          ? t('homestation.comments.localHint', 'Merged from the author’s list, collectors and deliveries you received. It is your local result, not every comment in the network.')
          : view === 'author'
            ? t('homestation.comments.authorHint', 'What the author chose to list. The author can organize this view but not what others keep.')
            : t('homestation.comments.collectorHint', 'What this independent collector has gathered so far; coverage differs from the author’s view.')}
      </p>
      <div className="mt-2 flex flex-wrap gap-1.5" role="tablist" aria-label={t('homestation.comments.typeLabel', 'Interaction type')}>
        {types.map(entry => (
          <button key={entry.id} type="button" role="tab" aria-selected={type === entry.id} className="hs-chip" data-testid={`hs-comment-type-${entry.id}`} onClick={() => setType(entry.id)}>
            {entry.label}
          </button>
        ))}
      </div>
      {stats ? (
        <p className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]" style={{ color: 'var(--cp-muted)' }} data-testid="hs-stats-scope">
          <span className="hs-badge is-success">{view === 'local' ? t('homestation.stats.localVerified', 'Local verified') : viewLabel(t, view, 'Open Index')}</span>
          <span>{t('homestation.stats.summary', '{{likes}} likes · {{comments}} comments · {{reposts}} reposts · {{quotes}} quotes', { likes: formatCount(stats.likes), comments: formatCount(stats.textComments), reposts: formatCount(stats.reposts), quotes: formatCount(stats.quotes) })}</span>
          <span>{t('homestation.stats.asOf', 'as of {{time}}', { time: formatDateTime(stats.asOf, locale) })}</span>
          {stats.sync === 'partial' ? <span className="hs-badge is-warning">{t('homestation.stats.partial', 'Partial sync')}</span> : null}
          {stats.claimed?.likes ? <span className="hs-badge">{t('homestation.stats.claimedBy', '{{name}} claims {{n}} likes (unverified)', { name: stats.claimed.source, n: formatCount(stats.claimed.likes) })}</span> : null}
        </p>
      ) : null}
      {owner ? <CommentComposer view={card} /> : null}
      {swr.isLoading ? <ListSkeleton rows={2} /> : null}
      {swr.error ? <ErrorState onRetry={revalidate} /> : null}
      {data && data.comments.length === 0 ? <EmptyState icon={<MessageCircle size={28} strokeWidth={1.4} />} title={t('homestation.comments.empty', 'Nothing in this view yet')} /> : null}
      {current.length > 0 ? <ul className="mt-1">{current.map(comment => <CommentRow key={comment.objId} comment={comment} view={view} />)}</ul> : null}
      {older.length > 0 ? (
        <div className="mt-4" data-testid="hs-older-comments">
          <p className="flex items-center gap-1.5 text-xs font-semibold" style={{ color: 'var(--cp-muted)' }}>
            <History size={13} />
            {t('homestation.comments.olderTitle', 'On earlier versions ({{n}}) — they don’t endorse this version', { n: older.length })}
          </p>
          <ul>{older.map(comment => <CommentRow key={comment.objId} comment={comment} view={view} />)}</ul>
        </div>
      ) : null}
    </section>
  )
}
