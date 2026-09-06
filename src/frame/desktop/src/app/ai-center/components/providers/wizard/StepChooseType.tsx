import { CheckCircle2, CircleAlert, Cloud, Loader2, PlusCircle, RefreshCw, Server } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { ProviderSetupCatalog, ProviderType, ProviderView } from '../../../../../api/aicc_mgr'

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

function providerCounts(providers: ProviderView[], type: ProviderType): ProviderProfileCounts {
  return providers
    .filter((provider) => provider.config.provider_profile_id === type)
    .reduce<ProviderProfileCounts>((counts, provider) => {
      if (provider.config.enabled) {
        counts.enabled += 1
      } else {
        counts.disabled += 1
      }
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
  if (profiles.length === 0) return <div className="flex min-h-48 items-center justify-center text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.wizard.emptyCatalog', 'No built-in providers are available.')}</div>
  const choices = [...profiles.map((profile) => ({ type: profile.provider_profile_id, name: profile.provider_profile_id === 'sn' ? t('aiCenter.wizard.snRouter', 'SN Router') : profile.display_name, description: profile.base_url, systemManaged: profile.provider_profile_id === 'sn' })), { type: 'custom' as const, name: t('aiCenter.wizard.customProvider', 'Custom Provider'), description: t('aiCenter.wizard.customProviderHint', 'Connect a base URL by protocol family.'), systemManaged: false }]
  return <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">{choices.map((item) => {
    const active = selected === item.type
    const Icon = item.type === 'custom' ? Server : Cloud
    const counts = providerCounts(providers, item.type)
    const state = providerCardState(counts)
    const StateIcon = state === 'working' ? CheckCircle2 : state === 'pending_enable' ? CircleAlert : PlusCircle
    const stateColor = state === 'working' ? 'var(--cp-success)' : state === 'pending_enable' ? 'var(--cp-warning)' : 'var(--cp-muted)'
    const stateLabel = state === 'working'
      ? t('aiCenter.wizard.providerWorking', '工作中')
      : state === 'pending_enable'
        ? t('aiCenter.wizard.providerPendingEnable', '待启用')
        : t('aiCenter.wizard.providerPendingAdd', '待添加')
    return <button key={item.type} type="button" onClick={() => { void onSelect(item.type) }} className="flex min-h-32 flex-col gap-3 rounded-xl p-4 text-left transition-all" style={{ background: active ? 'color-mix(in oklch, var(--cp-accent), transparent 90%)' : 'var(--cp-surface)', border: active ? '2px solid var(--cp-accent)' : '1px solid var(--cp-border)' }}><div className="flex items-start justify-between gap-3"><div className="flex min-w-0 items-center gap-2"><Icon size={19} className="shrink-0" style={{ color: active ? 'var(--cp-accent)' : 'var(--cp-muted)' }} /><span className="min-w-0 truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{item.name}</span>{item.systemManaged && <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px]" style={{ background: 'var(--cp-accent)', color: '#fff' }}>{t('aiCenter.wizard.apiKey', 'API Key')}</span>}</div><StateIcon size={16} className="shrink-0" aria-label={stateLabel} style={{ color: stateColor }} /></div><span className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>{item.description}</span><div className="mt-auto flex items-center gap-3 text-[11px] font-medium"><span className="inline-flex items-center gap-1" aria-label={t('aiCenter.wizard.providerEnabledCount', '启用 {{count}}', { count: counts.enabled })} style={{ color: 'var(--cp-success)' }}><span className="size-2 rounded-full" style={{ background: 'var(--cp-success)' }} />{counts.enabled}</span><span className="inline-flex items-center gap-1" aria-label={t('aiCenter.wizard.providerDisabledCount', '禁用 {{count}}', { count: counts.disabled })} style={{ color: 'var(--cp-warning)' }}><span className="size-2 rounded-full" style={{ background: 'var(--cp-warning)' }} />{counts.disabled}</span></div></button>
  })}</div>
}
