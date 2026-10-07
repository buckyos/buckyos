/* The presenter toolbar, top right (UI improvement §8.1, §9.1): add annotation | interactions | zoom ▾ |
 * identity and online members | share link. Zoom and view navigation live only here (and in the main
 * menu): there is no bottom-right navigation area. Interactions are not shown — there is no global
 * interaction system yet — and online members only appear once presence exists. */

import { useEffect, useState } from 'react'
import { Link2, LogOut, MessageSquarePlus, Minus, Plus } from 'lucide-react'
import { useStore } from '../../state/hooks'
import type { Camera } from '../canvas/render/camera'
import { MAX_ZOOM, MIN_ZOOM } from '../canvas/render/camera'
import { PopoverPanel, usePopover } from './popover'
import { shareLink, useShell, ZOOM_PRESETS } from './shellContext'

export function PresenterToolbar({ camera, hasSelection, onFitAll, onFitSelection, annotate }: {
  /** Free Surfaces only: flow pages have no camera, so no zoom. */
  camera: Camera | null
  hasSelection: boolean
  onFitAll: () => void
  onFitSelection: () => void
  annotate: { reason: string | null; active: boolean; run: () => void }
}) {
  const shell = useShell()
  return (
    <div className="aiws-panel aiws-presenter-toolbar" role="toolbar" aria-label="演讲工具" data-testid="aiws-presenter-toolbar">
      <button type="button" className={`aiws-tool${shell.size === 'narrow' ? '' : ' aiws-tool-text'}`} aria-pressed={annotate.active} disabled={annotate.reason !== null} data-testid="aiws-annotate-start"
        title={annotate.reason ?? (annotate.active ? '点选要批注的对象（Esc 取消）' : '添加批注：选中对象后批注；未选中时先点选目标')} aria-label="添加批注" onClick={annotate.run}>
        <MessageSquarePlus size={18} />{shell.size !== 'narrow' && <span>批注</span>}
      </button>
      {camera && <ZoomControl camera={camera} hasSelection={hasSelection} onFitAll={onFitAll} onFitSelection={onFitSelection} />}
      <IdentityButton />
      <ShareButton />
    </div>
  )
}

function useZoom(camera: Camera): number {
  const [zoom, setZoom] = useState(camera.zoom)
  useEffect(() => camera.onChange(() => setZoom(camera.zoom)), [camera])
  return zoom
}

export function ZoomControl({ camera, hasSelection, onFitAll, onFitSelection }: { camera: Camera; hasSelection: boolean; onFitAll: () => void; onFitSelection: () => void }) {
  const zoom = useZoom(camera)
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const [custom, setCustom] = useState('')
  const percent = Math.round(zoom * 100)
  const apply = () => {
    const value = Number(custom.replace('%', ''))
    if (Number.isFinite(value) && value > 0) camera.zoomTo(value / 100)
    setCustom('')
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-tool aiws-tool-text aiws-zoom-button" aria-haspopup="dialog" aria-expanded={open} aria-label={`缩放比例 ${percent}%`} title="缩放与视图导航" data-testid="aiws-zoom-menu" onClick={toggle}>
        <span data-testid="aiws-zoom">{percent}%</span>
      </button>
      {open && (
        <PopoverPanel align="end" label="缩放与视图导航" testId="aiws-zoom-panel" className="aiws-zoom-panel">
          <div className="aiws-zoom-row">
            <button type="button" className="aiws-tool" aria-label="缩小" title="缩小" data-testid="aiws-zoom-out" disabled={zoom <= MIN_ZOOM + 1e-6} onClick={() => camera.zoomTo(camera.zoom / 1.25)}><Minus size={16} /></button>
            <span className="aiws-zoom-value">{percent}%</span>
            <button type="button" className="aiws-tool" aria-label="放大" title="放大" data-testid="aiws-zoom-in" disabled={zoom >= MAX_ZOOM - 1e-6} onClick={() => camera.zoomTo(camera.zoom * 1.25)}><Plus size={16} /></button>
          </div>
          <div className="aiws-zoom-presets">
            {ZOOM_PRESETS.map((value) => <button key={value} type="button" aria-pressed={percent === value} data-testid={`aiws-zoom-${value}`} onClick={() => camera.zoomTo(value / 100)}>{value}%</button>)}
          </div>
          <form className="aiws-zoom-custom" onSubmit={(event) => { event.preventDefault(); apply() }}>
            <input aria-label="自定义比例（%）" placeholder={`${Math.round(MIN_ZOOM * 100)}–${Math.round(MAX_ZOOM * 100)}%`} value={custom} inputMode="numeric" onChange={(event) => setCustom(event.target.value)} data-testid="aiws-zoom-input" />
            <button type="submit">应用</button>
          </form>
          <button type="button" className="aiws-popover-item" data-testid="aiws-fit-all" onClick={() => { onFitAll(); close() }}>适应全部</button>
          <button type="button" className="aiws-popover-item" data-testid="aiws-fit-selection" disabled={!hasSelection} title={hasSelection ? undefined : '没有选中的对象'} onClick={() => { onFitSelection(); close() }}>适应选区</button>
        </PopoverPanel>
      )}
    </span>
  )
}

