/* The prompter (第三期规划 §11.1, §11.2): the page a prompter link opens — `/workspace/<id>/show/<show>#k=<token>` — in any
 * browser, before and without the Desktop's login. It shows the current step's title and notes (large, adjustable),
 * the next step, the step list and the elapsed time, and sends commands (previous, next, jump, black, back to the
 * step). It only shows what the stage published; it never computes the show itself. On a large screen that can read
 * the workspace (signed in on this machine), the current step's picture is rendered read-only behind the panel; a
 * phone gets the panel only. The token is the only credential: it can command, watch and read notes of this show,
 * and dies with it. */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useParams } from 'react-router-dom'
import '../aiworkspace.css'
import { AiwsClient, ServiceFailure } from '../api/client'
import { OnlineWorkspaceSession } from '../api/session'
import { prompterCall, resolveTransport, TransportError } from '../api/transport'
import type { KeyedContent, Result, ShowCommand, ShowNotes, ShowPathPayload, ShowState, ShowWatch } from '../api/types'
import { loadCore } from '../api/wasm'
import { StoreContext, useOutlineVersion, WorkspaceUiContext, type WorkspaceUi } from '../state/hooks'
import { WorkspaceStore } from '../state/store'
import { BlockHost } from '../ui/blocks/BlockHost'
import { registerDefaultBlocks } from '../ui/blocks/registerAll'
import { command } from './controller'
import { playable, resolveSteps, fitStage } from './model'
import { StageCanvas } from './StageCanvas'
import './presentation.css'

const WATCH_MS = 25_000
const FONT_KEY = 'aiworkspace.prompter-font'
/** A screen at least this wide (CSS px) may show the picture behind the panel. */
const LARGE_SCREEN = 1100

