/* The reflow library (RF7): the sites the user's library holds — who wrote each package and when, the preference,
 * how many pages were made — with a look inside a site, deleting it, and importing a package from a zip or from a
 * folder of the Zone (where an Agent that may not write the library leaves the package it wrote). */

import { FileArchive, FolderInput, Trash2 } from 'lucide-react'
import type { ZoneContent } from 'buckyos/content'
import { useCallback, useEffect, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useSelectContent } from '../../../sysdlg'
import { usePickFiles } from '../../../sysdlg/select/pickFiles'
import { readZoneFiles, readZoneFolder } from '../../../sysdlg/select/zoneFiles'
import { formatBytes } from '../../../sysdlg/shareFormat'
import { installFileBrowserData } from '../../filebrowser/data/install'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { reflowControllerOf } from './controller'
import { MANIFEST_FILE, ReflowError, importPackage, listSites, readManifestText, removeSite, type Author, type SiteEntry } from './library'
import { packageFilesOfFolder, packageFilesOfZip } from './packageFiles'
import { getReflowSource } from './source'
import { formatMoment } from './useReflow'
import './reflow.css'

const PICKER_ID = 'agent-browser-reflow-import'

type Report = { kind: 'imported'; site: string } | { kind: 'invalid'; problems: string[] } | { kind: 'failed' }

const hostsOf = (entry: SiteEntry) => entry.package ? [...new Set(entry.package.match.map((rule) => rule.host))].join(', ') : entry.site

function pretty(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
}

