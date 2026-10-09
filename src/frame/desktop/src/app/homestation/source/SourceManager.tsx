import { zodResolver } from '@hookform/resolvers/zod'
import { AlertTriangle, BellOff, Bot, CheckCircle2, Globe, Link, Loader2, MessageSquare, Pause, Play, Rss, Search, UserPlus, Users } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useForm, useWatch } from 'react-hook-form'
import useSWR from 'swr'
import { useI18n } from '../../../i18n/provider'
import { useItemActions } from '../actions'
import { formatTimeAgo, type Translate } from '../datamodel/format'
import { sourceInputSchema, type SourceInput } from '../datamodel/inputs'
import type { NotifyState, SourceResolution, SourceView, SubscriptionIntent } from '../datamodel/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useNow, useStoreRevalidate } from '../store/context'
import { ErrorState, ListSkeleton, PageHeader } from '../ui/primitives'
import { useToast } from '../ui/toastContext'

function notifyBadge(t: Translate, notify: NotifyState) {
  switch (notify) {
    case 'acknowledged':
      return <span className="hs-badge is-success" data-testid="hs-notify" data-state="acknowledged"><CheckCircle2 size={10} />{t('homestation.sources.notifyAck', 'They were notified')}</span>
    case 'unsupported':
      return <span className="hs-badge" data-testid="hs-notify" data-state="unsupported"><BellOff size={10} />{t('homestation.sources.notifyUnsupported', 'Source can’t receive follow notices')}</span>
    case 'retrying':
      return <span className="hs-badge is-warning" data-testid="hs-notify" data-state="retrying"><Loader2 size={10} />{t('homestation.sources.notifyRetrying', 'Notice pending, retrying')}</span>
    case 'pending':
      return <span className="hs-badge" data-testid="hs-notify" data-state="pending"><Loader2 size={10} className="animate-spin" />{t('homestation.sources.notifyPending', 'Sending follow notice…')}</span>
  }
}

function credibilityLabel(t: Translate, source: SourceView) {
  if (source.credibility === 'verified_did') return t('homestation.sources.credDid', 'Verified DID')
  if (source.credibility === 'known_site') return t('homestation.sources.credKnown', 'Known site')
  return t('homestation.sources.credUnknown', 'Unknown site')
}

function SourceIcon({ source }: { source: SourceView }) {
  const icon = source.kind === 'person' ? <Users size={15} /> : source.kind === 'rss' ? <Rss size={15} /> : source.kind === 'channel' ? <MessageSquare size={15} /> : <Globe size={15} />
  return <span className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full" style={{ background: 'var(--hs-chip-bg)', color: 'var(--cp-muted)' }}>{icon}</span>
}