function tokenOf(hash: string): string | null {
  const k = new URLSearchParams(hash.replace(/^#/, '')).get('k')
  return k && /^pt_[a-z2-7]{26}$/.test(k) ? k : null
}

async function call<T>(method: string, params: Record<string, unknown>, token: string, signal?: AbortSignal): Promise<T> {
  const result = await prompterCall<Result<T>>(method, params, token, signal)
  if (result && result.ok) return result as T
  throw new ServiceFailure(result?.error ?? { code: 'UNKNOWN', detail: 'malformed response' })
}

function ended(error: unknown): boolean {
  return error instanceof ServiceFailure && error.code === 'NOT_FOUND'
}

function clock(since: string, now: number): string {
  const start = Date.parse(since)
  if (!Number.isFinite(start)) return ''
  const s = Math.max(0, Math.floor((now - start) / 1000))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const pad = (n: number) => String(n).padStart(2, '0')
  return h ? `${h}:${pad(m)}:${pad(s % 60)}` : `${pad(m)}:${pad(s % 60)}`
}

export function PrompterRoute() {
  const { workspaceId = '', showId = '' } = useParams()
  const [token] = useState(() => tokenOf(window.location.hash))
  if (!token) {
    return (
      <div className="aiws-root aiws-prompter" data-testid="aiws-prompter">
        <div className="aiws-prompter-ended" data-testid="aiws-prompter-invalid"><b>提示器链接不完整</b><span className="aiws-muted">请从舞台的“复制提示器链接”重新获取完整链接（# 之后的部分不能缺少）。</span></div>
      </div>
    )
  }
  return <Prompter workspaceId={workspaceId} showId={showId} token={token} />
}

function Prompter({ workspaceId, showId, token }: { workspaceId: string; showId: string; token: string }) {
  const params = useMemo(() => ({ workspace_id: workspaceId, show_id: showId }), [workspaceId, showId])
  const [notes, setNotes] = useState<ShowNotes | null>(null)
  const [state, setState] = useState<ShowState | null>(null)
  const [over, setOver] = useState(false)
  const [trouble, setTrouble] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [font, setFont] = useState(() => { try { return Number(window.localStorage.getItem(FONT_KEY)) || 24 } catch { return 24 } })
  const [list, setList] = useState(false)
  const seq = useRef(0)

  useEffect(() => {
    let live = true
    call<ShowNotes>('show.notes', params, token).then((n) => { if (live) setNotes(n) }, (error: unknown) => { if (live) { if (ended(error)) setOver(true); else setTrouble(String(error instanceof Error ? error.message : error)) } })
    return () => { live = false }
  }, [params, token])

  // the newest state, by long polling
  useEffect(() => {
    let live = true
    const abort = new AbortController()
    void (async () => {
      let backoff = 500
      while (live) {
        try {
          const page = await call<ShowWatch>('show.watch', { ...params, after_seq: seq.current, timeout_ms: WATCH_MS }, token, abort.signal)
          backoff = 500
          setTrouble(null)
          if (page.seq > seq.current && 'step_id' in page.state) { seq.current = page.seq; setState(page.state as ShowState) }
        } catch (error) {
          if (!live) return
          if (ended(error)) { setOver(true); return }
          if (!(error instanceof TransportError) && !(error instanceof ServiceFailure)) throw error
          setTrouble('连接中断，正在重连…')
          await new Promise((resolve) => window.setTimeout(resolve, backoff))
          backoff = Math.min(backoff * 2, 8000)
        }
      }
    })()
    return () => { live = false; abort.abort() }
  }, [params, token])

  useEffect(() => { const t = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(t) }, [])
  useEffect(() => { try { window.localStorage.setItem(FONT_KEY, String(font)) } catch { /* per device only */ } }, [font])

  const send = useCallback((cmd: ShowCommand) => {
    void call<{ cursor: number }>('show.command', { ...params, command: cmd }, token).catch((error: unknown) => { if (ended(error)) setOver(true); else setTrouble('命令没有送达，请重试') })
  }, [params, token])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.target instanceof Element ? event.target : null)?.closest('input, textarea, select')) return
      if (['ArrowRight', 'ArrowDown', 'PageDown', ' '].includes(event.key)) { event.preventDefault(); send(command('next')) }
      else if (['ArrowLeft', 'ArrowUp', 'PageUp'].includes(event.key)) { event.preventDefault(); send(command('prev')) }
      else if (event.key === 'b' || event.key === 'B' || event.key === '.') send(command('black'))
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [send])

  const large = typeof window !== 'undefined' && Math.min(window.innerWidth, window.screen?.width ?? window.innerWidth) >= LARGE_SCREEN
  if (over) {
    return (
      <div className="aiws-root aiws-prompter" data-testid="aiws-prompter">
        <div className="aiws-prompter-ended" data-testid="aiws-prompter-ended"><b>放映已结束</b><span className="aiws-muted">这个提示器链接已失效。</span></div>
      </div>
    )
  }
  const steps = (notes?.steps ?? []).filter((s) => s.enabled && !s.missing)
  const currentId = state?.step_id ?? null
  const current = notes?.steps.find((s) => s.id === currentId) ?? null
  const position = state ? state.index : -1
  const next = position >= 0 ? steps[position + 1] ?? null : null
  return (
    <div className={`aiws-root aiws-prompter${large ? ' has-stage' : ''}`} data-testid="aiws-prompter" data-step={currentId ?? undefined} data-index={position}>
      {large && state && <PrompterStage workspaceId={workspaceId} state={state} />}
      <div className="aiws-prompter-panel">
        <div className="aiws-prompter-head">
          <span className="aiws-prompter-title" data-testid="aiws-prompter-title">{current?.title ?? (state ? '' : '等待舞台…')}</span>
          <span className="aiws-muted" data-testid="aiws-prompter-count">{state ? `${state.index + 1} / ${state.count}` : ''}</span>
          <span className="aiws-prompter-clock" data-testid="aiws-prompter-clock">{state ? clock(state.started_at, now) : ''}</span>
        </div>
        {state?.black && <div className="aiws-warning">舞台黑屏中</div>}
        {state?.free && <div className="aiws-warning">舞台正在自由浏览</div>}
        {trouble && <div className="aiws-warning" role="status">{trouble}</div>}
        <div className="aiws-prompter-notes" data-testid="aiws-prompter-notes" style={{ fontSize: font }}>{current?.notes || <span className="aiws-muted">这一步没有讲解备注。</span>}</div>
        <div className="aiws-prompter-next" data-testid="aiws-prompter-next">{next ? `下一步：${next.title ?? ''}` : state ? '这是最后一步' : ''}</div>
        <div className="aiws-prompter-controls">
          <button type="button" data-testid="aiws-prompter-prev" disabled={!state || state.index === 0} onClick={() => send(command('prev'))}>上一项</button>
          <button type="button" className="is-primary" data-testid="aiws-prompter-next-button" disabled={!state || state.index >= state.count - 1} onClick={() => send(command('next'))}>下一项</button>
        </div>
        <div className="aiws-prompter-row">
          <button type="button" data-testid="aiws-prompter-black" aria-pressed={state?.black ?? false} onClick={() => send(command('black'))}>{state?.black ? '取消黑屏' : '黑屏'}</button>
          <button type="button" data-testid="aiws-prompter-back" disabled={!state?.free} onClick={() => send(command('back'))}>回到本步骤</button>
          <button type="button" data-testid="aiws-prompter-list" aria-pressed={list} onClick={() => setList(!list)}>目录</button>
          <button type="button" aria-label="缩小字号" onClick={() => setFont(Math.max(14, font - 2))}>A−</button>
          <button type="button" aria-label="放大字号" onClick={() => setFont(Math.min(64, font + 2))}>A+</button>
        </div>
        {list && (
          <div className="aiws-prompter-steps" data-testid="aiws-prompter-steps">
            {steps.map((s, i) => <button key={s.id} type="button" className={s.id === currentId ? 'is-current' : ''} onClick={() => { send(command('goto', { step_id: s.id })); setList(false) }}>{i + 1}. {s.title ?? ''}</button>)}
          </div>
        )}
      </div>
    </div>
  )
}

