import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { AlertTriangle, LocateFixed } from 'lucide-react'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { EmptyView } from '../../components/empty-state/StateView'
import { StatusBadge } from '../../components/status/StatusBadge'
import { buildOpsDiagnostics, buildTechDiagnostics, getLogicalDirectoryWarnings, getResolverWarnings } from '../../datamodel/selectors'
import type { WarningRecord } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { formatDate, useShellContext } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm'

export function WarningsPage() {
  const { t } = useI18n()
  const { workspace, setServiceRole, serviceRole } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const navigate = useNavigate()
  const data = workspace.data!
  const [severity, setSeverity] = useState('')
  const [targetType, setTargetType] = useState('')
  const [search, setSearch] = useState('')
  const warnings = useMemo(() => {
    const merged = [
      ...data.warnings,
      ...buildTechDiagnostics(data),
      ...buildOpsDiagnostics(data),
      ...getResolverWarnings(data),
      ...getLogicalDirectoryWarnings(data),
    ]
    const byKey = new Map<string, WarningRecord>()
    merged.forEach((warning) => byKey.set(warning.warning_key, warning))
    return Array.from(byKey.values()).sort((a, b) => severityRank(b.severity) - severityRank(a.severity))
  }, [data])
  const filtered = useMemo(() => {
    return warnings.filter((warning) => {
      const severityMatch = !severity || warning.severity === severity
      const targetMatch = !targetType || warning.target_type === targetType
      const needle = search.trim().toLowerCase()
      const textMatch = !needle || [
        warning.warning_key,
        warning.target_type,
        warning.target_key,
        warning.message_key,
        warning.detail ?? '',
        t(warning.message_key, warning.message_key),
      ].some((value) => value.toLowerCase().includes(needle))
      return severityMatch && targetMatch && textMatch
    })
  }, [search, severity, targetType, t, warnings])
  const targetTypes = Array.from(new Set(warnings.map((warning) => warning.target_type))).sort()

  useEffect(() => {
    if (serviceRole !== 'ops') {
      setServiceRole('ops')
    }
  }, [serviceRole, setServiceRole])

  useEffect(() => {
    setInspector({
      title: t('warnings.title', 'Warnings'),
      subtitle: `${filtered.length} visible diagnostics`,
      status: warnings.some((warning) => warning.severity === 'blocked') ? t('status.blocked', 'Blocked') : t('status.warning', 'Warning'),
      json: filtered.slice(0, 8),
    })
  }, [filtered, setInspector, t, warnings])

  const columns = useMemo<Array<DataTableColumn<WarningRecord>>>(() => [
    {
      key: 'severity',
      title: t('warnings.severity', 'Severity'),
      render: (warning) => <StatusBadge tone={warning.severity === 'blocked' ? 'danger' : warning.severity === 'warning' ? 'warning' : 'neutral'}>{warning.severity}</StatusBadge>,
    },
    { key: 'message', title: t('table.summary', 'Summary'), render: (warning) => t(warning.message_key, warning.message_key) },
    { key: 'target', title: t('ops.target', 'Target'), render: (warning) => <span className="font-mono text-xs">{warning.target_type}:{warning.target_key}</span> },
    { key: 'detail', title: t('warnings.detail', 'Detail'), render: (warning) => warning.detail ?? '-' },
    { key: 'created', title: t('table.updated', 'Updated'), render: (warning) => formatDate(warning.created_at) },
    {
      key: 'jump',
      title: t('warnings.jump', 'Jump'),
      render: (warning) => (
        <button
          className="inline-flex items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-bold hover:border-[color:var(--cp-accent)]"
          onClick={(event) => {
            event.stopPropagation()
            navigate(getWarningPath(warning.target_type))
          }}
          type="button"
        >
          <LocateFixed size={14} />
          {t('warnings.locate', 'Locate')}
        </button>
      ),
    },
  ], [navigate, t])

  return (
    <div className="space-y-4" data-testid="warnings-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('warnings.title', 'Warnings')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{t('warnings.subtitle', 'Operations diagnostics with target navigation before publish.')}</p>
        </div>
        <StatusBadge tone={warnings.some((warning) => warning.severity === 'blocked') ? 'danger' : 'warning'}>{warnings.length}</StatusBadge>
      </header>

      <section className="grid gap-3 md:grid-cols-4">
        <Summary label={t('status.blocked', 'Blocked')} value={warnings.filter((warning) => warning.severity === 'blocked').length} tone="danger" />
        <Summary label={t('status.warning', 'Warning')} value={warnings.filter((warning) => warning.severity === 'warning').length} tone="warning" />
        <Summary label="Info" value={warnings.filter((warning) => warning.severity === 'info').length} tone="neutral" />
        <Summary label={t('top.pending', 'Pending changes')} value={data.pending_changes.length} tone="accent" />
      </section>

      <section className="shell-card grid gap-3 p-4 md:grid-cols-3">
        <label className="block text-sm font-semibold">
          <span className="mb-1 block text-[color:var(--cp-muted)]">{t('filter.search', 'Search')}</span>
          <input className={inputClass} value={search} onChange={(event) => setSearch(event.target.value)} />
        </label>
        <label className="block text-sm font-semibold">
          <span className="mb-1 block text-[color:var(--cp-muted)]">{t('warnings.severity', 'Severity')}</span>
          <select className={inputClass} value={severity} onChange={(event) => setSeverity(event.target.value)}>
            <option value="">{t('filter.all', 'All')}</option>
            <option value="blocked">{t('status.blocked', 'Blocked')}</option>
            <option value="warning">{t('status.warning', 'Warning')}</option>
            <option value="info">Info</option>
          </select>
        </label>
        <label className="block text-sm font-semibold">
          <span className="mb-1 block text-[color:var(--cp-muted)]">{t('ops.targetType', 'Target type')}</span>
          <select className={inputClass} value={targetType} onChange={(event) => setTargetType(event.target.value)}>
            <option value="">{t('filter.all', 'All')}</option>
            {targetTypes.map((item) => <option key={item} value={item}>{item}</option>)}
          </select>
        </label>
      </section>

      <section className="shell-card p-4">
        <div className="mb-3 flex items-center gap-2 text-sm font-bold">
          <AlertTriangle size={16} className="text-[color:var(--cp-warning)]" />
          {t('warnings.list', 'Diagnostics list')}
        </div>
        {filtered.length ? (
          <DataTable
            columns={columns}
            onSelect={(warning) => {
              setInspector({
                title: t(warning.message_key, warning.message_key),
                subtitle: `${warning.target_type}:${warning.target_key}`,
                status: warning.severity,
                json: warning,
              })
            }}
            rowKey={(warning) => warning.warning_key}
            rows={filtered}
          />
        ) : (
          <EmptyView />
        )}
      </section>
    </div>
  )
}

function Summary({ label, value, tone }: { label: string; value: number; tone: 'neutral' | 'success' | 'warning' | 'danger' | 'accent' }) {
  return (
    <div className="shell-card p-4 text-sm">
      <StatusBadge tone={tone}>{value}</StatusBadge>
      <div className="mt-2 text-[color:var(--cp-muted)]">{label}</div>
    </div>
  )
}

function severityRank(severity: WarningRecord['severity']) {
  if (severity === 'blocked') {
    return 3
  }
  if (severity === 'warning') {
    return 2
  }
  return 1
}

function getWarningPath(targetType: string) {
  if (targetType === 'provider') {
    return '/providers'
  }
  if (targetType === 'nick_rule') {
    return '/nick-rules'
  }
  if (targetType === 'logical_directory') {
    return '/logical-directory'
  }
  if (targetType === 'dictionary') {
    return '/dictionaries'
  }
  if (targetType === 'tech_source') {
    return '/tech-source'
  }
  if (targetType === 'variants' || targetType === 'version_rules') {
    return '/resolver-rules'
  }
  return '/models'
}