export function ReflowLibrarySheet({ onClose }: { onClose: () => void }) {
  const store = useAgentBrowserStore()
  const { t, locale } = useI18n()
  const agents = useAgentBrowser((snapshot) => snapshot.agents)
  const pickFiles = usePickFiles(PICKER_ID)
  const select = useSelectContent()
  const [sites, setSites] = useState<SiteEntry[] | null>(null)
  const [unreadable, setUnreadable] = useState(false)
  const [opened, setOpened] = useState<string | null>(null)
  const [manifest, setManifest] = useState<{ site: string; text: string } | null>(null)
  const [confirming, setConfirming] = useState(false)
  const [picking, setPicking] = useState(false)
  const [busy, setBusy] = useState(false)
  const [report, setReport] = useState<Report | null>(null)

  const read = useCallback(() => getReflowSource().then((source) => source.library()).then(listSites), [])
  useEffect(() => {
    let alive = true
    read().then((list) => { if (alive) setSites(list) }, () => { if (alive) setUnreadable(true) })
    return () => { alive = false }
  }, [read])
  const load = async () => {
    try {
      setSites(await read())
      setUnreadable(false)
    } catch {
      setUnreadable(true)
    }
  }

  const authorName = (author: Author) => author.kind === 'agent' ? agents.find((agent) => agent.agentDid === author.id)?.name ?? author.id : author.id

  const open = (site: string) => {
    setOpened(site)
    void getReflowSource().then((source) => source.library()).then((library) => readManifestText(library, site)).then((text) => setManifest({ site, text: text ?? '' }), () => undefined)
  }

  const remove = async (site: string) => {
    setBusy(true)
    try {
      await removeSite(await (await getReflowSource()).library(), site)
      setReport(null)
    } catch {
      setReport({ kind: 'failed' })
    }
    setConfirming(false)
    setOpened(null)
    await load()
    setBusy(false)
  }

  /** The system file picker shows instead of this panel, not over it. */
  const pick = async <T,>(choose: () => Promise<T>): Promise<T> => {
    setPicking(true)
    try {
      return await choose()
    } finally {
      setPicking(false)
    }
  }

  const importFrom = async (read: () => Promise<Map<string, Uint8Array> | null>) => {
    if (busy) return
    setBusy(true)
    setReport(null)
    try {
      const files = await read()
      if (!files) return
      const library = await (await getReflowSource()).library()
      const done = await importPackage(library, files, { author: { kind: 'user', id: library.userId } })
      setReport({ kind: 'imported', site: done.site })
      reflowControllerOf(store).libraryChanged(done.site)
      await load()
    } catch (error) {
      setReport(error instanceof ReflowError ? { kind: 'invalid', problems: error.problems.length > 0 ? error.problems : [error.message] } : { kind: 'failed' })
    } finally {
      setBusy(false)
    }
  }

  const importZip = () => importFrom(async () => {
    const picked = await pick(() => pickFiles({ accept: '.zip,application/zip', title: t('agentBrowser.reflow.library.pickZip', 'Choose the zip of a reflow package') }))
    return picked?.length ? packageFilesOfZip(new Uint8Array(await picked[0].arrayBuffer())) : null
  })

  const importFolder = () => importFrom(async () => {
    const result = await pick(() => select({ mode: 'folder', title: t('agentBrowser.reflow.library.pickFolder', 'Choose the folder of a reflow package') }, { id: PICKER_ID }))
    const folder = result.action === 'selected' ? result.items[0] : undefined
    if (!folder || folder.origin !== 'zone' || folder.kind !== 'folder' || folder.source.kind !== 'cyfs-path') return null
    installFileBrowserData()
    // A folder without a manifest is no package: nothing else of it is read.
    const manifest: ZoneContent = { origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path: `${folder.source.path.replace(/\/$/, '')}/${MANIFEST_FILE}` }, hints: { name: MANIFEST_FILE } }
    await readZoneFiles([manifest]).catch((error: unknown) => {
      throw (error as { code?: unknown } | null)?.code === 'NOT_FOUND' ? new ReflowError('the package is not valid', [`${MANIFEST_FILE} is missing`]) : error
    })
    return packageFilesOfFolder(await readZoneFolder(folder))
  })

  if (picking) return null
  const entry = sites?.find((item) => item.site === opened) ?? null

  if (entry && confirming) {
    return (
      <Sheet
        testId="ab-reflow-delete-confirm"
        title={t('agentBrowser.reflow.library.deleteTitle', 'Delete {{site}}?', { site: hostsOf(entry) })}
        onClose={() => setConfirming(false)}
        footer={<>
          <button type="button" className="ab-button" data-testid="ab-reflow-delete-cancel" onClick={() => setConfirming(false)}>{t('common.cancel')}</button>
          <button type="button" className="ab-button ab-button-danger" disabled={busy} data-testid="ab-reflow-delete-accept" onClick={() => void remove(entry.site)}>{t('agentBrowser.reflow.library.delete', 'Delete')}</button>
        </>}
      >
        <p className="ab-dialog-message">{t('agentBrowser.reflow.library.deleteBody', 'The reflow package of this site, your preference for it and every page made for it will be deleted from your library. The pages of this site will have to be made again.')}</p>
      </Sheet>
    )
  }

  if (entry) {
    return (
      <Sheet
        testId="ab-reflow-site"
        title={hostsOf(entry)}
        onClose={() => setOpened(null)}
        footer={<>
          <button type="button" className="ab-button" data-testid="ab-reflow-site-back" onClick={() => setOpened(null)}>{t('common.back')}</button>
          <button type="button" className="ab-button ab-button-danger" data-testid="ab-reflow-site-delete" onClick={() => setConfirming(true)}><Trash2 size={16} aria-hidden />{t('agentBrowser.reflow.library.delete', 'Delete')}</button>
        </>}
      >
        <dl className="ab-reflow-facts" data-testid="ab-reflow-site-facts">
          <dt>{t('agentBrowser.reflow.library.site', 'Site')}</dt><dd>{entry.site}</dd>
          {entry.package ? <>
            <dt>{t('agentBrowser.reflow.library.author', 'Written by')}</dt><dd>{authorName(entry.package.author)}</dd>
            <dt>{t('agentBrowser.reflow.library.updated', 'Updated')}</dt><dd>{formatMoment(entry.package.updated_at, locale)}</dd>
          </> : null}
          <dt>{t('agentBrowser.reflow.library.preference', 'Preference')}</dt><dd>{entry.preference || t('agentBrowser.reflow.library.none', 'None')}</dd>
          <dt>{t('agentBrowser.reflow.library.results', 'Made with the package')}</dt><dd>{entry.results}</dd>
          <dt>{t('agentBrowser.reflow.library.fallbacks', 'One-off pages')}</dt><dd>{entry.fallbacks}</dd>
        </dl>
        {entry.files.length > 0 ? <>
          <p className="ab-list-heading">manifest.json</p>
          <pre className="ab-reflow-manifest" data-testid="ab-reflow-manifest">{manifest?.site === entry.site ? pretty(manifest.text) : ''}</pre>
          <p className="ab-list-heading">{t('agentBrowser.reflow.library.files', 'Files of the package')}</p>
          <ul className="ab-reflow-files" data-testid="ab-reflow-files">
            {entry.files.map((file) => (
              <li key={file.name} data-testid="ab-reflow-file" data-name={file.name}>
                <span className="ab-reflow-file-name">{file.name}</span>
                <span className="ab-reflow-file-size">{formatBytes(file.size)}</span>
              </li>
            ))}
          </ul>
        </> : <p className="ab-sheet-empty">{t('agentBrowser.reflow.library.noPackageHint', 'This site has no reflow package: its pages are made one by one.')}</p>}
      </Sheet>
    )
  }

  return (
    <Sheet
      testId="ab-reflow-library"
      title={t('agentBrowser.reflow.library.title', 'Reflow library')}
      onClose={onClose}
      footer={<>
        <button type="button" className="ab-button" disabled={busy} data-testid="ab-reflow-import-folder" onClick={() => void importFolder()}><FolderInput size={16} aria-hidden />{t('agentBrowser.reflow.library.importFolder', 'Import folder')}</button>
        <button type="button" className="ab-button ab-button-primary" disabled={busy} data-testid="ab-reflow-import-zip" onClick={() => void importZip()}><FileArchive size={16} aria-hidden />{t('agentBrowser.reflow.library.importZip', 'Import zip')}</button>
      </>}
    >
      {report ? (
        <div className="ab-reflow-report" role={report.kind === 'imported' ? 'status' : 'alert'} data-testid="ab-reflow-report" data-kind={report.kind}>
          {report.kind === 'imported' ? t('agentBrowser.reflow.library.imported', 'The package of {{site}} is in your library.', { site: report.site })
            : report.kind === 'failed' ? t('agentBrowser.reflow.library.failed', 'That did not work. Try again.')
            : <>
              {t('agentBrowser.reflow.library.invalid', 'This is not a valid reflow package, so nothing was imported:')}
              <ul className="ab-reflow-problems">{report.problems.map((problem) => <li key={problem}>{problem}</li>)}</ul>
            </>}
        </div>
      ) : null}
      {sites === null ? (
        <p className="ab-sheet-empty" data-testid={unreadable ? 'ab-reflow-library-unreadable' : 'ab-reflow-library-loading'}>
          {unreadable ? t('agentBrowser.reflow.library.unreadable', 'Your reflow library could not be read.') : t('agentBrowser.reflow.library.loading', 'Loading…')}
        </p>
      ) : sites.length === 0 ? (
        <p className="ab-sheet-empty" data-testid="ab-reflow-library-empty">{t('agentBrowser.reflow.library.empty', 'Nothing has been reflowed yet. Pages you open in Reflow are kept here.')}</p>
      ) : (
        <ul className="ab-list" data-testid="ab-reflow-sites">
          {sites.map((item) => (
            <li key={item.site}>
              <button type="button" className="ab-list-item ab-reflow-site" data-testid={`ab-reflow-site-${item.site}`} data-package={item.package ? 'true' : 'false'} data-results={item.results} data-fallbacks={item.fallbacks} onClick={() => open(item.site)}>
                <span className="ab-list-text">
                  <span className="ab-list-title">{hostsOf(item)}</span>
                  <span className="ab-list-detail">
                    {item.package
                      ? t('agentBrowser.reflow.library.packageBy', 'Package by {{author}} · updated {{time}}', { author: authorName(item.package.author), time: formatMoment(item.package.updated_at, locale) })
                      : t('agentBrowser.reflow.library.noPackage', 'No package')}
                  </span>
                  {item.preference ? <span className="ab-list-detail">{t('agentBrowser.reflow.library.preferenceLine', 'Preference: {{preference}}', { preference: item.preference })}</span> : null}
                  <span className="ab-list-detail">{t('agentBrowser.reflow.library.made', '{{results}} made with the package · {{fallbacks}} one-off pages', { results: item.results, fallbacks: item.fallbacks })}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </Sheet>
  )
}
