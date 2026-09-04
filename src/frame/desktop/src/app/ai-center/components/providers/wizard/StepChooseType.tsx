import { Cloud, Loader2, RefreshCw, Server } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { ProviderSetupCatalog, ProviderType } from '../../../../../api/aicc_mgr'

interface StepChooseTypeProps {
  selected: ProviderType | null
  onSelect: (type: ProviderType) => void
  hasManagedSnProvider: boolean
  catalog: ProviderSetupCatalog | null
  loading: boolean
  error: string | null
  onRetry: () => void
}

export function StepChooseType({ selected, onSelect, hasManagedSnProvider, catalog, loading, error, onRetry }: StepChooseTypeProps) {
  const { t } = useI18n()
  if (loading) return <div className="flex min-h-48 items-center justify-center gap-2 text-sm" style={{ color: 'var(--cp-muted)' }}><Loader2 className="animate-spin" size={18} />{t('aiCenter.wizard.loadingCatalog', 'Loading provider catalog...')}</div>
  if (error) return <div className="flex min-h-48 flex-col items-center justify-center gap-3 text-sm" style={{ color: 'var(--cp-danger)' }}><span>{t('aiCenter.wizard.catalogFailed', 'Could not load provider catalog.')}</span><button type="button" onClick={onRetry} className="inline-flex min-h-11 items-center gap-2 rounded-lg px-4" style={{ border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}><RefreshCw size={16} />{t('common.retry', 'Retry')}</button></div>
  const profiles = catalog?.providers ?? []
  if (profiles.length === 0) return <div className="flex min-h-48 items-center justify-center text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.wizard.emptyCatalog', 'No built-in providers are available.')}</div>
  const choices = [...profiles.map((profile) => ({ type: profile.provider_profile_id, name: profile.display_name, description: profile.base_url, systemManaged: profile.provider_profile_id === 'sn' })), { type: 'custom' as const, name: t('aiCenter.wizard.customProvider', 'Custom Provider'), description: t('aiCenter.wizard.customProviderHint', 'Connect a base URL by protocol family.'), systemManaged: false }]
  return <div className="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">{choices.map((item) => {
    const active = selected === item.type
    const disabled = item.systemManaged && !hasManagedSnProvider
    const Icon = item.type === 'custom' ? Server : Cloud
    return <button key={item.type} type="button" onClick={() => onSelect(item.type)} disabled={disabled} className="flex min-h-28 flex-col gap-2 rounded-xl p-4 text-left transition-all disabled:cursor-not-allowed" style={{ background: active ? 'color-mix(in oklch, var(--cp-accent), transparent 90%)' : 'var(--cp-surface)', border: active ? '2px solid var(--cp-accent)' : '1px solid var(--cp-border)', opacity: disabled ? 0.65 : 1 }}><div className="flex items-center gap-2"><Icon size={19} style={{ color: active ? 'var(--cp-accent)' : 'var(--cp-muted)' }} /><span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{item.name}</span>{item.systemManaged && <span className="rounded px-1.5 py-0.5 text-[10px]" style={{ background: 'var(--cp-accent)', color: '#fff' }}>{hasManagedSnProvider ? t('aiCenter.wizard.systemManaged', 'System managed') : t('aiCenter.wizard.activationRequired', 'Activation required')}</span>}</div><span className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>{disabled ? t('aiCenter.wizard.snActivateFirst', 'Activate SN during BuckyOS setup to enable this system provider.') : item.description}</span></button>
  })}</div>
}