function SourceRow({ source }: { source: SourceView }) {
  const { t } = useI18n()
  const now = useNow()
  const store = useHomeStationStore()
  const actions = useItemActions()
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  const friendOnly = source.basis.includes('friend') && !source.basis.includes('active')
  const unsubscribe = async () => {
    setBusy(true)
    const result = await store.unfollow(source.id)
    setBusy(false)
    toast({ text: result === 'friend_basis' ? t('homestation.sources.keptByFriendship', 'Removed your own follow. {{name}} is still followed because you are friends.', { name: source.name }) : t('homestation.sources.removed', 'Unsubscribed from {{name}}.', { name: source.name }) })
  }
  return (
    <li className="flex items-start gap-3 py-3" style={{ borderBottom: '1px solid var(--hs-divider)' }} data-testid="hs-source-row" data-source={source.id}>
      <SourceIcon source={source} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="truncate text-sm font-semibold">{source.name}</span>
          {source.basis.includes('active') ? <span className="hs-badge">{t('homestation.sources.basisActive', 'Followed by you')}</span> : null}
          {source.basis.includes('friend') ? <span className="hs-badge is-accent">{t('homestation.sources.basisFriend', 'From friendship')}</span> : null}
          {source.paused ? <span className="hs-badge is-warning">{t('homestation.sources.paused', 'Collection paused')}</span> : null}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
          {notifyBadge(t, source.notify)}
          <span>{credibilityLabel(t, source)}</span>
          {source.updateMode ? <span>· {source.updateMode}</span> : null}
          {source.lastSuccessAt ? <span>· {t('homestation.sources.lastSuccess', 'last fetched {{time}} ago', { time: formatTimeAgo(t, source.lastSuccessAt, now) })}</span> : null}
        </div>
        {source.lastError && !source.paused ? (
          <p className="mt-1 flex items-start gap-1 text-[11px] leading-4" style={{ color: 'color-mix(in srgb, var(--cp-warning) 50%, var(--cp-text))' }} data-testid="hs-source-error">
            <AlertTriangle size={11} className="mt-0.5 flex-shrink-0" />
            {source.lastError.reason}
          </p>
        ) : null}
        {friendOnly ? (
          <p className="mt-1 text-[11px] leading-4" style={{ color: 'var(--cp-muted)' }} data-testid="hs-friend-derived">
            {t('homestation.sources.friendHint', 'Friends follow each other by default, so this can’t be unsubscribed on its own.')}
            {' '}
            {source.did ? <button type="button" className="underline" onClick={() => void actions.mute({ kind: 'person', did: source.did!, name: source.name })}>{t('homestation.sources.muteInstead', 'Don’t show their posts instead')}</button> : null}
          </p>
        ) : null}
      </div>
      <div className="flex flex-shrink-0 flex-col items-end gap-1">
        <button type="button" className="hs-badge" onClick={() => void store.pauseSource(source.id, !source.paused)} data-testid="hs-source-pause">
          {source.paused ? <Play size={10} /> : <Pause size={10} />}
          {source.paused ? t('homestation.sources.resume', 'Resume collecting') : t('homestation.sources.pause', 'Pause collecting')}
        </button>
        {!friendOnly ? (
          <button type="button" className="hs-badge is-danger" disabled={busy} onClick={() => void unsubscribe()} data-testid="hs-source-unsubscribe">
            {t('homestation.sources.unsubscribe', 'Unsubscribe')}
          </button>
        ) : null}
      </div>
    </li>
  )
}

function IntentBlock({ intent, sources }: { intent: SubscriptionIntent; sources: SourceView[] }) {
  const { t } = useI18n()
  const mapped = sources.filter(source => intent.sourceIds.includes(source.id))
  return (
    <div className="rounded-2xl border px-3 py-2.5" style={{ borderColor: 'var(--hs-divider)' }} data-testid="hs-intent">
      <div className="flex items-center gap-2 text-sm">
        <Bot size={15} style={{ color: 'var(--cp-accent)' }} />
        <span className="flex-1 font-medium">“{intent.text}”</span>
        <span className={intent.status === 'collecting' ? 'hs-badge is-success' : 'hs-badge'}>{intent.status === 'collecting' ? t('homestation.sources.intentCollecting', 'Collecting') : intent.status === 'mapping' ? t('homestation.sources.intentMapping', 'Agent is mapping sources') : t('homestation.sources.paused', 'Collection paused')}</span>
      </div>
      <p className="mt-1 text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.sources.intentNote', 'Your agent keeps this intent and maintains the sources behind it; it is not a one-off search.')}</p>
      <ul className="mt-1.5 flex flex-wrap gap-1.5">
        {mapped.map(source => <li key={source.id} className="hs-badge">{source.name}{source.lastError ? ' ⚠' : ''}</li>)}
      </ul>
    </div>
  )
}

