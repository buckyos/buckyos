/* The cover dialog (UI 框架 PRD §9.2, Q37): the cover now (or the placeholder's words), a picture chosen in the system
 * file picker — the Zone first, this device through the picker's own entry — previewed before it is saved, and
 * "移除封面" while there is one. One dialog for both ways in: the home card's ⋯ (over kRPC, `writeCoverFromHome`) and
 * "工作区 ▸ 设置封面…" inside the Workspace (`WorkspaceCoverHost`: an undoable settings Commit of the open store). */

import { useEffect, useState } from 'react'
import { ImageOff } from 'lucide-react'
import { describeError } from '../../api/session'
import type { Transport } from '../../api/transport'
import type { HomeCover } from '../../api/types'
import { useStore } from '../../state/hooks'
import { useWorkspaceSettings } from '../../state/settings'
import { useAiwsText } from '../../useAiwsText'
import { formatBytes } from '../directory/format'
import { CoverPicture } from '../home/CoverPicture'
import { Modal } from '../shell/dialogs'
import { useFilePicker } from '../shell/filePicker'
import { useShell } from '../shell/shellContext'
import { COVER_ACCEPT, CoverRefused, coverProblem, onCoverDialog, outcomeProblem } from './cover'
import './cover.css'

export interface CoverTarget {
  workspaceId: string
  title: string
  /** The cover now, or null. */
  current: HomeCover | null
  /** Where the current cover's bytes come from. */
  transport: Transport
  /** Why the caller cannot change it, or null. */
  reason: string | null
  /** Upload `file` and make it the cover; null removes it. Throws with what to tell the person. */
  write(file: File | null): Promise<void>
}

type Picked = { file: File; url: string }

export function CoverDialog({ target, onClose, onDone }: { target: CoverTarget; onClose: () => void; onDone?: (removed: boolean) => void }) {
  const { t } = useAiwsText()
  const [picked, setPicked] = useState<Picked | null>(null)
  const [busy, setBusy] = useState<'saving' | 'uploading' | null>(null)
  const [error, setError] = useState<string | null>(null)
  const picker = useFilePicker((message) => setError(message))
  useEffect(() => () => { if (picked) URL.revokeObjectURL(picked.url) }, [picked])

  const pick = async () => {
    setError(null)
    const file = await picker.files({ accept: COVER_ACCEPT, title: t('aiworkspace.cover.pickTitle') }).then((files) => files?.[0] ?? null)
    if (!file) return
    const problem = coverProblem(file)
    if (problem) { setError(problem); return }
    setPicked({ file, url: URL.createObjectURL(file) })
  }
  const write = async (file: File | null) => {
    setBusy(file ? 'uploading' : 'saving')
    setError(null)
    try {
      await target.write(file)
      onDone?.(file === null)
      onClose()
    } catch (e) {
      setError(e instanceof CoverRefused ? e.message : t('aiworkspace.cover.failed', { error: describeError(e) }))
    } finally {
      setBusy(null)
    }
  }
  const locked = target.reason !== null || busy !== null
  const label = t('aiworkspace.cover.title', { name: target.title })
  const state = picked ? 'picked' : target.current ? 'current' : 'none'
  return (
    <Modal label={label} testId="aiws-cover-dialog" className="aiws-cover-dialog" onClose={() => { if (!busy) onClose() }}>
      <div className="aiws-dialog-head"><b className="aiws-user-text">{label}</b><span className="aiws-grow" /><button type="button" className="aiws-link" disabled={busy !== null} data-testid="aiws-cover-close" onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <div className="aiws-cover-preview" data-testid="aiws-cover-preview" data-state={state}>
        {picked ? <img className="aiws-cover-preview-picture" src={picked.url} alt="" draggable={false} data-testid="aiws-cover-preview-picture" />
          : target.current ? (
            <CoverPicture transport={target.transport} cover={target.current} className="aiws-cover-preview-current">
              <ImageOff size={28} aria-hidden="true" />
            </CoverPicture>
          ) : <span className="aiws-cover-preview-none"><ImageOff size={28} aria-hidden="true" />{t('aiworkspace.cover.none')}</span>}
      </div>
      <div className="aiws-muted aiws-cover-caption" data-testid="aiws-cover-caption">
        {picked ? t('aiworkspace.cover.picked', { name: picked.file.name, size: formatBytes(picked.file.size) }) : target.current ? t('aiworkspace.cover.current') : null}
      </div>
      {target.reason ? <div className="aiws-warning" role="status" data-testid="aiws-cover-reason">{target.reason}</div>
        : <div className="aiws-muted aiws-cover-hint"><div>{t('aiworkspace.cover.hint')}</div><div>{t('aiworkspace.cover.deviceHint')}</div></div>}
      {error && <div className="aiws-error" role="alert" data-testid="aiws-cover-error">{error}</div>}
      {busy && <div className="aiws-muted" role="status" data-testid="aiws-cover-busy">{t(busy === 'uploading' ? 'aiworkspace.cover.uploading' : 'aiworkspace.cover.saving')}</div>}
      <div className="aiws-dialog-actions">
        <button type="button" data-testid="aiws-cover-pick" disabled={locked} onClick={() => { void pick() }}>{t(picked || target.current ? 'aiworkspace.cover.pickAnother' : 'aiworkspace.cover.pick')}</button>
        {target.current && !picked && <button type="button" className="is-danger" data-testid="aiws-cover-remove" disabled={locked} onClick={() => { void write(null) }}>{t('aiworkspace.cover.remove')}</button>}
        <span className="aiws-grow" />
        <button type="button" disabled={busy !== null} onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
        <button type="button" className="is-primary" data-testid="aiws-cover-save" disabled={locked || !picked} onClick={() => { if (picked) void write(picked.file) }}>{t('aiworkspace.cover.save')}</button>
      </div>
    </Modal>
  )
}

/** "工作区 ▸ 设置封面…" (ws.cover): the open Workspace's cover, written as one undoable settings Commit. Mounted once by
 * the shell (FrameworkHost). */
export function WorkspaceCoverHost() {
  const [open, setOpen] = useState(false)
  useEffect(() => onCoverDialog(() => setOpen(true)), [])
  return open ? <WorkspaceCoverDialog onClose={() => setOpen(false)} /> : null
}

function WorkspaceCoverDialog({ onClose }: { onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const settings = useWorkspaceSettings()
  const info = store.session.info()
  const cover = settings.value.cover
  const target: CoverTarget = {
    workspaceId: store.session.workspaceId,
    title: info.title,
    current: cover ? { workspace_id: store.session.workspaceId, object_id: cover.object_id, media_type: cover.media_type } : null,
    transport: shell.client.transport,
    reason: !info.capabilities.includes('manage') ? t('aiworkspace.cover.needManage') : settings.writeReason,
    write: async (file) => {
      const value = file ? { object_id: (await store.session.uploadAsset(file, file.name)).object_id } : null
      const problem = outcomeProblem(await settings.update({ cover: value }, t(file ? 'aiworkspace.cover.setLabel' : 'aiworkspace.cover.removeLabel')))
      if (problem) throw new CoverRefused(problem)
    },
  }
  return <CoverDialog target={target} onClose={onClose} onDone={(removed) => store.notify('info', t(removed ? 'aiworkspace.cover.removed' : 'aiworkspace.cover.saved', { name: info.title }))} />
}