/** The current step's picture behind the panel, when this browser may read the workspace (§11.2). */
function PrompterStage({ workspaceId, state }: { workspaceId: string; state: ShowState }) {
  const [store, setStore] = useState<WorkspaceStore | null>(null)
  useEffect(() => {
    let live = true
    let opened: WorkspaceStore | null = null
    const availability = resolveTransport()
    if (!availability.ok) return
    void (async () => {
      try {
        registerDefaultBlocks()
        const client = new AiwsClient(availability.transport)
        const [core, session] = await Promise.all([loadCore(), OnlineWorkspaceSession.open(client, workspaceId)])
        opened = new WorkspaceStore(session, core)
        await opened.outline.reload()
        if (live) setStore(opened)
        else void opened.dispose()
      } catch {
        // not signed in here, or no access: the panel alone
      }
    })()
    return () => { live = false; if (opened) void opened.dispose() }
  }, [workspaceId])
  if (!store) return null
  return (
    <StoreContext.Provider value={store}>
      <PrompterUi store={store}><PassiveStage store={store} state={state} /></PrompterUi>
    </StoreContext.Provider>
  )
}

function PrompterUi({ store, children }: { store: WorkspaceStore; children: React.ReactNode }) {
  const outlineVersion = useOutlineVersion()
  const ui = useMemo<WorkspaceUi>(() => {
    const entities = store.outline.all()
    return {
      entities, byId: new Map(entities.map((e) => [e.entity_id, e])), annotations: [], openEntity: () => undefined, annotate: null,
      activeAnnotation: null, setActiveAnnotation: () => undefined, annotationFilter: null, showAnnotations: () => undefined,
      renderCell: (cellId, depth) => <BlockHost cellId={cellId} mode="view" view="source" depth={depth} />,
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
  }, [store, outlineVersion])
  return <WorkspaceUiContext.Provider value={ui}>{children}</WorkspaceUiContext.Provider>
}

function PassiveStage({ store, state }: { store: WorkspaceStore; state: ShowState }) {
  const outlineVersion = useOutlineVersion()
  const [path, setPath] = useState<ShowPathPayload | null>(null)
  useEffect(() => {
    let live = true
    store.session.read<KeyedContent<ShowPathPayload>>(state.path_id).then((r) => { if (live) setPath(r.content.payload) }, () => undefined)
    return () => { live = false }
  }, [store, state.path_id])
  const steps = useMemo(() => (path ? playable(resolveSteps(store.outline, path)) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
    [path, store, outlineVersion])
  const index = steps.findIndex((s) => s.step.id === state.step_id)
  const current = steps[index] ?? null
  const ref = useRef<HTMLDivElement>(null)
  const [nonce, setNonce] = useState(0)
  const [seen, setSeen] = useState<string | null>(null)
  if (current && seen !== current.step.id) { setSeen(current.step.id); setNonce(nonce + 1) }
  if (!path || !current) return null
  const pointer = state.pointer
  return (
    <div ref={ref} className="aiws-prompter-stage" data-testid="aiws-prompter-stage">
      <StageCanvas passive stage={path.stage} background={path.background} current={current} next={steps[index + 1] ?? null} transition="cut" nonce={nonce} free={false} black={state.black}
        overMask={pointer ? <PrompterPointer pointer={pointer} stage={path.stage} /> : null} />
    </div>
  )
}

function PrompterPointer({ pointer, stage }: { pointer: { x: number; y: number }; stage: { w: number; h: number } }) {
  const [size, setSize] = useState({ w: window.innerWidth, h: window.innerHeight })
  useEffect(() => {
    const onResize = () => setSize({ w: window.innerWidth, h: window.innerHeight })
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])
  const s = fitStage({ x: 0, y: 0, w: size.w, h: size.h }, stage)
  return <div className="aiws-prompter-pointer" style={{ left: s.x + pointer.x * s.w, top: s.y + pointer.y * s.h }} />
}
