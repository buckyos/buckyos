import type { ReactNode } from 'react'
import { ArrowRight } from 'lucide-react'
import { useI18n } from '../../../../../i18n/provider'
import { summarizeCoverage, type UsecaseCoverageChange } from '../../../datamodel/usecases'
import { ExactModelLabel, StatusDot } from '../../policy/PolicyWidgets'
import { panelStyle } from '../../policy/policy-labels'
import { useUsecaseName } from '../../usecases/usecase-labels'

const muted = { color: 'var(--cp-muted)' }

function Section({ title, count, testId, children }: { title: string; count: number; testId: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2 rounded-xl p-4" style={panelStyle} data-testid={testId} data-count={count}>
      <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{title} <span className="font-normal tabular-nums" style={muted}>{count}</span></h3>
      {children}
    </section>
  )
}

function UsecaseLabel({ usecase }: { usecase: string }) {
  const name = useUsecaseName()
  return (
    <span className="flex min-w-0 flex-col">
      <span className="text-sm" style={{ color: 'var(--cp-text)' }}>{name(usecase)}</span>
      <span className="truncate font-mono text-xs" style={muted}>{usecase}</span>
    </span>
  )
}

/** What adding the Provider does to every usecase of the Zone: newly covered, a changed default model (each can be
 * kept as it is, which writes the old model into the Zone default), still uncovered. */
export function StepCoverage({ coverage, keep, canKeep, onKeepChange }: {
  coverage: UsecaseCoverageChange[]
  /** Usecase → the old model to keep. */
  keep: Record<string, string>
  /** Administrators can keep an old model (it is written into the Zone default). */
  canKeep: boolean
  onKeepChange: (change: UsecaseCoverageChange, keepOld: boolean) => void
}) {
  const { t } = useI18n()
  const name = useUsecaseName()
  const summary = summarizeCoverage(coverage)
  if (coverage.length === 0) {
    return <p className="max-w-lg text-sm" data-testid="wizard-coverage" style={muted}>{t('aiCenter.wizard.coverage.none', 'The validation did not report which usecases this Provider covers.')}</p>
  }
  return (
    <div className="flex max-w-2xl flex-col gap-4" data-testid="wizard-coverage">
      <p className="text-sm leading-6" style={{ color: 'var(--cp-text)' }}>
        {t('aiCenter.wizard.coverage.intro', 'Adding this Provider changes which model each usecase uses for everyone in the Zone. The aim is that every usecase has a model.')}
      </p>
      {summary.newlyCovered.length > 0 && (
        <Section title={t('aiCenter.wizard.coverage.newly', 'Newly covered')} count={summary.newlyCovered.length} testId="coverage-newly">
          {summary.newlyCovered.map((change) => (
            <div key={change.usecase} className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1" data-testid="coverage-row" data-usecase={change.usecase}>
              <UsecaseLabel usecase={change.usecase} />
              <span className="min-w-0 text-xs">{change.after && <ExactModelLabel exactModel={change.after} />}</span>
            </div>
          ))}
        </Section>
      )}
      {summary.changed.length > 0 && (
        <Section title={t('aiCenter.wizard.coverage.changed', 'Default model changes')} count={summary.changed.length} testId="coverage-changed">
          <p className="text-xs leading-5" style={muted}>
            {canKeep
              ? t('aiCenter.wizard.coverage.changedHint', 'Uncheck a usecase to keep its current model; it is then saved in the Zone default.')
              : t('aiCenter.wizard.coverage.changedReadonly', 'An administrator can keep the current model of these usecases.')}
          </p>
          {summary.changed.map((change) => {
            const kept = keep[change.usecase] !== undefined
            return (
              <label key={change.usecase} className="flex min-h-11 items-start gap-3 rounded-lg p-2" data-testid="coverage-row" data-usecase={change.usecase} style={{ background: 'var(--cp-bg)' }}>
                <input
                  type="checkbox"
                  data-testid="coverage-use-new"
                  className="mt-1 shrink-0"
                  checked={!kept}
                  disabled={!canKeep}
                  onChange={(event) => onKeepChange(change, !event.target.checked)}
                  style={{ accentColor: 'var(--cp-accent)' }}
                />
                <span className="flex min-w-0 flex-1 flex-col gap-1">
                  <UsecaseLabel usecase={change.usecase} />
                  <span className="flex min-w-0 flex-wrap items-center gap-1 text-xs" style={{ color: 'var(--cp-text)' }}>
                    {change.before && <ExactModelLabel exactModel={change.before} />}
                    <ArrowRight size={12} style={muted} />
                    {change.after && <ExactModelLabel exactModel={change.after} />}
                  </span>
                  <span className="text-xs" style={{ color: kept ? 'var(--cp-warning)' : 'var(--cp-muted)' }}>
                    {kept
                      ? t('aiCenter.wizard.coverage.keeps', 'Keeps {{model}} (saved in the Zone default)', { model: change.before ?? '' })
                      : t('aiCenter.wizard.coverage.switches', 'Switches to the new model')}
                  </span>
                </span>
              </label>
            )
          })}
        </Section>
      )}
      {summary.stillUncovered.length > 0 && (
        <Section title={t('aiCenter.wizard.coverage.uncovered', 'Still without a model')} count={summary.stillUncovered.length} testId="coverage-uncovered">
          {summary.stillUncovered.some((change) => change.after_coverage === 'fallback_llm') && (
            <StatusDot tone="warning" label={t('aiCenter.usecases.status.fallbackLlmHint', 'No model of its own; the fallback LLM serves it and sessions show a warning.')} />
          )}
          <div className="flex flex-wrap gap-1.5">
            {summary.stillUncovered.map((change) => (
              <span
                key={change.usecase}
                data-testid="coverage-row"
                data-usecase={change.usecase}
                data-coverage={change.after_coverage}
                title={change.usecase}
                className="inline-flex min-h-8 items-center gap-1.5 rounded-md px-2 text-xs"
                style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)' }}
              >
                <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: change.after_coverage === 'fallback_llm' ? 'var(--cp-warning)' : 'var(--cp-danger)' }} />
                {name(change.usecase)}
              </span>
            ))}
          </div>
        </Section>
      )}
      {summary.unchanged > 0 && (
        <p className="text-xs" style={muted}>{t('aiCenter.wizard.coverage.unchanged', 'Other usecases keep their model: {{count}}', { count: summary.unchanged })}</p>
      )}
    </div>
  )
}
