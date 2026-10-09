import { zodResolver } from '@hookform/resolvers/zod'
import { EyeOff, Filter, Plus, Sparkles, UsersRound } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Controller, useForm } from 'react-hook-form'
import { useI18n } from '../../../i18n/provider'
import { filterRuleInputSchema, type FilterRuleInput } from '../datamodel/inputs'
import type { AudienceSpec, FilterRule, GenerationTag, MuteRule } from '../datamodel/types'
import type { HomeStationStore } from '../mock/store'
import { useHsNav } from '../navContext'
import { AudiencePicker } from '../publish/AudiencePicker'
import { useHomeStationStore, useStoreSelector } from '../store/context'
import { PageHeader } from '../ui/primitives'
import { useToast } from '../ui/toastContext'

const selectMutes = (store: HomeStationStore) => store.peekMuteRules()
const selectRules = (store: HomeStationStore) => store.peekFilterRules()
const selectGroups = (store: HomeStationStore) => store.peekGroups()
const selectFriends = (store: HomeStationStore) => store.peekFriends()
const selectHidden = (store: HomeStationStore) => store.peekHiddenSummary()
const selectSettings = (store: HomeStationStore) => store.peekSettings()

function useConditionLabel() {
  const { t } = useI18n()
  return useCallback((condition: GenerationTag) => ({
    ai_full: t('homestation.filters.condAiFull', 'Fully AI-generated'),
    ai_assisted: t('homestation.filters.condAiAssisted', 'AI-assisted'),
    low_quality: t('homestation.filters.condLowQuality', 'Low quality'),
  })[condition], [t])
}

function RuleEditor({ rule }: { rule: FilterRule }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const toast = useToast()
  const conditionLabel = useConditionLabel()
  const form = useForm<FilterRuleInput>({ resolver: zodResolver(filterRuleInputSchema), defaultValues: { enabled: rule.enabled, conditions: rule.conditions, acceptInferred: rule.acceptInferred, minConfidence: rule.minConfidence, unknown: rule.unknown } })
  const save = form.handleSubmit(async values => {
    await store.setFilterRule({ ...rule, ...values })
    form.reset(values)
    toast({ text: t('homestation.filters.saved', 'Rule saved. Your feed, topics and catch-up were recomputed.'), tone: 'success' })
  })
  const error = form.formState.errors.conditions?.message
  return (
    <form className="rounded-2xl border p-3 text-sm" style={{ borderColor: 'var(--hs-divider)' }} onSubmit={save} data-testid="hs-rule-editor" data-rule={rule.id}>
      <label className="flex items-center gap-2 font-medium">
        <input type="checkbox" {...form.register('enabled')} data-testid="hs-rule-enabled" />
        {t('homestation.filters.ruleTitle', 'Hide items that are all of:')}
      </label>
      <Controller
        control={form.control}
        name="conditions"
        render={({ field }) => (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {(['ai_full', 'ai_assisted', 'low_quality'] as GenerationTag[]).map(condition => (
              <button
                key={condition}
                type="button"
                className="hs-chip"
                aria-pressed={field.value.includes(condition)}
                onClick={() => field.onChange(field.value.includes(condition) ? field.value.filter(value => value !== condition) : [...field.value, condition])}
              >
                <Sparkles size={12} />
                {conditionLabel(condition)}
              </button>
            ))}
          </div>
        )}
      />
      {error ? <p className="mt-1 text-xs" role="alert" style={{ color: 'var(--cp-danger)' }}>{t(error, 'Choose at least one condition.')}</p> : null}
      <label className="mt-3 flex items-center gap-2 text-xs">
        <input type="checkbox" {...form.register('acceptInferred')} />
        {t('homestation.filters.acceptInferred', 'Accept your local model’s inferences (not only author declarations or your own corrections)')}
      </label>
      <label className="mt-2 flex flex-wrap items-center gap-2 text-xs">
        {t('homestation.filters.threshold', 'Minimum model score')}
        <Controller control={form.control} name="minConfidence" render={({ field }) => (
          <>
            <input type="range" min={0.5} max={0.99} step={0.01} value={field.value} onChange={event => field.onChange(Number(event.target.value))} aria-label={t('homestation.filters.threshold', 'Minimum model score')} />
            <span className="tabular-nums">{field.value.toFixed(2)}</span>
          </>
        )} />
      </label>
      <label className="mt-2 flex flex-wrap items-center gap-2 text-xs">
        {t('homestation.filters.unknown', 'When an item hasn’t been classified yet')}
        <select className="hs-input w-auto py-1 text-xs" {...form.register('unknown')}>
          <option value="show">{t('homestation.filters.unknownShow', 'show it (unknown is not “human-made”)')}</option>
          <option value="hide">{t('homestation.filters.unknownHide', 'hide it until classified')}</option>
        </select>
      </label>
      <div className="mt-3 flex justify-end">
        <button type="submit" className="hs-btn is-primary" disabled={!form.formState.isDirty} data-testid="hs-rule-save">{t('common.save', 'Save')}</button>
      </div>
    </form>
  )
}

