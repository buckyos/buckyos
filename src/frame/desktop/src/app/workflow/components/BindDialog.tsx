/* ── Bind Definition → MountPoint dialog ── */

import { useState } from 'react'
import { ArrowRight, X } from 'lucide-react'
import { useWorkflowStore } from '../hooks/use-workflow-store'
import type { WorkflowDefinition } from '../mock/types'

interface Props {
  definition: WorkflowDefinition
  onClose: () => void
  onDone: (appId: string, mountPointId: string) => void
}

export function BindDialog({ definition, onClose, onDone }: Props) {
  const store = useWorkflowStore()
  const apps = store.listApps()
  const [appId, setAppId] = useState<string>(apps[0]?.id ?? '')
  const app = apps.find((a) => a.id === appId)
  const [mountPointId, setMountPointId] = useState<string>(
    app?.mountPoints[0]?.id ?? '',
  )
  const mp = app?.mountPoints.find((m) => m.id === mountPointId)
  const blocked = definition.analysis.errorCount > 0

  function handleSubmit() {
    if (!app || !mp || blocked) return
    store.bindMountPoint(app.id, mp.id, definition.id, definition.version)
    onDone(app.id, mp.id)
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: 'rgba(0,0,0,0.4)' }}
      onClick={onClose}
    >
      <div
        className="w-[520px] max-w-[92vw] rounded-2xl"
        style={{
          background: 'var(--cp-surface)',
          border: '1px solid var(--cp-border)',
          boxShadow: 'var(--cp-window-shadow)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className="flex items-center gap-2 px-4 py-3"
          style={{ borderBottom: '1px solid var(--cp-border)' }}
        >
          <div className="flex-1 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
            Bind to mount point
          </div>
          <button
            type="button"
            onClick={onClose}
            className="flex h-6 w-6 items-center justify-center"
            style={{ color: 'var(--cp-muted)' }}
          >
            <X size={14} />
          </button>
        </div>
        <div className="p-4">
          {blocked && (
            <div
              className="mb-3 rounded p-2 text-xs"
              style={{
                background: 'color-mix(in srgb, var(--cp-danger) 14%, transparent)',
                color: 'var(--cp-danger)',
                border: '1px solid var(--cp-danger)',
              }}
            >
              This Definition has Error-level analysis issues. Binding is blocked.
            </div>
          )}

          <div className="mb-3 grid grid-cols-2 gap-3">
            <label className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              Application
              <select
                className="mt-1 block w-full rounded-lg px-2 py-1.5 text-xs"
                style={{
                  background: 'var(--cp-surface-2)',
                  color: 'var(--cp-text)',
                  border: '1px solid var(--cp-border)',
                }}
                value={appId}
                onChange={(e) => {
                  setAppId(e.target.value)
                  const a = apps.find((x) => x.id === e.target.value)
                  setMountPointId(a?.mountPoints[0]?.id ?? '')
                }}
              >
                {apps.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              Mount point
              <select
                className="mt-1 block w-full rounded-lg px-2 py-1.5 text-xs"
                style={{
                  background: 'var(--cp-surface-2)',
                  color: 'var(--cp-text)',
                  border: '1px solid var(--cp-border)',
                }}
                value={mountPointId}
                onChange={(e) => setMountPointId(e.target.value)}
              >
                {(app?.mountPoints ?? []).map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {mp && (
            <div
              className="mb-3 rounded-lg p-3 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                border: '1px solid var(--cp-border)',
                color: 'var(--cp-text)',
              }}
            >
              <div
                className="mb-2 grid grid-cols-2 gap-2"
                style={{ color: 'var(--cp-muted)' }}
              >
                <div>required: {String(mp.required)}</div>
                <div>allowEmpty: {String(mp.allowEmpty)}</div>
              </div>

              <div className="flex items-center gap-2">
                <div className="flex-1 rounded p-2" style={{ background: 'var(--cp-surface)' }}>
                  <div className="text-[10px] uppercase" style={{ color: 'var(--cp-muted)' }}>
                    current
                  </div>
                  <div className="mt-1">
                    {mp.currentBinding
                      ? `${store.getDefinition(mp.currentBinding.definitionId)?.name ?? mp.currentBinding.definitionId} v${mp.currentBinding.definitionVersion}`
                      : 'Empty'}
                  </div>
                </div>
                <ArrowRight size={14} style={{ color: 'var(--cp-muted)' }} />
                <div
                  className="flex-1 rounded p-2"
                  style={{
                    background: 'color-mix(in srgb, var(--cp-accent) 14%, var(--cp-surface))',
                    border: '1px solid color-mix(in srgb, var(--cp-accent) 32%, var(--cp-border))',
                  }}
                >
                  <div
                    className="text-[10px] uppercase"
                    style={{ color: 'var(--cp-accent)' }}
                  >
                    new
                  </div>
                  <div className="mt-1">
                    {definition.name} v{definition.version}
                  </div>
                </div>
              </div>

              <div className="mt-2 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                Replacement does not affect in-flight Runs. Each Run locks its
                Definition+version at create time.
              </div>
            </div>
          )}

          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg px-3 py-1.5 text-xs"
              style={{
                background: 'var(--cp-surface-2)',
                color: 'var(--cp-text)',
                border: '1px solid var(--cp-border)',
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleSubmit}
              disabled={blocked || !mp}
              className="rounded-lg px-3 py-1.5 text-xs"
              style={{
                background: blocked
                  ? 'var(--cp-surface-2)'
                  : 'var(--cp-accent)',
                color: blocked ? 'var(--cp-muted)' : 'white',
                border: '1px solid var(--cp-border)',
                opacity: blocked ? 0.6 : 1,
              }}
            >
              Confirm bind
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
