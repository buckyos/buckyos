/* "开始放映" (第三期规划 §7.2, §8.1): choose a presentation path and its first step; the edits of this window are
 * settled first (what cannot be is asked about), then the show is started (showSession.ts). */

import { useCallback, useState } from 'react'
import { describeError } from '../api/session'
import type { KeyedContent, ShowPathPayload } from '../api/types'
import { useLoad, useStore, useVersion } from '../state/hooks'
import { hasLiveBlocks, pathsOf, playable, resolveSteps, surfacesOfSteps } from './model'
import { WRITE_CAPS } from './showSession'

export function StartShowDialog({ initialPathId, initialStepId, onClose, onStart }: {
  initialPathId: string | null
  initialStepId: string | null
  onClose: () => void
  onStart: (pathId: string, stepId: string | null) => Promise<void>
}) {
  const store = useStore()
  const paths = pathsOf(store.outline, 'presentation')
  const [pathId, setPathId] = useState(initialPathId ?? paths[0]?.entity_id ?? '')
  const [stepId, setStepId] = useState(initialStepId ?? '')
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [leaving, setLeaving] = useState<{ unsaved: number; attention: number; memoryOnly: number } | null>(null)
  const version = useVersion(`e:${pathId}`)
  const load = useCallback(() => (pathId ? store.session.read<KeyedContent<ShowPathPayload>>(pathId).then((r) => r.content.payload) : Promise.resolve(null)), [store, pathId])
  const path = useLoad(load, version).data ?? null
  const all = path ? resolveSteps(store.outline, path) : []
  const steps = playable(all)
  const skipped = all.length - steps.length
  const live = steps.length > 0 && hasLiveBlocks(store.outline, surfacesOfSteps(steps))
  const offline = store.session.status().kind !== 'live'
  const writer = store.session.info().capabilities.some((c) => WRITE_CAPS.includes(c))
  const go = async (force: boolean) => {
    if (!pathId) return
    setError(null)
    if (!force) {
      // §8.1: end the active edit and wait for the writes that can still complete
      setBusy('正在保存未提交的修改…')
      const left = await store.prepareLeave()
      setBusy(null)
      if (left.unsaved + left.attention + left.memoryOnly > 0) { setLeaving(left); return }
    }
    setBusy(live && writer && !offline ? '正在准备临时副本…' : '正在开始放映…')
    try {
      await onStart(pathId, stepId || null)
    } catch (e) {
      setError(describeError(e))
      setBusy(null)
    }
  }
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget && !busy) onClose() }}>
      <div className="aiws-dialog" role="dialog" aria-modal="true" aria-label="开始放映" data-testid="aiws-start-show" onKeyDown={(event) => { if (event.key === 'Escape' && !busy) onClose() }}>
        <div className="aiws-dialog-head"><b>开始放映</b></div>
        {paths.length === 0 ? (
          <div className="aiws-muted">这个工作区还没有演讲路径：在画布的“路径编辑”模式中创建一条。</div>
        ) : (
          <>
            <label className="aiws-inline-form">演讲路径
              <select aria-label="演讲路径" data-testid="aiws-start-path" value={pathId} disabled={busy !== null} onChange={(event) => { setPathId(event.target.value); setStepId('') }}>
                {paths.map((p) => <option key={p.entity_id} value={p.entity_id}>{p.title ?? p.name ?? p.entity_id}</option>)}
              </select>
            </label>
            <label className="aiws-inline-form">从这一步开始
              <select aria-label="起始步骤" data-testid="aiws-start-step" value={stepId} disabled={busy !== null} onChange={(event) => setStepId(event.target.value)}>
                <option value="">第一步</option>
                {steps.map((s, i) => <option key={s.step.id} value={s.step.id}>{i + 1}. {s.title}</option>)}
              </select>
            </label>
            {path && <div className="aiws-muted" data-testid="aiws-start-summary">{steps.length} 步{skipped > 0 ? `；${skipped} 个已停用或失效的步骤会被跳过` : ''}；舞台 {path.stage.w} × {path.stage.h}。</div>}
            {offline ? (
              <div className="aiws-warning">后台当前不可达：只能在这个窗口里只读放映，不能关闭他人的写入，没有提示器链接，也不能现场操作 Block。</div>
            ) : writer ? (
              <div className="aiws-muted">放映期间，这个工作区对其他人和你的其他窗口关闭写入，放映结束后恢复。{live ? '路径中有“放映时可操作”的 Block：现场操作写入临时副本，放映结束后自动删除，不影响原作品。' : ''}</div>
            ) : (
              <div className="aiws-muted">你没有写权限：这是只读放映，内容可能在放映中被他人修改。</div>
            )}
            {leaving && (
              <div className="aiws-warning" data-testid="aiws-start-pending">
                还有 {leaving.unsaved + leaving.attention + leaving.memoryOnly} 项修改没有提交（未保存或需要处理）。放映期间无法提交它们，放映结束后可以继续处理。
                <div className="aiws-dialog-actions">
                  <button type="button" onClick={() => { setLeaving(null); void go(true) }}>仍然开始</button>
                  <button type="button" onClick={onClose}>取消</button>
                </div>
              </div>
            )}
          </>
        )}
        {error && <div className="aiws-error" role="alert">没有开始放映：{error}</div>}
        <div className="aiws-dialog-actions">
          <button type="button" className="is-primary" data-testid="aiws-start-show-go" disabled={!pathId || steps.length === 0 || busy !== null || leaving !== null} onClick={() => { void go(false) }}>{busy ?? '开始放映'}</button>
          <button type="button" disabled={busy !== null} onClick={onClose}>取消</button>
        </div>
      </div>
    </div>
  )
}
