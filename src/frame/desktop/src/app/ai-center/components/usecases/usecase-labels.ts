import { useI18n } from '../../../../i18n/provider'
import type { ModelMetadata } from '../../mock/types'
import type { OnUnavailable, UsecaseModelState, UsecaseRow } from '../../datamodel/usecases'
import type { toneColor } from '../policy/policy-labels'

type Tone = keyof typeof toneColor

/** The friendly name of a usecase; the id is shown beside it. */
export function useUsecaseName() {
  const { t } = useI18n()
  return (usecase: string) => t(`aiCenter.usecases.name.${usecase}`, usecase)
}

/** Which layer decided a cell: the built-in default, the Zone default, me, a routing group, the fallback LLM. */
export function useSourceLabel(groupNames: Record<string, string> = {}) {
  const { t } = useI18n()
  return (source: string | undefined) => {
    if (!source || source === 'default') return t('aiCenter.usecases.source.default', 'Default')
    if (source === 'zone') return t('aiCenter.usecases.source.zone', 'Zone default')
    if (source === 'user') return t('aiCenter.usecases.source.user', 'Me')
    if (source === 'fallback_llm') return t('aiCenter.usecases.source.fallbackLlm', 'Fallback LLM')
    if (source.startsWith('group:')) {
      const id = source.slice('group:'.length)
      return t('aiCenter.usecases.source.group', 'Group {{name}}', { name: groupNames[id] ?? id })
    }
    return source
  }
}

export function usePolicyLabel() {
  const { t } = useI18n()
  return (policy: OnUnavailable) => {
    switch (policy) {
      case 'none': return t('aiCenter.usecases.policy.none', 'Don’t switch')
      case 'same_model': return t('aiCenter.usecases.policy.same_model', 'Same model, another Provider')
      case 'not_pricier': return t('aiCenter.usecases.policy.not_pricier', 'Next model that is not pricier')
      case 'next_model': return t('aiCenter.usecases.policy.next_model', 'Next model')
    }
  }
}

export function usePolicyHint() {
  const { t } = useI18n()
  return (policy: OnUnavailable) => {
    switch (policy) {
      case 'none': return t('aiCenter.usecases.policyHint.none', 'Return the real error, with the Provider and the reason.')
      case 'same_model': return t('aiCenter.usecases.policyHint.same_model', 'Use the same model on another Provider. Useful when several Providers offer this model.')
      case 'not_pricier': return t('aiCenter.usecases.policyHint.not_pricier', 'Route again as if this Provider were disabled, and use the result only if it does not cost more. The system judges by known prices; models without a known price are never chosen as “not pricier”.')
      case 'next_model': return t('aiCenter.usecases.policyHint.next_model', 'Route again as if this Provider were disabled, and use the next model even if it costs more. The system judges by known prices; models without a known price are never chosen as “not pricier”.')
    }
  }
}

/** The "when unavailable" cell: the policy and whom it would switch to now, or why nobody. */
export function useFallbackText() {
  const { t } = useI18n()
  const policyLabel = usePolicyLabel()
  return (row: UsecaseRow): { policy: string; outcome?: string; warning: boolean } => {
    const policy = policyLabel(row.on_unavailable)
    if (row.on_unavailable === 'none' || !row.model) return { policy, warning: false }
    if (row.fallback_to) return { policy, outcome: t('aiCenter.usecases.switchesTo', 'would switch to {{model}}', { model: row.fallback_to }), warning: false }
    if (row.fallback_note === 'no_known_cheaper_model') return { policy, outcome: t('aiCenter.usecases.note.noCheaper', 'no model with a known price that is not higher'), warning: true }
    if (row.fallback_note === 'no_other_model') return { policy, outcome: t('aiCenter.usecases.note.noOther', 'no other model to switch to'), warning: true }
    if (row.fallback_note) return { policy, outcome: row.fallback_note, warning: true }
    return { policy, warning: false }
  }
}

export function useCoverageStatus() {
  const { t } = useI18n()
  return (row: UsecaseRow): { tone: Tone; label: string; detail?: string } => {
    if (row.enabled !== undefined) {
      return row.enabled
        ? { tone: 'ok', label: t('aiCenter.usecases.fallback.on', 'On') }
        : { tone: 'muted', label: t('aiCenter.usecases.fallback.off', 'Off') }
    }
    if (row.coverage === 'covered') return { tone: 'ok', label: t('aiCenter.usecases.status.covered', 'Covered') }
    if (row.coverage === 'fallback_llm') {
      return { tone: 'warning', label: t('aiCenter.usecases.status.fallbackLlm', 'Fallback LLM'), detail: t('aiCenter.usecases.status.fallbackLlmHint', 'No model of its own; the fallback LLM serves it and sessions show a warning.') }
    }
    return {
      tone: 'error',
      label: t('aiCenter.usecases.status.uncovered', 'Uncovered'),
      detail: row.coverage_reason === 'requirements_unmet'
        ? t('aiCenter.usecases.reason.requirementsUnmet', 'Models of this type are connected, but none meets what this usecase needs (for example context length or tool calls).')
        : t('aiCenter.usecases.reason.noModel', 'No connected Provider offers a model of this type.'),
    }
  }
}

export function useModelStateLabel() {
  const { t } = useI18n()
  return (state: UsecaseModelState) => {
    switch (state) {
      case 'missing': return t('aiCenter.usecases.state.missing', 'no longer offered by any Provider')
      case 'not_permitted': return t('aiCenter.usecases.state.notPermitted', 'you may not use it')
      case 'unsuitable': return t('aiCenter.usecases.state.unsuitable', 'does not meet this usecase')
      case 'available': return t('aiCenter.usecases.state.available', 'available')
    }
  }
}

/** A Provider's list price in USD per million tokens, the price `not_pricier` compares; unknown otherwise. */
export function useModelPriceText() {
  const { t } = useI18n()
  return (model: ModelMetadata): string | null => {
    const { input_token_usd: input, output_token_usd: output } = model.pricing
    if (input === undefined && output === undefined) return null
    const usd = (value: number | undefined) => (value === undefined ? '—' : `$${(value * 1_000_000).toFixed(2)}`)
    return t('aiCenter.usecases.price', 'In {{input}} · Out {{output}} / 1M tokens', { input: usd(input), output: usd(output) })
  }
}
