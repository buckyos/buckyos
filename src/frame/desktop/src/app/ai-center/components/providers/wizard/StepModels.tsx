import { CheckCircle2, CircleAlert, CircleDashed } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import type { KnownProviderProfile, WizardDraft } from '../../../../../api/aicc_mgr'

interface StepModelsProps {
  draft: WizardDraft
  profile: KnownProviderProfile | null
  onUpdate: (partial: Partial<WizardDraft>) => void
}

/**
 * Some Provider catalog APIs list models that the current account has not
 * activated, while other providers expose no discovery API. In both cases the
 * operator selects the models that should become routable.
 */
export function StepModels({ draft, profile, onUpdate }: StepModelsProps) {
  const { t } = useI18n()
  const models = profile?.selectable_inventory_models ?? []

  if (models.length === 0) {
    return (
      <div className="max-w-lg text-sm" style={{ color: 'var(--cp-muted)' }}>
        {t('aiCenter.wizard.noSelectableModels', 'This provider discovers its models automatically.')}
      </div>
    )
  }

  const allIds = models.map((model) => model.id)
  const selected = new Set(draft.selected_inventory_models ?? allIds)
  const selectedCount = allIds.filter((id) => selected.has(id)).length

  const applySelection = (next: Set<string>) => {
    onUpdate({ selected_inventory_models: allIds.filter((id) => next.has(id)) })
  }

  const toggle = (id: string) => {
    const next = new Set(selected)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    applySelection(next)
  }

  return (
    <div className="max-w-lg">
      <div
        className="rounded-xl p-4"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
              {t('aiCenter.wizard.selectModels', 'Models to publish')}
            </div>
            <p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t(
                'aiCenter.wizard.selectModelsHint',
                'The provider cannot reliably report account entitlement for these models. Only selected models become routable.',
              )}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={() => applySelection(new Set(allIds))}
              className="min-h-9 rounded-lg px-3 text-xs"
              style={{ border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
            >
              {t('common.selectAll', 'Select all')}
            </button>
            <button
              type="button"
              onClick={() => applySelection(new Set())}
              className="min-h-9 rounded-lg px-3 text-xs"
              style={{ border: '1px solid var(--cp-border)', color: 'var(--cp-muted)' }}
            >
              {t('common.clear', 'Clear')}
            </button>
          </div>
        </div>

        <ul className="mt-3 flex flex-col gap-1">
          {models.map((model) => {
            const checked = selected.has(model.id)
            return (
              <li key={model.id}>
                <button
                  type="button"
                  onClick={() => toggle(model.id)}
                  className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left"
                  style={{ border: '1px solid var(--cp-border)' }}
                >
                  {checked
                    ? <CheckCircle2 size={18} className="shrink-0" style={{ color: 'var(--cp-accent)' }} />
                    : <CircleDashed size={18} className="shrink-0" style={{ color: 'var(--cp-muted)' }} />}
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm" style={{ color: 'var(--cp-text)' }}>
                      {model.label || model.id}
                    </span>
                    <span className="block truncate text-xs" style={{ color: 'var(--cp-muted)' }}>
                      {model.id}
                    </span>
                  </span>
                </button>
              </li>
            )
          })}
        </ul>

        <div
          className="mt-3 flex items-center gap-2 text-xs"
          style={{ color: selectedCount === 0 ? 'var(--cp-warning)' : 'var(--cp-muted)' }}
        >
          {selectedCount === 0 && <CircleAlert size={14} className="shrink-0" />}
          {selectedCount === 0
            ? t('aiCenter.wizard.noModelSelected', 'No model selected: this provider will not be routable.')
            : t(
              'aiCenter.wizard.modelsSelected',
              '{{count}} of {{total}} models will join the inventory.',
              { count: selectedCount, total: allIds.length },
            )}
        </div>
      </div>
    </div>
  )
}
