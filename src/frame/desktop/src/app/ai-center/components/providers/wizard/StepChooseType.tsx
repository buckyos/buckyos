import { Cloud, Cpu, Globe, Network, Zap, Server } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { ProviderType } from '../../../mock/types'

const PROVIDER_TYPES: {
  type: ProviderType
  name: string
  desc: string
  icon: typeof Network
  recommended?: boolean
}[] = [
  { type: 'sn_router', name: 'SN Router', desc: 'System default AI router, suitable for most users', icon: Network, recommended: true },
  { type: 'openai', name: 'OpenAI', desc: 'GPT series models', icon: Zap },
  { type: 'anthropic', name: 'Anthropic', desc: 'Claude series models', icon: Cpu },
  { type: 'google', name: 'Google', desc: 'Gemini series models', icon: Globe },
  { type: 'openrouter', name: 'OpenRouter', desc: 'Multi-model aggregation router', icon: Cloud },
  { type: 'custom', name: 'Custom', desc: 'Custom API endpoint, supports OpenAI/Anthropic/Google protocol', icon: Server },
]

interface StepChooseTypeProps {
  selected: ProviderType | null
  onSelect: (type: ProviderType) => void
}

export function StepChooseType({ selected, onSelect }: StepChooseTypeProps) {
  const { t } = useI18n()

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
      {PROVIDER_TYPES.map((item) => {
        const active = selected === item.type
        return (
          <button
            key={item.type}
            type="button"
            onClick={() => onSelect(item.type)}
            className="flex flex-col gap-2 p-4 rounded-xl text-left transition-all"
            style={{
              background: active ? 'color-mix(in oklch, var(--cp-accent), transparent 90%)' : 'var(--cp-surface)',
              border: active ? '2px solid var(--cp-accent)' : '1px solid var(--cp-border)',
            }}
          >
            <div className="flex items-center gap-2">
              <item.icon size={20} style={{ color: active ? 'var(--cp-accent)' : 'var(--cp-muted)' }} />
              <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                {item.name}
              </span>
              {item.recommended && (
                <span
                  className="text-[10px] px-1.5 py-0.5 rounded font-medium"
                  style={{ background: 'var(--cp-accent)', color: '#fff' }}
                >
                  {t('aiCenter.wizard.recommended', 'Recommended')}
                </span>
              )}
            </div>
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {item.desc}
            </span>
          </button>
        )
      })}
    </div>
  )
}