export function FeedPreferences() {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const nav = useHsNav()
  const toast = useToast()
  const mutes = useStoreSelector(selectMutes)
  const rules = useStoreSelector(selectRules)
  const groups = useStoreSelector(selectGroups)
  const friends = useStoreSelector(selectFriends)
  const hidden = useStoreSelector(selectHidden)
  const settings = useStoreSelector(selectSettings)
  const [choice, setChoice] = useState('')
  const people = [...new Set([...friends, 'did:bns:sarah', 'did:bns:david'])].map(did => ({ did, name: store.peekIdentity(did).name }))

  const addMute = () => {
    if (!choice) return
    const [kind, id] = choice.split('|')
    const rule: MuteRule = kind === 'group' ? { kind: 'group', groupId: id, name: groups.find(group => group.id === id)?.name ?? id } : { kind: 'person', did: id, name: store.peekIdentity(id).name }
    void store.setMuteRule(rule, true)
    setChoice('')
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-prefs">
      <PageHeader title={t('homestation.nav.prefs', 'Filters and “don’t show”')} subtitle={t('homestation.prefs.subtitle', 'Private display rules. Nothing here is sent to anyone.')} onBack={nav.isDesktop ? undefined : nav.back} backLabel={t('common.back', 'Back')} />
      <div className="desktop-scrollbar mx-auto min-h-0 w-full max-w-[760px] flex-1 space-y-6 overflow-y-auto px-4 py-4">
        <section>
          <h3 className="hs-section-title flex items-center gap-1.5"><EyeOff size={12} />{t('homestation.prefs.muteTitle', 'Don’t show posts from')}</h3>
          <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.prefs.muteNote', 'Applies to your main feed and the catch-up list. You keep following them and stay friends; they are not notified, and chat permissions don’t change.')}</p>
          <ul className="mt-2 space-y-1.5" data-testid="hs-mute-list">
            {mutes.length === 0 ? <li className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('homestation.prefs.muteEmpty', 'Nobody is hidden.')}</li> : null}
            {mutes.map(rule => (
              <li key={rule.kind === 'person' ? rule.did : rule.groupId} className="flex items-center gap-2 rounded-xl px-3 py-2 text-sm" style={{ background: 'var(--hs-subtle-bg)' }}>
                {rule.kind === 'group' ? <UsersRound size={14} /> : <EyeOff size={14} />}
                <span className="flex-1">{rule.kind === 'group' ? t('homestation.prefs.muteGroupItem', 'Group “{{name}}”', { name: rule.name }) : rule.name}</span>
                <button type="button" className="hs-badge is-accent" onClick={() => { void store.setMuteRule(rule, false); toast({ text: t('homestation.prefs.unmuted', 'Showing {{name}} again. Already-collected posts come back after filtering.', { name: rule.name }) }) }} data-testid="hs-unmute">
                  {t('homestation.prefs.unmute', 'Show again')}
                </button>
              </li>
            ))}
          </ul>
          <div className="mt-2 flex items-center gap-2">
            <select className="hs-input py-1.5 text-sm" value={choice} onChange={event => setChoice(event.target.value)} aria-label={t('homestation.prefs.muteChoose', 'Choose a person or contact group')} data-testid="hs-mute-choice">
              <option value="">{t('homestation.prefs.muteChoose', 'Choose a person or contact group')}</option>
              <optgroup label={t('homestation.prefs.people', 'People')}>
                {people.map(person => <option key={person.did} value={`person|${person.did}`}>{person.name}</option>)}
              </optgroup>
              <optgroup label={t('homestation.prefs.groups', 'Contact groups (from Message Center)')}>
                {groups.map(group => <option key={group.id} value={`group|${group.id}`}>{group.name}</option>)}
              </optgroup>
            </select>
            <button type="button" className="hs-btn" disabled={!choice} onClick={addMute} data-testid="hs-mute-add"><Plus size={13} />{t('homestation.prefs.muteAdd', 'Hide')}</button>
          </div>
        </section>

        <section>
          <h3 className="hs-section-title flex items-center gap-1.5"><Filter size={12} />{t('homestation.prefs.filterTitle', 'Content filters')}</h3>
          <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.prefs.filterNote', 'Rules use the effective tags your HomeStation keeps for each item: the author’s tags, your local classifier’s inferences and your corrections. Filtering never deletes, marks as read or touches bookmarks.')}</p>
          <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
            <span className="hs-badge is-warning" data-testid="hs-prefs-hidden">{t('homestation.filters.hiddenCount', '{{n}} hidden by your filters', { n: hidden.hiddenByRules })}</span>
            <button type="button" className="hs-badge is-accent" onClick={nav.showFilteredInFeed} data-testid="hs-prefs-show-filtered">{t('homestation.filters.viewFiltered', 'View filtered items in the feed')}</button>
          </div>
          <div className="mt-3 space-y-3">
            {rules.map(rule => <RuleEditor key={rule.id} rule={rule} />)}
          </div>
        </section>

        <section>
          <h3 className="hs-section-title">{t('homestation.prefs.defaultAudience', 'Default audience')}</h3>
          <p className="mt-1 text-xs leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.prefs.defaultAudienceNote', 'Used for new posts, comments and reposts. Likes are always public by default; bookmarks are always private by default. (Pending product decision; temporary default: Public.)')}</p>
          <div className="mt-2">
            <AudiencePicker idPrefix="hs-default" value={settings.defaultAudience} onChange={(value: AudienceSpec) => void store.setDefaultAudience(value)} />
          </div>
        </section>
        <div className="h-16" />
      </div>
    </div>
  )
}
