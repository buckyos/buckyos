import { zodResolver } from '@hookform/resolvers/zod'
import { EyeOff, Plus, UsersRound } from 'lucide-react'
import { useCallback, useState } from 'react'
import { Controller, useForm } from 'react-hook-form'
import { useI18n } from '../../../i18n/provider'
import { useNetworkDialogs } from '../../../components/network/dialogs'
import { filterRuleInputSchema, type FilterRuleInput } from '../datamodel/inputs'
import type { AudienceSpec, FilterRule, GenerationTag, MuteRule } from '../datamodel/types'
import type { HomeStationStore } from '../store/types'
import { useHsNav } from '../navContext'
import { AudiencePicker } from '../publish/AudiencePicker'
import { useHomeStationStore, useStoreSelector } from '../store/context'
import { ColumnBody, PageHeader } from '../ui/primitives'
import { useToast } from '../ui/toastContext'

const selectMutes = (store: HomeStationStore) => store.peekMuteRules()
const selectRules = (store: HomeStationStore) => store.peekFilterRules()
const selectGroups = (store: HomeStationStore) => store.peekGroups()
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
    <form className="rounded-xl p-4 text-[14px] leading-5" style={{ border: '1px solid var(--hs-line-strong)' }} onSubmit={save} data-testid="hs-rule-editor" data-rule={rule.id}>
      <label className="flex items-center gap-3 font-medium">
        <input type="checkbox" className="h-4 w-4" {...form.register('enabled')} data-testid="hs-rule-enabled" />
        {t('homestation.filters.ruleTitle', 'Hide items that are all of:')}
      </label>
      <Controller
        control={form.control}
        name="conditions"
        render={({ field }) => (
          <div className="mt-3 flex flex-wrap gap-2">
            {(['ai_full', 'ai_assisted', 'low_quality'] as GenerationTag[]).map(condition => (
              <button
                key={condition}
                type="button"
                className="hs-chip"
                aria-pressed={field.value.includes(condition)}
                onClick={() => field.onChange(field.value.includes(condition) ? field.value.filter(value => value !== condition) : [...field.value, condition])}
              >
                {conditionLabel(condition)}
              </button>
            ))}
          </div>
        )}
      />
      {error ? <p className="mt-2 text-[13px]" role="alert" style={{ color: 'var(--hs-danger-ink)' }}>{t(error, 'Choose at least one condition.')}</p> : null}
      <label className="mt-4 flex items-start gap-3 text-[13px]">
        <input type="checkbox" className="mt-0.5 h-4 w-4 flex-shrink-0" {...form.register('acceptInferred')} />
        {t('homestation.filters.acceptInferred', 'Accept your local model’s inferences (not only author declarations or your own corrections)')}
      </label>
      <label className="mt-3 flex flex-wrap items-center gap-3 text-[13px]">
        {t('homestation.filters.threshold', 'Minimum model score')}
        <Controller control={form.control} name="minConfidence" render={({ field }) => (
          <>
            <input type="range" min={0.5} max={0.99} step={0.01} value={field.value} onChange={event => field.onChange(Number(event.target.value))} aria-label={t('homestation.filters.threshold', 'Minimum model score')} />
            <span className="tabular-nums">{field.value.toFixed(2)}</span>
          </>
        )} />
      </label>
      <label className="mt-3 flex flex-wrap items-center gap-3 text-[13px]">
        {t('homestation.filters.unknown', 'When an item hasn’t been classified yet')}
        <select className="hs-input" style={{ minHeight: 32, paddingTop: 4, paddingBottom: 4, fontSize: 13 }} {...form.register('unknown')}>
          <option value="show">{t('homestation.filters.unknownShow', 'show it (unknown is not “human-made”)')}</option>
          <option value="hide">{t('homestation.filters.unknownHide', 'hide it until classified')}</option>
        </select>
      </label>
      <div className="mt-4 flex justify-end">
        <button type="submit" className="hs-btn is-primary is-small" disabled={!form.formState.isDirty} data-testid="hs-rule-save">{t('common.save', 'Save')}</button>
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
  const hidden = useStoreSelector(selectHidden)
  const settings = useStoreSelector(selectSettings)
  const dialogs = useNetworkDialogs()
  const [choice, setChoice] = useState('')

  const addGroupMute = () => {
    if (!choice) return
    void store.setMuteRule({ kind: 'group', groupId: choice, name: groups.find(group => group.id === choice)?.name ?? choice }, true)
    setChoice('')
  }
  const addPersonMute = async () => {
    const muted = mutes.flatMap(rule => rule.kind === 'person' ? [{ did: rule.did, reason: t('homestation.prefs.alreadyHidden', 'Already hidden') }] : [])
    const result = await dialogs.pickPeople({ title: t('homestation.prefs.mutePerson', 'Hide a person'), confirmLabel: t('homestation.prefs.muteAdd', 'Hide'), filter: { kinds: ['user', 'agent'], exclude: muted } })
    if (result.action !== 'selected') return
    for (const person of result.people) await store.setMuteRule({ kind: 'person', did: person.did, name: person.name } satisfies MuteRule, true)
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="hs-prefs">
      <PageHeader title={t('homestation.nav.prefs', 'Filters and “don’t show”')} onBack={nav.isDesktop ? undefined : nav.back} backLabel={t('common.back', 'Back')} />
      <ColumnBody className={nav.isDesktop ? 'space-y-8 px-6 py-5' : 'space-y-8 px-4 py-4'}>
        <p className="text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.prefs.subtitle', 'Private display rules. Nothing here is sent to anyone.')}</p>
        <section>
          <h3 className="hs-section-title">{t('homestation.prefs.muteTitle', 'Don’t show posts from')}</h3>
          <p className="hs-section-note">{t('homestation.prefs.muteNote', 'Applies to your main feed and the catch-up list. You keep following them and stay friends; they are not notified, and chat permissions don’t change.')}</p>
          <ul className="mt-3 space-y-2" data-testid="hs-mute-list">
            {mutes.length === 0 ? <li className="text-[13px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.prefs.muteEmpty', 'Nobody is hidden.')}</li> : null}
            {mutes.map(rule => (
              <li key={rule.kind === 'person' ? rule.did : rule.groupId} className="flex items-center gap-3 rounded-xl px-4 py-3 text-[15px] leading-[22px]" style={{ background: 'var(--hs-subtle-bg)' }}>
                <span style={{ color: 'var(--cp-muted)' }}>{rule.kind === 'group' ? <UsersRound size={16} /> : <EyeOff size={16} />}</span>
                <span className="min-w-0 flex-1 truncate">{rule.kind === 'group' ? t('homestation.prefs.muteGroupItem', 'Group “{{name}}”', { name: rule.name }) : rule.name}</span>
                <button type="button" className="hs-link text-[13px]" onClick={() => { void store.setMuteRule(rule, false); toast({ text: t('homestation.prefs.unmuted', 'Showing {{name}} again. Already-collected posts come back after filtering.', { name: rule.name }) }) }} data-testid="hs-unmute">
                  {t('homestation.prefs.unmute', 'Show again')}
                </button>
              </li>
            ))}
          </ul>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <button type="button" className="hs-btn" onClick={() => void addPersonMute()} data-testid="hs-mute-person"><Plus size={16} />{t('homestation.prefs.mutePerson', 'Hide a person')}</button>
            <select className="hs-input min-w-0 flex-1" value={choice} onChange={event => setChoice(event.target.value)} aria-label={t('homestation.prefs.muteGroupChoose', 'Choose a contact group')} data-testid="hs-mute-choice">
              <option value="">{t('homestation.prefs.muteGroupChoose', 'Choose a contact group')}</option>
              {groups.map(group => <option key={group.id} value={group.id}>{group.name}</option>)}
            </select>
            <button type="button" className="hs-btn" disabled={!choice} onClick={addGroupMute} data-testid="hs-mute-add"><Plus size={16} />{t('homestation.prefs.muteAdd', 'Hide')}</button>
          </div>
        </section>

        <section>
          <h3 className="hs-section-title">{t('homestation.prefs.filterTitle', 'Content filters')}</h3>
          <p className="hs-section-note">{t('homestation.prefs.filterNote', 'Rules use the effective tags your HomeStation keeps for each item: the author’s tags, your local classifier’s inferences and your corrections. Filtering never deletes, marks as read or touches bookmarks.')}</p>
          <div className="hs-meta mt-3">
            <span className="tabular-nums" data-testid="hs-prefs-hidden">{t('homestation.filters.hiddenCount', '{{n}} hidden by your filters', { n: hidden.hiddenByRules })}</span>
            <button type="button" className="hs-link" onClick={nav.showFilteredInFeed} data-testid="hs-prefs-show-filtered">{t('homestation.filters.viewFiltered', 'View filtered items in the feed')}</button>
          </div>
          <div className="mt-3 space-y-3">
            {rules.map(rule => <RuleEditor key={rule.id} rule={rule} />)}
          </div>
        </section>

        <section>
          <h3 className="hs-section-title">{t('homestation.prefs.defaultAudience', 'Default audience')}</h3>
          <p className="hs-section-note">{t('homestation.prefs.defaultAudienceNote', 'Used for new posts, comments and reposts. Likes are always public by default; bookmarks are always private by default. (Pending product decision; temporary default: Public.)')}</p>
          <div className="mt-3">
            <AudiencePicker idPrefix="hs-default" value={settings.defaultAudience} onChange={(value: AudienceSpec) => void store.setDefaultAudience(value)} />
          </div>
        </section>
      </ColumnBody>
    </div>
  )
}