function initial(name: string | null): string {
  if (!name) return '?'
  const trimmed = name.replace(/^did:[^:]+:/, '')
  return (trimmed[0] ?? '?').toUpperCase()
}

function IdentityButton() {
  const store = useStore()
  const shell = useShell()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const principal = store.session.principal
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-avatar" aria-haspopup="dialog" aria-expanded={open} aria-label={`当前身份：${principal ?? '未知'}`} title={principal ?? '当前身份'}
        data-testid="aiws-principal" data-principal={principal ?? ''} onClick={toggle}>
        {initial(principal)}
      </button>
      {open && (
        <PopoverPanel align="end" label="身份与在线成员" testId="aiws-identity-panel" className="aiws-identity-panel">
          <div className="aiws-panel-title">我的身份</div>
          <div className="aiws-identity-row"><span className="aiws-avatar is-static" aria-hidden="true">{initial(principal)}</span><b>{principal ?? '未知身份'}</b></div>
          <div className="aiws-muted">{shell.identity.dev ? '开发直连身份：由本地测试令牌指定，不是 Zone 账号，不能在这里登出。' : 'Zone 账号'}</div>
          <div className="aiws-panel-title">在线成员</div>
          <div className="aiws-muted" data-testid="aiws-presence-unavailable">在线状态尚未接入：协作面板中的授权名单不等于在线成员。</div>
          <button type="button" className="aiws-popover-item" data-testid="aiws-logout" disabled={shell.identity.dev} title={shell.identity.dev ? '开发直连身份没有登录会话' : undefined}
            onClick={() => { close(); shell.logout() }}>
            <LogOut size={16} /> 退出登录
          </button>
        </PopoverPanel>
      )}
    </span>
  )
}

function ShareButton() {
  const store = useStore()
  const shell = useShell()
  const { open, setOpen, close, bindAnchor, bindTrigger } = usePopover()
  const [copied, setCopied] = useState<'copied' | 'manual' | null>(null)
  const link = shareLink(store.session.workspaceId, shell.topMode === 'canvas' ? shell.activeSurface?.entity_id ?? null : null)
  const copy = async () => {
    setOpen(true)
    try { await navigator.clipboard.writeText(link); setCopied('copied') } catch { setCopied('manual') }
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={`aiws-tool${shell.size === 'narrow' ? '' : ' aiws-tool-text'} is-primary`} aria-label="分享链接" title="复制访问链接" data-testid="aiws-share" onClick={() => { void copy() }}>
        <Link2 size={18} />{shell.size !== 'narrow' && <span>分享</span>}
      </button>
      {open && (
        <PopoverPanel align="end" label="分享链接" testId="aiws-share-panel" className="aiws-share-panel">
          <div className="aiws-panel-title">{copied === 'copied' ? '已复制访问链接' : '访问链接'}</div>
          <input readOnly aria-label="访问链接" value={link} data-testid="aiws-share-link" onFocus={(event) => event.target.select()} />
          {copied === 'manual' && <div className="aiws-muted">浏览器不允许自动复制，请手动复制上面的链接。</div>}
          <div className="aiws-muted">链接定位到{shell.topMode === 'canvas' && shell.activeSurface ? `画布「${shell.activeSurface.title ?? shell.activeSurface.name ?? ''}」` : '这个工作区'}。它不包含登录凭据，也不会授予权限：接收者需要登录，并且已有访问权限。</div>
          <div className="aiws-dialog-actions">
            <button type="button" data-testid="aiws-share-collab" onClick={() => { close(); shell.setSide('collab') }}>在协作中授权…</button>
          </div>
          <div className="aiws-muted">导出分享包请用主菜单的“导出…”。</div>
        </PopoverPanel>
      )}
    </span>
  )
}
