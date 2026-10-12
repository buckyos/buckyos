import { CheckCircle2, CircleAlert, CircleDashed, Cloud, Loader2, RefreshCw, Server } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { KnownProviderProfile, ProviderSetupCatalog, ProviderType, ProviderView } from '../../../../../api/aicc_mgr'

interface StepChooseTypeProps {
  selected: ProviderType | null
  onSelect: (type: ProviderType) => void | Promise<void>
  providers: ProviderView[]
  catalog: ProviderSetupCatalog | null
  loading: boolean
  error: string | null
  onRetry: () => void
}

type ProviderCardState = 'pending_add' | 'pending_enable' | 'working'

interface ProviderProfileCounts {
  enabled: number
  disabled: number
}

interface ProviderChoice {
  id: string
  name: string
  profiles: KnownProviderProfile[]
}

function groupedChoices(profiles: KnownProviderProfile[]): ProviderChoice[] {
  const groups = new Map<string, ProviderChoice>()
  for (const profile of profiles) {
    const id = profile.setup_group?.id ?? profile.provider_profile_id
    const existing = groups.get(id)
    if (existing) existing.profiles.push(profile)
    else groups.set(id, { id, name: profile.setup_group?.display_name ?? profile.display_name, profiles: [profile] })
  }
  return [...groups.values()]
}

function defaultProfile(choice: ProviderChoice): KnownProviderProfile {
  return choice.profiles.find((profile) => profile.setup_group?.default) ?? choice.profiles[0]
}

function providerCounts(providers: ProviderView[], types: ProviderType[]): ProviderProfileCounts {
  const profileIds = new Set(types)
  return providers
    .filter((provider) => profileIds.has(provider.config.provider_profile_id))
    .reduce<ProviderProfileCounts>((counts, provider) => {
      if (provider.config.enabled) counts.enabled += 1
      else counts.disabled += 1
      return counts
    }, { enabled: 0, disabled: 0 })
}

function providerCardState(counts: ProviderProfileCounts): ProviderCardState {
  if (counts.enabled + counts.disabled === 0) return 'pending_add'
  if (counts.disabled > 0) return 'pending_enable'
  return 'working'
}

export function StepChooseType({ selected, onSelect, providers, catalog, loading, error, onRetry }: StepChooseTypeProps) {
  const { t } = useI18n()
  if (loading) return <div className="flex min-h-48 items-center justify-center gap-2 text-sm" style={{ color: 'var(--cp-muted)' }}><Loader2 className="animate-spin" size={18} />{t('aiCenter.wizard.loadingCatalog', 'Loading provider catalog...')}</div>
  if (error) return <div className="flex min-h-48 flex-col items-center justify-center gap-3 text-sm" style={{ color: 'var(--cp-danger)' }}><span>{t('aiCenter.wizard.catalogFailed', 'Could not load provider catalog.')}</span><button type="button" onClick={onRetry} className="inline-flex min-h-11 items-center gap-2 rounded-lg px-4" style={{ border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}><RefreshCw size={16} />{t('common.retry', 'Retry')}</button></div>
  const profiles = catalog?.providers ?? []
  if (profiles.length === 0) return <div className="flex min-h-48 items-center justify-center text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.wizard.emptyCatalog', 'No provider profiles are available.')}</div>
  const choices = groupedChoices(profiles)
  return <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
    {choices.map((choice) => {
      const selectedProfile = choice.profiles.find((profile) => profile.provider_profile_id === selected) ?? defaultProfile(choice)
      const active = choice.profiles.some((profile) => profile.provider_profile_id === selected)
      const counts = providerCounts(providers, choice.profiles.map((profile) => profile.provider_profile_id))
      const state = providerCardState(counts)
      const StateIcon = state === 'working' ? CheckCircle2 : state === 'pending_enable' ? CircleAlert : CircleDashed
      const stateColor = state === 'working' ? 'var(--cp-success)' : state === 'pending_enable' ? 'var(--cp-warning)' : 'var(--cp-muted)'
      const stateLabel = state === 'working'
        ? t('aiCenter.wizard.providerWorking', 'Working')
        : state === 'pending_enable'
          ? t('aiCenter.wizard.providerPendingEnable', 'Pending enable')
          : t('aiCenter.wizard.providerPendingAdd', 'Not added')
      return <div key={choice.id} className="flex min-h-32 flex-col gap-3 rounded-xl p-4 text-left transition-all" style={{ background: active ? 'color-mix(in oklch, var(--cp-accent), transparent 90%)' : 'var(--cp-surface)', border: active ? '2px solid var(--cp-accent)' : '1px solid var(--cp-border)' }}>
        <button type="button" onClick={() => { void onSelect(selectedProfile.provider_profile_id) }} className="flex items-start justify-between gap-3 text-left">
          <div className="flex min-w-0 items-center gap-2"><Cloud size={19} className="shrink-0" style={{ color: active ? 'var(--cp-accent)' : 'var(--cp-muted)' }} /><span className="min-w-0 truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{choice.name}</span>{selectedProfile.provider_profile_id === 'sn' && <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px]" style={{ background: 'var(--cp-accent)', color: '#fff' }}>{t('aiCenter.wizard.apiKey', 'API Key')}</span>}</div>
          <StateIcon size={16} className="shrink-0" aria-label={stateLabel} style={{ color: stateColor }} />
        </button>
        {choice.profiles.length > 1 && <label className="flex items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
          <span className="shrink-0">{t('aiCenter.wizard.accountType', 'Account type')}</span>
          <select value={selectedProfile.provider_profile_id} onChange={(event) => { void onSelect(event.target.value) }} className="min-h-9 min-w-0 flex-1 rounded-md px-2" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
            {choice.profiles.map((profile) => <option key={profile.provider_profile_id} value={profile.provider_profile_id}>{profile.setup_group?.account_type_label ?? profile.display_name}</option>)}
          </select>
        </label>}
        <span className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>{selectedProfile.base_url}</span>
        <div className="mt-auto flex items-center gap-3 text-[11px] font-medium"><span className="inline-flex items-center gap-1" aria-label={t('aiCenter.wizard.providerEnabledCount', 'Enabled: {{count}}', { count: counts.enabled })} style={{ color: 'var(--cp-success)' }}><span className="size-2 rounded-full" style={{ background: 'var(--cp-success)' }} />{counts.enabled}</span><span className="inline-flex items-center gap-1" aria-label={t('aiCenter.wizard.providerDisabledCount', 'Disabled: {{count}}', { count: counts.disabled })} style={{ color: 'var(--cp-warning)' }}><span className="size-2 rounded-full" style={{ background: 'var(--cp-warning)' }} />{counts.disabled}</span></div>
      </div>
    })}
    <button type="button" onClick={() => { void onSelect('custom') }} className="flex min-h-32 flex-col gap-3 rounded-xl p-4 text-left transition-all" style={{ background: selected === 'custom' ? 'color-mix(in oklch, var(--cp-accent), transparent 90%)' : 'var(--cp-surface)', border: selected === 'custom' ? '2px solid var(--cp-accent)' : '1px solid var(--cp-border)' }}><div className="flex items-center gap-2"><Server size={19} style={{ color: selected === 'custom' ? 'var(--cp-accent)' : 'var(--cp-muted)' }} /><span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.wizard.customProvider', 'Custom Provider')}</span></div><span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.wizard.customProviderHint', 'Connect an OpenAI- or Anthropic-compatible endpoint.')}</span></button>
  </div>
}