function ResolutionPanel({ resolution, onDone }: { resolution: SourceResolution; onDone: () => void }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const toast = useToast()
  const [busy, setBusy] = useState(false)
  const subscribe = async (candidates: SourceView[]) => {
    setBusy(true)
    await store.follow({ ...resolution, candidates })
    setBusy(false)
    toast({ text: resolution.inputKind === 'follow' ? t('homestation.sources.followDone', 'Following. A signed follow notice is on its way.') : t('homestation.sources.subscribeDone', 'Subscribed. New items go to your candidates first.'), tone: 'success' })
    onDone()
  }
  if (resolution.candidates.length === 0) {
    return <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }} data-testid="hs-resolution-empty">{t('homestation.sources.noMatch', 'Nothing matched. Try a full DID such as did:bns:alice.')}</p>
  }
  return (
    <div className="mt-3 space-y-2" data-testid="hs-resolution">
      {resolution.inputKind === 'natural' ? <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('homestation.sources.mapped', 'Your agent mapped “{{text}}” to {{n}} sources. It keeps maintaining this list.', { text: resolution.input, n: resolution.candidates.length })}</p> : null}
      <ul className="space-y-1.5">
        {resolution.candidates.map(candidate => (
          <li key={candidate.id} className="flex items-start gap-2 rounded-xl px-3 py-2" style={{ background: 'var(--hs-subtle-bg)' }}>
            <SourceIcon source={candidate} />
            <div className="min-w-0 flex-1 text-xs">
              <p className="text-sm font-semibold">{candidate.name}</p>
              <p style={{ color: 'var(--cp-muted)' }}>{credibilityLabel(t, candidate)} · {candidate.updateMode ?? candidate.kind}</p>
              <div className="mt-1">{notifyBadge(t, candidate.notify === 'pending' ? 'pending' : candidate.notify)}</div>
              {candidate.basis.length ? <p className="mt-1" style={{ color: 'var(--cp-muted)' }}>{t('homestation.sources.alreadyFollowing', 'Already in your sources')}</p> : null}
            </div>
            {resolution.inputKind !== 'natural' ? (
              <button type="button" className="hs-btn is-primary" disabled={busy || candidate.basis.includes('active')} onClick={() => void subscribe([candidate])} data-testid="hs-resolution-follow">
                <UserPlus size={13} />
                {resolution.inputKind === 'follow' ? t('homestation.sources.follow', 'Follow') : t('homestation.sources.subscribe', 'Subscribe')}
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      {resolution.inputKind === 'natural' ? (
        <button type="button" className="hs-btn is-primary" disabled={busy} onClick={() => void subscribe(resolution.candidates)} data-testid="hs-resolution-follow">
          {t('homestation.sources.subscribeIntent', 'Keep this subscription')}
        </button>
      ) : null}
      {resolution.inputKind === 'follow' ? <p className="text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.sources.followNote', 'Following tells them you follow (a signed notice) and reads their home feed. It doesn’t make you friends and doesn’t mean everything enters your feed.')}</p> : null}
    </div>
  )
}

function AddSource() {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const [resolution, setResolution] = useState<SourceResolution | null>(null)
  const form = useForm<SourceInput>({ resolver: zodResolver(sourceInputSchema), defaultValues: { kind: 'follow', text: '' } })
  const kind = useWatch({ control: form.control, name: 'kind' })
  const submit = form.handleSubmit(async values => setResolution(await store.resolveSourceInput(values.kind, values.text)))
  const modes: { id: SourceInput['kind']; label: string; icon: React.ReactNode; placeholder: string }[] = [
    { id: 'follow', label: t('homestation.sources.modeFollow', 'Follow'), icon: <UserPlus size={14} />, placeholder: t('homestation.sources.followPlaceholder', 'A name or DID, e.g. did:bns:david') },
    { id: 'url', label: t('homestation.sources.modeUrl', 'Paste URL'), icon: <Link size={14} />, placeholder: t('homestation.sources.urlPlaceholder', 'A website, RSS feed or HomeStation URL') },
    { id: 'natural', label: t('homestation.sources.modeNatural', 'Describe'), icon: <MessageSquare size={14} />, placeholder: t('homestation.sources.naturalPlaceholder', 'e.g. second-hand furniture in San Francisco') },
  ]
  const error = form.formState.errors.text?.message
  return (
    <form className="rounded-2xl border p-3" style={{ borderColor: 'var(--hs-divider)' }} onSubmit={submit} data-testid="hs-add-source">
      <div className="flex gap-1.5" role="radiogroup" aria-label={t('homestation.sources.addLabel', 'Add a source')}>
        {modes.map(mode => (
          <button key={mode.id} type="button" role="radio" aria-checked={kind === mode.id} className="hs-chip flex-1 justify-center" onClick={() => { form.setValue('kind', mode.id); setResolution(null) }} data-testid={`hs-source-mode-${mode.id}`}>
            {mode.icon}
            {mode.label}
          </button>
        ))}
      </div>
      <div className="mt-2 flex items-center gap-2">
        <Search size={14} style={{ color: 'var(--cp-muted)' }} />
        <input {...form.register('text')} className="hs-input py-1.5 text-sm" placeholder={modes.find(mode => mode.id === kind)?.placeholder} aria-label={modes.find(mode => mode.id === kind)?.placeholder} data-testid="hs-source-input" />
        <button type="submit" className="hs-btn whitespace-nowrap" disabled={form.formState.isSubmitting} data-testid="hs-source-resolve">
          {form.formState.isSubmitting ? <Loader2 size={13} className="animate-spin" /> : null}
          {t('homestation.sources.resolve', 'Look up')}
        </button>
      </div>
      {error ? <p className="mt-1 text-xs" role="alert" style={{ color: 'var(--cp-danger)' }}>{t(error, 'Check the input.')}</p> : null}
      {resolution ? <ResolutionPanel resolution={resolution} onDone={() => { setResolution(null); form.reset({ kind, text: '' }) }} /> : null}
    </form>
  )
}

export function SourceManager() {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const swr = useSWR(['hs-sources', store.id], () => store.listSources(), { revalidateOnFocus: false })
  const { mutate } = swr
  useStoreRevalidate(['sources'], useCallback(() => void mutate(), [mutate]))
  const sources = swr.data?.sources ?? []
  const people = sources.filter(source => source.kind === 'person')
  const others = sources.filter(source => source.kind !== 'person' && !source.intentId)
  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-sources">
      <PageHeader title={t('homestation.nav.sources', 'Sources')} subtitle={t('homestation.sources.subtitle', 'Who and what your HomeStation collects from')} onBack={nav.isDesktop ? undefined : nav.back} backLabel={t('common.back', 'Back')} />
      <div className="desktop-scrollbar mx-auto min-h-0 w-full max-w-[760px] flex-1 overflow-y-auto px-4 py-3">
        <AddSource />
        {swr.isLoading ? <ListSkeleton rows={3} /> : null}
        {swr.error ? <ErrorState onRetry={() => void mutate()} /> : null}
        {swr.data?.intents.length ? (
          <section className="mt-5 space-y-2">
            <h3 className="hs-section-title">{t('homestation.sources.intents', 'Subscription intents')}</h3>
            {swr.data.intents.map(intent => <IntentBlock key={intent.id} intent={intent} sources={sources} />)}
          </section>
        ) : null}
        {people.length ? (
          <section className="mt-5">
            <h3 className="hs-section-title">{t('homestation.sources.people', 'People ({{n}})', { n: people.length })}</h3>
            <ul>{people.map(source => <SourceRow key={source.id} source={source} />)}</ul>
          </section>
        ) : null}
        {others.length ? (
          <section className="mt-5">
            <h3 className="hs-section-title">{t('homestation.sources.sites', 'Sites and feeds ({{n}})', { n: others.length })}</h3>
            <ul>{others.map(source => <SourceRow key={source.id} source={source} />)}</ul>
          </section>
        ) : null}
        {sources.filter(source => source.intentId).length ? (
          <section className="mt-5">
            <h3 className="hs-section-title">{t('homestation.sources.mappedSources', 'Mapped by your agent')}</h3>
            <ul>{sources.filter(source => source.intentId).map(source => <SourceRow key={source.id} source={source} />)}</ul>
          </section>
        ) : null}
        <div className="h-20" />
      </div>
    </div>
  )
}
