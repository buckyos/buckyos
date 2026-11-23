import { Checkbox } from '@mui/material'
import type { ReactNode } from 'react'
import type { PlannedRecord, RecordConflict, RecordResult } from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { CopyButton } from '../shared/CopyButton'
import { ListCard } from '../shared/Section'
import { codeText, type Tone } from './labels'
import { Mono, Muted, StatusLine } from './parts'

const line = (type: string, values: string[]) => values.map((value) => `${type} ${value}`)

function Part({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex gap-2 text-xs leading-5">
      <span className="w-14 shrink-0" style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="min-w-0 flex-1 space-y-0.5">{children}</span>
    </span>
  )
}

/** Records the user adds at their DNS provider, each part with its own copy button. */
export function RecordCards({ records }: { records: PlannedRecord[] }) {
  const { t } = useI18n()
  return (
    <div className="space-y-2">
      {records.map((record) => (
        <ListCard key={`${record.id}-${record.type}`} testId={`domain-record-${record.id}`}>
          <div className="space-y-1">
            <Part label={t('settings.domain.record.type')}><Mono>{record.type}</Mono></Part>
            <Part label={t('settings.domain.record.name')}><Mono>{record.name}</Mono></Part>
            <Part label={t('settings.domain.record.value')}>
              {record.values.map((value) => <span key={value} className="block"><Mono>{value}</Mono></span>)}
            </Part>
          </div>
          <div className="mt-2 flex flex-wrap gap-2">
            <CopyButton label={t('settings.domain.record.copyName')} text={() => record.name} testId={`domain-copy-name-${record.id}`} />
            <CopyButton label={t('settings.domain.record.copyValue')} text={() => record.values.join('\n')} testId={`domain-copy-value-${record.id}`} />
          </div>
        </ListCard>
      ))}
    </div>
  )
}

/** The records the node is going to write. */
export function RecordList({ records }: { records: PlannedRecord[] }) {
  return (
    <ul className="space-y-2">
      {records.map((record) => (
        <li key={`${record.id}-${record.type}`} className="flex flex-col text-xs leading-5" data-testid={`domain-plan-${record.id}`}>
          <Mono>{record.name}</Mono>
          {line(record.type, record.values).map((text) => (
            <code key={text} dir="ltr" className="break-all font-mono" style={{ color: 'var(--cp-muted)' }}>{text}</code>
          ))}
        </li>
      ))}
    </ul>
  )
}

function Change({ conflict }: { conflict: RecordConflict }) {
  const { t } = useI18n()
  const lines = [
    ['before', conflict.existing.flatMap((record) => line(record.type, record.values))],
    ['after', line(conflict.planned.type, conflict.planned.values)],
  ] as const
  return (
    <span className="block min-w-0 text-xs leading-5">
      <Mono>{conflict.name}</Mono>
      <span className="mt-1 block space-y-0.5">
        {lines.map(([key, values]) => (
          <Part key={key} label={t(`settings.domain.conflict.${key}`)}><Mono>{values.join(', ')}</Mono></Part>
        ))}
      </span>
    </span>
  )
}

/**
 * Names that already carry something else. With `onConfirm` each one is ticked before the node replaces it; without,
 * the user changes them at the DNS provider.
 */
export function ConflictList({ conflicts, confirmed, onConfirm }: {
  conflicts: RecordConflict[]
  confirmed?: string[]
  onConfirm?: (names: string[]) => void
}) {
  const { t } = useI18n()
  if (!conflicts.length) return null
  return (
    <div className="space-y-2" data-testid="domain-conflicts">
      <p className="text-sm leading-6" style={{ color: 'var(--cp-warning)' }}>{t(onConfirm ? 'settings.domain.conflict.confirm' : 'settings.domain.conflict.manual')}</p>
      {conflicts.map((conflict) => {
        if (!onConfirm) return <ListCard key={conflict.name}><Change conflict={conflict} /></ListCard>
        const checked = confirmed?.includes(conflict.name) ?? false
        return (
          <ListCard key={conflict.name} testId={`domain-conflict-${conflict.name}`}>
            <label className="flex cursor-pointer items-start gap-2">
              <Checkbox
                size="small"
                sx={{ p: 0.25 }}
                checked={checked}
                onChange={(event) => onConfirm(event.target.checked ? [...(confirmed ?? []), conflict.name] : (confirmed ?? []).filter((name) => name !== conflict.name))}
                slotProps={{ input: { 'aria-label': t('settings.domain.conflict.replace', undefined, { name: conflict.name }) } }}
              />
              <Change conflict={conflict} />
            </label>
          </ListCard>
        )
      })}
    </div>
  )
}

const RESULT_TONES: Record<RecordResult['status'], Tone> = { written: 'pass', unchanged: 'pass', needs_confirm: 'warn', failed: 'fail' }

export function RecordResults({ results }: { results: RecordResult[] }) {
  const { t } = useI18n()
  return (
    <div data-testid="domain-results">
      {results.map((result) => (
        <StatusLine
          key={`${result.name} ${result.type}`}
          tone={RESULT_TONES[result.status]}
          title={<Mono>{`${result.type} ${result.name}`}</Mono>}
          state={t(`settings.domain.result.${result.status}`)}
        >
          {result.code ? <Muted>{codeText(t, result.code)}</Muted> : null}
        </StatusLine>
      ))}
    </div>
  )
}
