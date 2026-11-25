import type { DomainCheckItem, DomainCheckTarget } from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { CHECK_TONES, codeText, itemText } from './labels'
import { Details, Mono, Muted, StatusLine } from './parts'

function Values({ label, values }: { label: string; values: string[] }) {
  const { t } = useI18n()
  return (
    <div className="flex gap-2 text-xs leading-5">
      <span className="w-20 shrink-0" style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span className="min-w-0 flex-1 space-y-0.5">
        {values.length
          ? values.map((value) => <span key={value} className="block"><Mono>{value}</Mono></span>)
          : <span style={{ color: 'var(--cp-muted)' }}>{t('settings.domain.nothing')}</span>}
      </span>
    </div>
  )
}

function Compared({ expected, actual }: Pick<DomainCheckTarget, 'expected' | 'actual'>) {
  const { t } = useI18n()
  if (!expected.length && !actual.length) return null
  return (
    <div className="mt-1 space-y-0.5">
      {expected.length ? <Values label={t('settings.domain.expected')} values={expected} /> : null}
      <Values label={t('settings.domain.actual')} values={actual} />
    </div>
  )
}

function Target({ target }: { target: DomainCheckTarget }) {
  const { t } = useI18n()
  return (
    <div className="mt-2 text-xs leading-5">
      <Mono>{target.host}</Mono>
      {target.code ? <Muted>{codeText(t, target.code)}</Muted> : null}
      <Compared {...target} />
    </div>
  )
}

/** The detection items: what is expected, what was found and what to do; values are shown for what is not passed yet. */
export function DomainChecks({ items }: { items: DomainCheckItem[] }) {
  const { t } = useI18n()
  return (
    <div className="divide-y divide-[color:var(--cp-border)]" data-testid="domain-checks">
      {items.map((item) => {
        const open = item.status === 'waiting' || item.status === 'fail'
        const { text, detail } = itemText(t, item)
        const targets = open ? item.targets.filter((target) => target.status !== 'pass') : []
        return (
          <StatusLine
            key={item.id}
            tone={CHECK_TONES[item.status]}
            title={t(`settings.domain.item.${item.id}`, item.id)}
            state={t(`settings.domain.status.${item.status}`, item.status)}
            testId={`domain-check-${item.id}`}
          >
            {targets.length
              ? targets.map((target) => <Target key={target.host} target={target} />)
              : open ? (
                <>
                  {text ? <Muted>{text}</Muted> : null}
                  <Compared {...item} />
                </>
              ) : item.status === 'skipped' && text ? <Muted>{text}</Muted> : null}
            {open && detail ? <Details>{detail}</Details> : null}
          </StatusLine>
        )
      })}
    </div>
  )
}
