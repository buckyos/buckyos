import { BrainCircuit, Cloud, Cpu, Globe, Network, Zap } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'

const providerPreviews = [
  { label: 'SN Router', icon: Network },
  { label: 'OpenAI', icon: Zap },
  { label: 'Anthropic', icon: Cpu },
  { label: 'Google', icon: Globe },
  { label: 'OpenRouter', icon: Cloud },
]

interface EnableAIGuideProps {
  onGetStarted: () => void
}

export function EnableAIGuide({ onGetStarted }: EnableAIGuideProps) {
  const { t } = useI18n()

  return (
    <div className="flex flex-col items-center justify-center py-16 px-4 text-center">
      <div className="mb-6" style={{ color: 'var(--cp-accent)' }}>
        <BrainCircuit size={64} strokeWidth={1.5} />
      </div>

      <h1
        className="text-xl font-semibold mb-3"
        style={{ color: 'var(--cp-text)' }}
      >
        {t('aiCenter.enable.title', 'AI Features Not Enabled')}
      </h1>

      <p
        className="text-sm max-w-md mb-8"
        style={{ color: 'var(--cp-muted)' }}
      >
        {t('aiCenter.enable.desc', 'Add at least one AI Provider to start using AI capabilities across your system.')}
      </p>

      <div className="flex items-center gap-3 mb-8">
        <button
          type="button"
          onClick={onGetStarted}
          className="px-6 py-2.5 rounded-lg text-sm font-medium transition-opacity hover:opacity-80"
          style={{ background: 'var(--cp-accent)', color: '#fff' }}
        >
          {t('aiCenter.enable.cta', 'Get Started')}
        </button>
        <button
          type="button"
          className="text-sm hover:underline"
          style={{ color: 'var(--cp-accent)' }}
        >
          {t('aiCenter.enable.learnMore', 'Learn about Providers')}
        </button>
      </div>

      <div className="flex flex-wrap justify-center gap-3">
        {providerPreviews.map((p) => (
          <div
            key={p.label}
            className="flex items-center gap-2 px-4 py-2.5 rounded-lg text-xs"
            style={{
              background: 'var(--cp-surface)',
              border: '1px solid var(--cp-border)',
              color: 'var(--cp-muted)',
            }}
          >
            <p.icon size={16} />
            <span>{p.label}</span>
          </div>
        ))}
      </div>
    </div>
  )
}
