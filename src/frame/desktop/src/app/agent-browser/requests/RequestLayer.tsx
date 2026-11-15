/* What a page asks for outside itself (protocol §3.4), shown by the App: the options of a <select>, a JS dialog,
 * a file to upload (from the Zone first, this device second), a finished download to keep in the Zone, and the
 * notice of a permission the service refused. */

import { Check, Download, FileUp, ShieldOff } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useSaveToZone } from '../../../components/saveToZone'
import { useI18n } from '../../../i18n/provider'
import { useSelectContent } from '../../../sysdlg'
import { acceptToSelectors } from '../../../sysdlg/select/pickFiles'
import { formatBytes } from '../../../sysdlg/shareFormat'
import type { ScreenRequest } from '../api/types'
import { Sheet } from '../shell/Sheet'
import { useAgentBrowser, useAgentBrowserStore } from '../store/context'
import { canOperate } from '../store/store'

type RequestOf<K extends ScreenRequest['kind']> = Extract<ScreenRequest, { kind: K }>

/** How long after a press of the page a file request still counts as its result. */
const PRESS_MS = 5000

function hostOf(url: string): string {
  try { return new URL(url).host || url } catch { return url }
}

function PageDialog({ request }: { request: RequestOf<'dialog'> }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const url = useAgentBrowser((snapshot) => snapshot.state?.url ?? '')
  const [text, setText] = useState(request.default_prompt ?? '')
  const kind = request.dialog_type
  const answer = (accept: boolean) => store.respond(request.request_id, { accept, ...(accept && kind === 'prompt' ? { prompt_text: text } : {}) })
  const leaving = kind === 'beforeunload'
  return (
    <Sheet
      modal
      testId="ab-dialog"
      title={leaving ? t('agentBrowser.dialog.leaveTitle', 'Leave this page?') : t('agentBrowser.dialog.title', '{{host}} says', { host: hostOf(url) })}
      onClose={() => answer(kind === 'alert')}
      footer={<>
        {kind === 'alert' ? null : <button type="button" className="ab-button" data-testid="ab-dialog-cancel" onClick={() => answer(false)}>{leaving ? t('agentBrowser.dialog.stay', 'Stay') : t('common.cancel')}</button>}
        <button type="button" className="ab-button ab-button-primary" data-testid="ab-dialog-accept" onClick={() => answer(true)}>{leaving ? t('agentBrowser.dialog.leave', 'Leave') : t('agentBrowser.ok', 'OK')}</button>
      </>}
    >
      <p className="ab-dialog-message" data-testid="ab-dialog-message">{request.message || (leaving ? t('agentBrowser.dialog.leaveBody', 'Changes you made may not be saved.') : '')}</p>
      {kind === 'prompt' ? (
        <input
          className="ab-field"
          data-testid="ab-dialog-input"
          value={text}
          autoFocus
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => { if (event.key === 'Enter' && !event.nativeEvent.isComposing) answer(true) }}
        />
      ) : null}
    </Sheet>
  )
}

function SelectPanel({ request }: { request: RequestOf<'select'> }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const [chosen, setChosen] = useState(() => new Set(request.options.filter((option) => option.selected).map((option) => option.index)))
  const pick = (index: number) => {
    if (!request.multiple) { store.respond(request.request_id, { selected: [index] }); return }
    setChosen((previous) => {
      const next = new Set(previous)
      if (!next.delete(index)) next.add(index)
      return next
    })
  }
  return (
    <Sheet
      testId="ab-select"
      title={t(request.multiple ? 'agentBrowser.select.titleMultiple' : 'agentBrowser.select.title', request.multiple ? 'Choose options' : 'Choose an option')}
      onClose={() => store.respond(request.request_id, { cancel: true })}
      footer={request.multiple ? <button type="button" className="ab-button ab-button-primary" data-testid="ab-select-done" onClick={() => store.respond(request.request_id, { selected: [...chosen].sort((a, b) => a - b) })}>{t('agentBrowser.ok', 'OK')}</button> : undefined}
    >
      <ul role="listbox" aria-multiselectable={request.multiple} className="ab-list">
        {request.options.map((option, position) => {
          const selected = chosen.has(option.index)
          const heading = option.group && option.group !== request.options[position - 1]?.group ? option.group : null
          return (
            <li key={option.index} role="presentation">
              {heading ? <p className="ab-list-heading">{heading}</p> : null}
              <button
                type="button"
                role="option"
                aria-selected={selected}
                disabled={option.disabled}
                className="ab-list-item"
                data-testid={`ab-select-option-${option.index}`}
                onClick={() => pick(option.index)}
              >
                <span className="ab-list-text">{option.label}</span>
                {selected ? <Check size={18} aria-hidden /> : null}
              </button>
            </li>
          )
        })}
      </ul>
    </Sheet>
  )
}

