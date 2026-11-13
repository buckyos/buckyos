import { useEffect, useMemo, useRef, useState } from 'react'
import { Button, Dialog, DialogActions, DialogContent, DialogTitle, Pagination, TextField } from '@mui/material'
import { ChevronRight, ExternalLink } from 'lucide-react'
import useSWR from 'swr'
import { useI18n } from '../../../i18n/provider'
import { InfoRow, Section, SectionNote } from './shared/Section'
import { groupSoftwareProjects, type SoftwarePackage, type SoftwareProject } from '../data/open-source-projects'

interface SoftwareScope {
  id: string
  label: string
  status: 'complete' | 'unavailable' | 'partial'
  isSnapshot?: boolean
}

interface SoftwareCatalog {
  schemaVersion: number
  scopes: SoftwareScope[]
  packages: SoftwarePackage[]
}

interface LicenseDetails {
  files: { name: string; text: string }[]
}

const catalogUrl = `${import.meta.env.BASE_URL}open-source/index.json`
const fetchOptions = { revalidateOnFocus: false, shouldRetryOnError: false }
const pageSize = 50

function scopeLabel(scope: SoftwareScope, t: ReturnType<typeof useI18n>['t']): string {
  if (scope.id.startsWith('ext-core-')) {
    return t('settings.openSource.scope.extCoreTarget', 'Extension tools ({{target}})', { target: scope.id.slice('ext-core-'.length) })
  }
  return t(`settings.openSource.scope.${scope.id}`, scope.label)
}

async function fetchJson<T>(url: string): Promise<T> {
  const response = await fetch(url, { cache: 'no-cache' })
  if (!response.ok) throw new Error(`HTTP ${response.status}`)
  return response.json() as Promise<T>
}

async function fetchCatalog(url: string): Promise<SoftwareCatalog> {
  const catalog = await fetchJson<SoftwareCatalog>(url)
  if (catalog.schemaVersion !== 1 || !Array.isArray(catalog.packages) || !Array.isArray(catalog.scopes)) {
    throw new Error('Invalid software catalog')
  }
  return catalog
}

function webUrl(value: string): string | null {
  try {
    const url = new URL(value)
    return ['https:', 'http:'].includes(url.protocol) ? url.href : null
  } catch {
    return null
  }
}

function SoftwareDetails({ software, scopes }: { software: SoftwarePackage; scopes: SoftwareScope[] }) {
  const { t } = useI18n()
  const { data, error, mutate } = useSWR(
    software.licenseFileCount > 0 ? `${import.meta.env.BASE_URL}open-source/licenses/${software.id[0]}.json` : null,
    fetchJson<Record<string, LicenseDetails>>,
    fetchOptions,
  )
  const files = data?.[software.id]?.files
  const links = [
    { label: t('settings.openSource.homepage', 'Project website'), href: webUrl(software.homepage) },
    { label: t('settings.openSource.repository', 'Source code'), href: webUrl(software.repository) },
  ].filter((link) => link.href)

  return (
    <>
      {software.description && <p className="mb-3 text-sm leading-6 text-[color:var(--cp-muted)]">{software.description}</p>}
      <InfoRow label={t('settings.openSource.version', 'Version')} value={software.version} />
      {software.revision && <InfoRow label={t('settings.openSource.revision', 'Source revision')} value={<span className="break-all font-mono text-xs">{software.revision}</span>} />}
      <InfoRow label={t('settings.openSource.license', 'License')} value={software.license || t('settings.openSource.licenseUnknown', 'License not provided')} />
      <InfoRow label={t('settings.openSource.ecosystem', 'Package type')} value={software.ecosystem} />
      <InfoRow
        label={t('settings.openSource.usedBy', 'Used by')}
        value={software.scopes.map((id) => scopeLabel(scopes.find((scope) => scope.id === id) ?? { id, label: id, status: 'complete' }, t)).join(' · ')}
      />
      {links.length > 0 && (
        <div className="my-3 flex flex-wrap gap-3">
          {links.map((link) => (
            <a key={link.label} href={link.href!} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-sm text-[color:var(--cp-accent)] underline underline-offset-2">
              {link.label}<ExternalLink size={13} aria-hidden="true" />
            </a>
          ))}
        </div>
      )}
      <h3 className="mb-2 mt-5 text-sm font-semibold">{t('settings.openSource.licenseText', 'License text & notices')}</h3>
      {software.licenseFileCount === 0 ? (
        <SectionNote>{t('settings.openSource.noLicenseText', 'This package does not include a license text in the catalog. Consult its project source for the full terms.')}</SectionNote>
      ) : error || (data && !files) ? (
        <div className="space-y-2">
          <SectionNote tone="danger">{t('settings.openSource.licenseLoadError', 'The license text could not be loaded.')}</SectionNote>
          <Button size="small" onClick={() => void mutate()}>{t('settings.openSource.retry', 'Retry')}</Button>
        </div>
      ) : !files ? (
        <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>
      ) : (
        <div className="space-y-4" data-testid="settings-open-source-license-text">
          {files.map((file, index) => (
            <div key={`${file.name}-${index}`}>
              <h4 className="mb-2 text-xs font-medium text-[color:var(--cp-muted)]">{file.name}</h4>
              <pre className="whitespace-pre-wrap rounded-lg bg-[color:var(--cp-bg)] p-3 font-mono text-xs leading-5 [overflow-wrap:anywhere]">{file.text}</pre>
            </div>
          ))}
        </div>
      )}
    </>
  )
}

