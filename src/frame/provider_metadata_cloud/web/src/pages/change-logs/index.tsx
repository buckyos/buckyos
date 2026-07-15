import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { StatusBadge } from '../../components/status/StatusBadge'
import type { ChangeLogRecord } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { formatDate, useShellContext } from '../pageUtils'

export function ChangeLogsPage() {
  const { t } = useI18n()
  const { workspace, serviceRole } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const rows = workspace.data!.change_logs.filter((log) => log.service_role === serviceRole)
  const columns: Array<DataTableColumn<ChangeLogRecord>> = [
    { key: 'revision', title: t('table.revision', 'Revision'), render: (log) => <StatusBadge>{log.to_revision}</StatusBadge> },
    { key: 'summary', title: t('table.summary', 'Summary'), render: (log) => log.summary },
    { key: 'operator', title: t('table.operator', 'Operator'), render: (log) => log.operator_id },
    { key: 'updated', title: t('table.updated', 'Updated'), render: (log) => formatDate(log.created_at) },
  ]

  return (
    <div className="space-y-4" data-testid="change-logs-page">
      <header>
        <h1 className="text-2xl font-bold">{t('logs.title', 'Change Logs')}</h1>
        <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{rows.length} records</p>
      </header>
      <DataTable
        columns={columns}
        onSelect={(log) => {
          setInspector({
            title: log.change_id,
            subtitle: log.summary,
            status: log.to_revision,
            json: log,
          })
        }}
        rowKey={(log) => log.change_id}
        rows={rows}
      />
    </div>
  )
}