function FileChooser({ request }: { request: RequestOf<'file_chooser'> }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const select = useSelectContent()
  const operable = useAgentBrowser(canOperate)
  const [uploading, setUploading] = useState(false)
  const choosing = useRef(false)
  const choose = useCallback(async () => {
    if (choosing.current) return
    choosing.current = true
    try {
      const result = await select(
        { mode: 'open', multiple: request.multiple, accept: acceptToSelectors(request.accept.join(',')), allowLocal: true, title: t('agentBrowser.upload.pick', 'Choose a file to upload') },
        { id: 'agent-browser-upload' },
      )
      if (result.action !== 'selected' || result.items.length === 0) { store.respond(request.request_id, { cancel: true }); return }
      setUploading(true)
      await store.uploadFiles(request.request_id, result.items)
    } catch (error) {
      store.reportFailure(error)
    } finally {
      choosing.current = false
      setUploading(false)
    }
  }, [store, select, request, t])
  // The user just pressed the page's own upload control: the picker opens at once. A request raised while the Agent
  // operates, or one that was already waiting when the screen connected, waits for a press.
  const opened = useRef(!operable || !store.pressedWithin(PRESS_MS))
  useEffect(() => {
    if (opened.current) return
    opened.current = true
    void choose()
  }, [choose])
  return (
    <div className="ab-note" role="status" data-testid="ab-file-chooser">
      <FileUp size={18} aria-hidden />
      <p className="ab-note-text">{uploading ? t('agentBrowser.upload.uploading', 'Uploading…') : t('agentBrowser.upload.asks', 'The page asks for a file to upload.')}</p>
      <button type="button" className="ab-button ab-button-primary" disabled={uploading} data-testid="ab-file-choose" onClick={() => void choose()}>{t('agentBrowser.upload.choose', 'Choose file')}</button>
      <button type="button" className="ab-button" disabled={uploading} data-testid="ab-file-cancel" onClick={() => store.respond(request.request_id, { cancel: true })}>{t('common.cancel')}</button>
    </div>
  )
}

function DownloadNote({ request }: { request: RequestOf<'download'> }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const saveToZone = useSaveToZone(t('agentBrowser.download.nameTaken', 'That name is taken. Choose another name'))
  const [saving, setSaving] = useState(false)
  const [savedPath, setSavedPath] = useState<string | null>(null)
  const failed = request.state === 'failed'
  const save = async () => {
    setSaving(true)
    try {
      const result = await saveToZone({
        name: request.name,
        kind: 'file',
        grant: async () => (await store.exportDownload(request.download_id)).obj_id,
        mockFiles: async () => [{ path: '', blob: new Blob([`mock download: ${request.name}`]) }],
      })
      if (result.status === 'saved') setSavedPath(result.path)
    } catch (error) {
      store.reportFailure(error)
    } finally {
      setSaving(false)
    }
  }
  return (
    <div className="ab-note" role="status" data-testid="ab-download" data-state={savedPath ? 'saved' : request.state}>
      <Download size={18} aria-hidden />
      <p className="ab-note-text">
        {savedPath ? t('agentBrowser.download.saved', 'Saved to {{path}}', { path: savedPath })
          : failed ? t('agentBrowser.download.failed', 'Download failed: {{name}}', { name: request.name })
          : t('agentBrowser.download.done', 'Downloaded {{name}} ({{size}})', { name: request.name, size: formatBytes(request.size) })}
      </p>
      {failed || savedPath ? null : <button type="button" className="ab-button ab-button-primary" disabled={saving} data-testid="ab-download-save" onClick={() => void save()}>{t('agentBrowser.download.save', 'Save to Zone')}</button>}
      <button type="button" className="ab-button" disabled={saving} data-testid="ab-download-dismiss" onClick={() => store.dismissRequest(request.request_id)}>{failed || savedPath ? t('agentBrowser.ok', 'OK') : t('agentBrowser.dismiss', 'Dismiss')}</button>
    </div>
  )
}

function PermissionNote({ request }: { request: RequestOf<'permission'> }) {
  const store = useAgentBrowserStore()
  const { t } = useI18n()
  const what = t(`agentBrowser.permission.${request.permission}`, request.permission)
  return (
    <div className="ab-note" role="status" data-testid="ab-permission">
      <ShieldOff size={18} aria-hidden />
      <p className="ab-note-text">{t('agentBrowser.permission.denied', '{{origin}} asked for {{what}}. The request was refused.', { origin: hostOf(request.origin), what })}</p>
      <button type="button" className="ab-button" data-testid="ab-permission-dismiss" onClick={() => store.dismissRequest(request.request_id)}>{t('agentBrowser.ok', 'OK')}</button>
    </div>
  )
}

export function RequestLayer() {
  const requests = useAgentBrowser((snapshot) => snapshot.requests)
  const dialog = requests.find((request) => request.kind === 'dialog')
  const select = requests.find((request) => request.kind === 'select')
  const notes = requests.filter((request) => request.kind === 'file_chooser' || request.kind === 'download' || request.kind === 'permission')
  return (
    <>
      {notes.length > 0 ? (
        <div className="ab-notes" data-testid="ab-notes">
          {notes.map((request) => request.kind === 'file_chooser' ? <FileChooser key={request.request_id} request={request} />
            : request.kind === 'download' ? <DownloadNote key={request.request_id} request={request} />
            : request.kind === 'permission' ? <PermissionNote key={request.request_id} request={request} /> : null)}
        </div>
      ) : null}
      {dialog ? <PageDialog key={dialog.request_id} request={dialog} /> : select ? <SelectPanel key={select.request_id} request={select} /> : null}
    </>
  )
}