function ProjectDetails({ project, scopes, onClose }: { project: SoftwareProject; scopes: SoftwareScope[]; onClose: () => void }) {
  const { t } = useI18n()
  const [software, setSoftware] = useState(project.packages[0])
  const [showComponents, setShowComponents] = useState(project.packages.length > 1)
  const [query, setQuery] = useState('')
  const selectedButton = useRef<HTMLButtonElement>(null)
  const components = useMemo(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
    return project.packages.filter((item) => {
      const text = [item.name, item.version, item.license, item.ecosystem].join(' ').toLowerCase()
      return terms.every((term) => text.includes(term))
    })
  }, [project, query])

  useEffect(() => {
    if (showComponents) selectedButton.current?.focus()
  }, [showComponents])

  return (
    <Dialog open onClose={onClose} fullWidth maxWidth="md" aria-labelledby="open-source-detail-title">
      <DialogTitle id="open-source-detail-title" sx={{ overflowWrap: 'anywhere' }}>{project.name}</DialogTitle>
      <DialogContent key={showComponents ? 'components' : software.id} dividers sx={{ overflowWrap: 'anywhere' }}>
        {showComponents ? (
          <>
            <SectionNote>{project.isFamily
              ? t('settings.openSource.familyDescription', 'Components with similar names are grouped here. Each component keeps its own source, version and license.')
              : t('settings.openSource.projectDescription', 'Select a component to view its exact version, license and source.')}</SectionNote>
            <TextField
              fullWidth size="small" type="search" value={query} sx={{ mt: 2 }}
              label={t('settings.openSource.searchComponents', 'Search components')}
              onChange={(event) => setQuery(event.target.value)}
            />
            <ul className="mt-3 divide-y divide-[color:var(--cp-border)]">
              {components.map((component) => (
                <li key={component.id}>
                  <button
                    type="button" ref={component.id === software.id ? selectedButton : undefined}
                    onClick={() => { setSoftware(component); setShowComponents(false) }}
                    className="flex w-full min-w-0 items-center gap-3 rounded-lg px-2 py-3 text-left hover:bg-[color:var(--cp-hover)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)]"
                    data-testid="settings-open-source-component"
                  >
                    <span className="min-w-0 flex-1">
                      <span className="block break-words text-sm font-medium [overflow-wrap:anywhere]">{component.name}</span>
                      <span className="mt-1 block break-words text-xs text-[color:var(--cp-muted)]">
                        {component.version} · {component.ecosystem} · {component.license || t('settings.openSource.licenseUnknown', 'License not provided')}
                      </span>
                      {component.revision && <span className="mt-1 block break-all font-mono text-xs text-[color:var(--cp-muted)]">{component.revision.slice(0, 12)}</span>}
                    </span>
                    <ChevronRight size={16} className="shrink-0 text-[color:var(--cp-muted)]" aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
            {components.length === 0 && <p className="py-4 text-center text-sm text-[color:var(--cp-muted)]" role="status">{t('settings.openSource.noComponents', 'No matching components')}</p>}
          </>
        ) : (
          <>
            {software.name !== project.name && <h3 className="mb-3 font-semibold">{software.name}</h3>}
            <SoftwareDetails software={software} scopes={scopes} />
          </>
        )}
      </DialogContent>
      <DialogActions>
        {!showComponents && project.packages.length > 1 && <Button onClick={() => setShowComponents(true)}>{t('settings.openSource.backToProject', 'Back to group')}</Button>}
        <Button onClick={onClose}>{t('settings.openSource.close', 'Close')}</Button>
      </DialogActions>
    </Dialog>
  )
}

export function OpenSourcePanel() {
  const { t } = useI18n()
  const { data, error, mutate } = useSWR(catalogUrl, fetchCatalog, fetchOptions)
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(1)
  const [selected, setSelected] = useState<SoftwareProject | null>(null)
  const projects = useMemo(() => groupSoftwareProjects(data?.packages ?? []), [data])
  const filtered = useMemo(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
    return projects.filter((project) => project.packages.some((software) => {
      const text = [project.name, software.name, software.version, software.license, software.ecosystem].join(' ').toLowerCase()
      return terms.every((term) => text.includes(term))
    }))
  }, [projects, query])
  const pageCount = Math.max(1, Math.ceil(filtered.length / pageSize))
  const activePage = Math.min(page, pageCount)

  return (
    <Section
      title={t('settings.openSource.title', 'Open-source software')}
      description={t('settings.openSource.description', 'BuckyOS relies on many open-source projects. Thank you to the developers and contributors who make it possible.')}
      testId="settings-open-source"
    >
      {error ? (
        <div className="space-y-2">
          <SectionNote tone="danger">{t('settings.openSource.loadError', 'The software catalog could not be loaded.')}</SectionNote>
          <Button size="small" onClick={() => void mutate()}>{t('settings.openSource.retry', 'Retry')}</Button>
        </div>
      ) : !data ? (
        <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>
      ) : (
        <div className="space-y-3">
          <details className="text-xs text-[color:var(--cp-muted)]">
            <summary className="cursor-pointer py-1">{t('settings.openSource.coverage', 'Catalog coverage')}</summary>
            <ul className="space-y-2 py-2">
              {data.scopes.map((scope) => (
                <li key={scope.id} className="flex flex-wrap justify-between gap-x-3 gap-y-1">
                  <span>{scopeLabel(scope, t)}</span>
                  <span>
                    {t(`settings.openSource.status.${scope.status}`, scope.status)}
                    {scope.isSnapshot && ` · ${t('settings.openSource.snapshot', 'Saved snapshot')}`}
                  </span>
                </li>
              ))}
            </ul>
            <p className="leading-5">{t('settings.openSource.coverageNote', 'This catalog covers the BuckyOS build. The host operating system, container base images and apps or tools installed separately are not fully included.')}</p>
          </details>
          <TextField
            fullWidth size="small" type="search" value={query}
            label={t('settings.openSource.search', 'Search open-source software')}
            onChange={(event) => { setQuery(event.target.value); setPage(1) }}
          />
          <p className="text-xs text-[color:var(--cp-muted)]" role="status">
            {t('settings.openSource.count', '{{shown}} of {{total}} groups · {{packages}} packages', { shown: filtered.length, total: projects.length, packages: data.packages.length })}
          </p>
          {filtered.length === 0 ? (
            <p className="py-4 text-center text-sm text-[color:var(--cp-muted)]" role="status">{t('settings.openSource.noResults', 'No matching software')}</p>
          ) : (
            <ul key={`${activePage}-${query}`} className="max-h-96 overflow-y-auto divide-y divide-[color:var(--cp-border)]">
              {filtered.slice((activePage - 1) * pageSize, activePage * pageSize).map((project) => (
                <li key={project.id}>
                  <button
                    type="button" onClick={() => setSelected(project)}
                    className="flex w-full min-w-0 items-center gap-3 rounded-lg px-2 py-3 text-left hover:bg-[color:var(--cp-hover)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[color:var(--cp-accent)]"
                    data-testid="settings-open-source-project"
                  >
                    <span className="min-w-0 flex-1">
                      <span className="flex flex-wrap items-center gap-2">
                        <span className="break-words text-sm font-medium [overflow-wrap:anywhere]">{project.name}</span>
                        {project.isFamily && <span className="rounded bg-[color:var(--cp-bg)] px-1.5 py-0.5 text-xs text-[color:var(--cp-muted)]">{t('settings.openSource.family', 'Family')}</span>}
                      </span>
                      <span className="mt-1 block break-words text-xs text-[color:var(--cp-muted)]">
                        {project.packages.length === 1
                          ? t('settings.openSource.oneComponent', '1 component')
                          : t('settings.openSource.componentCount', '{{count}} components', { count: project.packages.length })} · {project.packages.every((item) => item.license === project.packages[0].license)
                          ? project.packages[0].license || t('settings.openSource.licenseUnknown', 'License not provided')
                          : t('settings.openSource.componentLicenses', 'See component licenses')}
                      </span>
                    </span>
                    <ChevronRight size={16} className="shrink-0 text-[color:var(--cp-muted)]" aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          )}
          {pageCount > 1 && (
            <Pagination
              count={pageCount} page={activePage} onChange={(_, next) => setPage(next)} size="small" siblingCount={0}
              aria-label={t('settings.openSource.pagination', 'Software catalog pages')}
              getItemAriaLabel={(type, number) => type === 'page'
                ? t('settings.openSource.page', 'Page {{page}}', { page: number ?? 1 })
                : type === 'previous' ? t('settings.openSource.previous', 'Previous page') : t('settings.openSource.next', 'Next page')}
              sx={{ '& ul': { justifyContent: 'center' } }}
            />
          )}
        </div>
      )}
      {selected && data && <ProjectDetails key={selected.id} project={selected} scopes={data.scopes} onClose={() => setSelected(null)} />}
    </Section>
  )
}
