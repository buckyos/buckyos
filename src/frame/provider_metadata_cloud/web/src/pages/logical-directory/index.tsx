import { useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { FolderPlus, Search, ShieldAlert } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { EmptyView } from '../../components/empty-state/StateView'
import { StatusBadge } from '../../components/status/StatusBadge'
import {
  getDirectoryBreadcrumbs,
  getLogicalDirectoryWarnings,
  getMaterializedLogicalDirectories,
  materializeDirectoryItems,
  searchLogicalDirectory,
} from '../../datamodel/selectors'
import { logicalDirectoryInputSchema, type LogicalDirectoryInput } from '../../datamodel/schemas'
import type { LogicalDirectoryRecord, ModelParamRuleRecord } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

type SelectedItem =
  | { kind: 'directory'; item: LogicalDirectoryRecord }
  | { kind: 'model'; item: ModelParamRuleRecord }

export function LogicalDirectoryPage() {
  const { t } = useI18n()
  const { workspace, viewMode, upsertLogicalDirectory } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const directories = useMemo(() => getMaterializedLogicalDirectories(data), [data])
  const rootDirectory = directories.find((directory) => directory.path === '/') ?? directories[0]
  const [currentDirectoryKey, setCurrentDirectoryKey] = useState(rootDirectory?.directory_key ?? '')
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<SelectedItem | null>(rootDirectory ? { kind: 'directory', item: rootDirectory } : null)
  const [formOpen, setFormOpen] = useState(false)
  const currentDirectory = directories.find((directory) => directory.directory_key === currentDirectoryKey) ?? rootDirectory
  const isSearchMode = search.trim().length > 0
  const searchResults = useMemo(() => searchLogicalDirectory(data, search), [data, search])
  const directoryItems = useMemo(() => currentDirectory ? materializeDirectoryItems(data, currentDirectory) : { childDirectories: [], models: [] }, [currentDirectory, data])
  const breadcrumbs = useMemo(() => currentDirectory ? getDirectoryBreadcrumbs(data, currentDirectory.directory_key) : [], [currentDirectory, data])
  const warnings = useMemo(() => getLogicalDirectoryWarnings(data), [data])
  const directoryForm = useForm<LogicalDirectoryInput>({
    resolver: zodResolver(logicalDirectoryInputSchema),
    defaultValues: {
      directory_key: 'round3-research',
      path: '/research',
      title: 'Research',
      parent_key: currentDirectory?.directory_key ?? '',
    },
  })

  const itemColumns = useMemo<Array<DataTableColumn<SelectedItem>>>(() => [
    { key: 'kind', title: t('logical.itemType', 'Type'), render: (row) => <StatusBadge tone={row.kind === 'directory' ? 'accent' : 'success'}>{row.kind}</StatusBadge> },
    { key: 'key', title: t('rules.ruleKey', 'Rule key'), render: (row) => <span className="font-mono text-xs">{row.kind === 'directory' ? row.item.directory_key : row.item.rule_key}</span> },
    { key: 'title', title: t('table.modelId', 'Model selector'), render: (row) => row.kind === 'directory' ? row.item.path : <span className="font-mono text-xs">{row.item.model_id_selector}</span> },
    { key: 'scope', title: t('table.provider', 'Provider'), render: (row) => row.kind === 'directory' ? '-' : row.item.provider_key ?? row.item.original_provider ?? 'global' },
  ], [t])

  const tableRows: SelectedItem[] = isSearchMode
    ? searchResults
    : [
        ...directoryItems.childDirectories.map((item) => ({ kind: 'directory' as const, item })),
        ...directoryItems.models.map((item) => ({ kind: 'model' as const, item })),
      ]

  const selectItem = (item: SelectedItem) => {
    setSelected(item)
    if (item.kind === 'directory') {
      setCurrentDirectoryKey(item.item.directory_key)
      setSearch('')
      directoryForm.setValue('parent_key', item.item.directory_key)
    }
    setInspector({
      title: item.kind === 'directory' ? item.item.path : item.item.model_id_selector ?? item.item.rule_key,
      subtitle: item.kind,
      status: item.kind === 'directory' ? `${item.item.model_rule_keys.length} mounts` : item.item.match_type,
      json: item.item,
    })
  }

  return (
    <div className="space-y-4" data-testid="logical-directory-page">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('logical.title', 'Logical Directory')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')}</p>
        </div>
        <div className="flex items-center gap-2">
          <StatusBadge tone={warnings.length ? 'warning' : 'success'}>{warnings.length}</StatusBadge>
          <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={() => setFormOpen((value) => !value)}>
            <FolderPlus size={16} />
            {t('logical.createDirectory', 'Create directory')}
          </button>
        </div>
      </header>

      <section className="shell-card grid gap-3 p-4 lg:grid-cols-[minmax(0,1fr)_auto]">
        <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
          {t('filter.search', 'Search')}
          <span className="relative">
            <Search className="pointer-events-none absolute left-3 top-2.5 text-[color:var(--cp-muted)]" size={16} />
            <input
              className={`${inputClass} pl-9`}
              placeholder={t('logical.searchPlaceholder', 'Search directories and mounted models')}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </span>
        </label>
        <div className="flex items-end">
          <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" disabled={!isSearchMode} onClick={() => setSearch('')} type="button">
            {t('logical.pathBrowse', 'Path browse')}
          </button>
        </div>
        <div className="flex flex-wrap items-center gap-1 text-xs text-[color:var(--cp-muted)] lg:col-span-2">
          {isSearchMode
            ? t('logical.searchMode', 'Search result mode is active. Path browsing is suspended until search is cleared.')
            : breadcrumbs.map((directory, index) => <span className="flex items-center gap-1" key={directory.directory_key}>{index > 0 && <span>/</span>}<button className="rounded px-1 font-mono hover:bg-[color:var(--cp-surface-2)] hover:text-[color:var(--cp-accent)]" onClick={() => selectItem({ kind: 'directory', item: directory })} type="button">{index === 0 ? 'Root' : directory.path.split('/').filter(Boolean).at(-1)}</button></span>)}
        </div>
      </section>

      {formOpen && (
        <form
          className="shell-card grid gap-3 p-4 md:grid-cols-5"
          onSubmit={directoryForm.handleSubmit(async (value) => {
            await upsertLogicalDirectory(value)
            setFormOpen(false)
          })}
        >
          <Field label={t('logical.directoryKey', 'Directory key')} error={directoryForm.formState.errors.directory_key?.message}>
            <input className={inputClass} {...directoryForm.register('directory_key')} />
          </Field>
          <Field label={t('logical.path', 'Path')} error={directoryForm.formState.errors.path?.message}>
            <input className={inputClass} {...directoryForm.register('path')} />
          </Field>
          <Field label={t('logical.titleField', 'Title')}>
            <input className={inputClass} {...directoryForm.register('title')} />
          </Field>
          <Field label={t('logical.parent', 'Parent')}>
            <select className={inputClass} {...directoryForm.register('parent_key')}>
              <option value="">{t('models.global', 'Global / origin')}</option>
              {directories.map((directory) => <option key={directory.directory_key} value={directory.directory_key}>{directory.path}</option>)}
            </select>
          </Field>
          <div className="flex items-end justify-end gap-2">
            <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="button" onClick={() => setFormOpen(false)}>{t('action.discard', 'Discard')}</button>
            <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
          </div>
        </form>
      )}

      <section className="grid gap-4 xl:grid-cols-[280px_minmax(0,1fr)_320px]">
        <aside className="shell-card p-4">
          <h2 className="mb-3 text-sm font-bold">{t('logical.tree', 'Directory tree')}</h2>
          <div className="space-y-1">
            {directories.map((directory) => (
              <button
                className={`block w-full rounded-md px-3 py-2 text-left text-sm ${directory.directory_key === currentDirectory?.directory_key ? 'bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'hover:bg-[color:var(--cp-surface-2)]'}`}
                style={{ paddingLeft: `${12 + Math.max(0, directory.path.split('/').filter(Boolean).length) * 12}px` }}
                key={directory.directory_key}
                onClick={() => selectItem({ kind: 'directory', item: directory })}
                type="button"
              >
                <span className="block font-semibold">{directory.title}</span>
                <span className="font-mono text-xs text-[color:var(--cp-muted)]">{directory.path}</span>
              </button>
            ))}
          </div>
        </aside>

        <section className="space-y-3">
          {tableRows.length ? (
            <DataTable columns={itemColumns} onSelect={selectItem} rowKey={(row) => row.kind === 'directory' ? row.item.directory_key : row.item.rule_key} rows={tableRows} />
          ) : (
            <EmptyView />
          )}
        </section>

        <aside className="shell-card p-4">
          <h2 className="mb-1 text-sm font-bold">{t('inspector.title', 'Inspector')}</h2>
          <p className="mb-3 text-xs text-[color:var(--cp-muted)]">{t('logical.inspectorHint', 'Details of the selected directory or directly mounted rule.')}</p>
          {selected ? (
            <div className="space-y-3 text-sm">
              <Fact label={t('logical.itemType', 'Type')} value={selected.kind} />
              <Fact label={t('rules.ruleKey', 'Rule key')} value={selected.kind === 'directory' ? selected.item.directory_key : selected.item.rule_key} />
              <Fact label={t('logical.path', 'Path')} value={selected.kind === 'directory' ? selected.item.path : selected.item.logical_mounts.join(', ') || '-'} />
            </div>
          ) : (
            <EmptyView />
          )}
        </aside>
      </section>

      <section className="shell-card p-4">
        <h2 className="mb-3 flex items-center gap-2 text-sm font-bold"><ShieldAlert size={16} />{t('logical.risks', 'Logical directory risk checks')}</h2>
        <div className="grid gap-2 md:grid-cols-2">
          {warnings.map((warning) => (
            <button
              className="rounded-md border border-[color:var(--cp-border)] p-3 text-left text-sm hover:border-[color:var(--cp-accent)]"
              key={warning.warning_key}
              onClick={() => {
                const directory = data.logical_directories.find((item) => item.directory_key === warning.target_key)
                if (directory) {
                  selectItem({ kind: 'directory', item: directory })
                }
              }}
              type="button"
            >
              <StatusBadge tone={warning.severity === 'blocked' ? 'danger' : 'warning'}>{warning.severity}</StatusBadge>
              <div className="mt-2">{t(warning.message_key, warning.message_key)}</div>
              {warning.detail && <div className="mt-1 font-mono text-xs text-[color:var(--cp-muted)]">{warning.detail}</div>}
            </button>
          ))}
          {!warnings.length && <div className="text-sm text-[color:var(--cp-muted)]">{t('status.published', 'Published')}</div>}
        </div>
      </section>
    </div>
  )
}

function Fact({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="rounded-md border border-[color:var(--cp-border)] px-3 py-2">
      <div className="text-xs text-[color:var(--cp-muted)]">{label}</div>
      <div className="mt-1 break-words font-mono text-xs">{value}</div>
    </div>
  )
}

function Field({ label, error, children }: { label: string; error?: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
      {label}
      {children}
      {error && <span className="text-[color:var(--cp-danger)]">{error}</span>}
    </label>
  )
}
