/* The view toolbar, top right (UI improvement §8.1, §9.1; renamed from the "presenter toolbar" in 第三期规划 §2, which
 * keeps "提示器" for the show's prompter): add annotation | guide | show | zoom ▾ | identity and online members | share
 * link. Zoom and view navigation live only here (and in the main menu): there is no bottom-right navigation area.
 * Interactions are not shown — there is no global interaction system yet — and online members only appear once
 * presence exists. The guide (§7.3) appears when the workspace has a guide path, the show button (§7.2) when it has a
 * presentation path. On a phone (§16) it is the only toolbar and starts with the canvas switcher (icon, name, the
 * workspace's canvases). */

import { useCallback, useEffect, useState } from 'react'
import { Globe, Link2, LogIn, LogOut, MessageSquarePlus, Minus, Play, Plus, Signpost } from 'lucide-react'
import { describeError } from '../../api/session'
import type { GrantList, Json } from '../../api/types'
import { pathsOf } from '../../presentation/model'
import { guestLabel, readGuestName, writeGuestName } from '../../state/guest'
import { useLoad, useOutlineVersion, useStore } from '../../state/hooks'
import type { Camera } from '../canvas/render/camera'
import { MAX_ZOOM, MIN_ZOOM } from '../canvas/render/camera'
import { SurfaceSwitcher } from './MainToolbar'
import { MenuList, PopoverPanel, usePopover, type MenuItem } from './popover'
import { PUBLIC_LINK_CAPABILITIES, PUBLIC_SUBJECT, shareLink, useShell, ZOOM_PRESETS } from './shellContext'
import { ObjectImage } from '../../../../presentation/ObjectIcon'
import { useCurrentUser } from '../../../../presentation/currentUser'

export function ViewToolbar({ camera, hasSelection, onFitAll, onFitSelection, annotate }: {
  /** Free Surfaces only: flow pages have no camera, so no zoom. */
  camera: Camera | null
  hasSelection: boolean
  onFitAll: () => void
  onFitSelection: () => void
  /** Absent where there is nothing to annotate (a workspace without canvases). `note`: a free canvas also takes notes
   * from here (a note is an annotation), so the button opens a choice. */
  annotate: { reason: string | null; active: boolean; run: () => void; note?: { reason: string | null; run: () => void } | null } | null
}) {
  const shell = useShell()
  const compact = shell.size === 'narrow' || shell.phone
  return (
    <div className="aiws-panel aiws-view-toolbar" role="toolbar" aria-label={shell.phone ? '画布工具' : '视图工具'} data-testid="aiws-view-toolbar">
      {shell.phone && <SurfaceSwitcher phone />}
      <GuideButton compact={compact} />
      {!shell.phone && <ShowButton />}
      {annotate && <AnnotateButton annotate={annotate} compact={compact} />}
      {camera && <ZoomControl camera={camera} hasSelection={hasSelection} onFitAll={onFitAll} onFitSelection={onFitSelection} />}
      <IdentityButton />
      <ShareButton />
    </div>
  )
}

/** Add an annotation; on a free canvas a menu first offers the other kind of annotation, a note. While a target is
 * being picked the button cancels the pick. */
function AnnotateButton({ annotate, compact }: { annotate: NonNullable<Parameters<typeof ViewToolbar>[0]['annotate']>; compact: boolean }) {
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const note = annotate.note
  const menu = Boolean(note) && !annotate.active
  const items: MenuItem[] = note ? [
    { id: 'annotate-object', testId: 'aiws-annotate-object', label: '批注对象', disabled: annotate.reason !== null, reason: annotate.reason, run: annotate.run },
    { id: 'annotate-note', testId: 'aiws-annotate-note', label: '添加便签', disabled: note.reason !== null, reason: note.reason, explain: true, run: note.run },
  ] : []
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={`aiws-tool${compact ? '' : ' aiws-tool-text'}`} aria-pressed={annotate.active} disabled={annotate.reason !== null && note?.reason !== null}
        data-testid="aiws-annotate-start" aria-haspopup={menu ? 'menu' : undefined} aria-expanded={menu ? open : undefined}
        title={annotate.active ? '点选要批注的对象（Esc 取消）' : menu ? '添加批注或便签' : annotate.reason ?? '添加批注：选中对象后批注；未选中时先点选目标'} aria-label="添加批注"
        onClick={menu ? toggle : annotate.run}>
        <MessageSquarePlus size={18} />{!compact && <span>批注</span>}
      </button>
      {open && menu && <MenuList items={items} label="添加批注" className="aiws-popover aiws-popover-end" onDone={close} />}
    </span>
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

/** "使用引导" (§7.3): one guide starts at once (where the user left it), several are offered in a list. */
function GuideButton({ compact }: { compact: boolean }) {
  const store = useStore()
  const shell = useShell()
  useOutlineVersion()
  const { open, toggle, close, bindAnchor, bindTrigger } = usePopover()
  const guides = pathsOf(store.outline, 'guide')
  if (guides.length === 0) return null
  const begin = (pathId: string) => {
    const saved = store.userState.get<Json>(`guide:${pathId}`) as { index?: number; done?: boolean } | undefined
    shell.setGuide({ pathId, index: saved && !saved.done ? saved.index ?? 0 : 0 })
  }
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={`aiws-tool${compact ? '' : ' aiws-tool-text'}`} aria-label="使用引导" title="使用引导：逐步看这张画布怎么用" data-testid="aiws-guide-start"
        aria-pressed={shell.guide !== null} onClick={() => (guides.length === 1 ? begin(guides[0].entity_id) : toggle())}>
        <Signpost size={18} />{!compact && <span>引导</span>}
      </button>
      {open && (
        <PopoverPanel align="end" label="使用引导" testId="aiws-guide-list">
          {guides.map((g) => <button key={g.entity_id} type="button" className="aiws-popover-item" onClick={() => { close(); begin(g.entity_id) }}>{g.title ?? g.name ?? g.entity_id}</button>)}
        </PopoverPanel>
      )}
    </span>
  )
}

/** "开始放映" (§7.2): shown when the workspace has a presentation path. */
function ShowButton() {
  const store = useStore()
  const shell = useShell()
  useOutlineVersion()
  if (pathsOf(store.outline, 'presentation').length === 0) return null
  return (
    <button type="button" className="aiws-tool" aria-label="开始放映" title="开始放映…" data-testid="aiws-show-start" onClick={() => shell.startShow()}>
      <Play size={18} />
    </button>
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
  const guest = shell.identity.guest
  const me = useCurrentUser()
  const [guestName, setGuestName] = useState(readGuestName)
  // A Zone account's own picture; a development token identity has none.
  const picture = !shell.identity.dev && !guest && me && me.userId === principal ? me.avatarUrl : undefined
  const shown = guest ? guestName || guestLabel(principal ?? '') : me && me.userId === principal ? me.displayName : principal ?? '未知身份'
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className="aiws-avatar" aria-haspopup="dialog" aria-expanded={open} aria-label={`当前身份：${guest ? '访客' : principal ?? '未知'}`} title={guest ? '访客（通过公开链接访问）' : principal ?? '当前身份'}
        data-testid="aiws-principal" data-principal={principal ?? ''} data-guest={guest ? 'true' : undefined} onClick={toggle}>
        <ObjectImage icons={[picture]} className="aiws-avatar-img" fallback={guest ? '访' : initial(principal)} />
      </button>
      {open && (
        <PopoverPanel align="end" label="身份与在线成员" testId="aiws-identity-panel" className="aiws-identity-panel">
          <div className="aiws-panel-title">我的身份</div>
          <div className="aiws-identity-row"><span className="aiws-avatar is-static" aria-hidden="true"><ObjectImage icons={[picture]} className="aiws-avatar-img" fallback={guest ? '访' : initial(principal)} /></span><b data-testid="aiws-identity-name">{shown}</b></div>
          {guest ? (
            <>
              <div className="aiws-muted">通过公开链接访问，无需登录：可以查看和批注，不能修改内容。批注以下面的称呼署名。</div>
              <label className="aiws-guest-name">称呼
                <input value={guestName} maxLength={32} placeholder={guestLabel(principal ?? '')} aria-label="称呼" data-testid="aiws-guest-name"
                  onChange={(event) => { setGuestName(event.target.value); writeGuestName(event.target.value) }} />
              </label>
              <button type="button" className="aiws-popover-item" data-testid="aiws-login" onClick={() => { close(); shell.login() }}>
                <LogIn size={16} /> 登录 BuckyOS
              </button>
            </>
          ) : (
            <>
              <div className="aiws-muted">{shell.identity.dev ? '开发直连身份：由本地测试令牌指定，不是 Zone 账号，不能在这里登出。' : 'Zone 账号'}</div>
              <div className="aiws-panel-title">在线成员</div>
              <div className="aiws-muted" data-testid="aiws-presence-unavailable">在线状态尚未接入：协作面板中的授权名单不等于在线成员。</div>
              <button type="button" className="aiws-popover-item" data-testid="aiws-logout" disabled={shell.identity.dev} title={shell.identity.dev ? '开发直连身份没有登录会话' : undefined}
                onClick={() => { close(); shell.logout() }}>
                <LogOut size={16} /> 退出登录
              </button>
            </>
          )}
        </PopoverPanel>
      )}
    </span>
  )
}


/** The share panel (§8.3): the access link, and — for a manager — the one-click public link: a `public` grant
 * (read + comment) that lets anyone with the link open it and annotate without signing in, until it is withdrawn. */
function ShareButton() {
  const store = useStore()
  const shell = useShell()
  const { open, setOpen, close, bindAnchor, bindTrigger } = usePopover()
  const [copied, setCopied] = useState<'copied' | 'manual' | null>(null)
  const [tick, setTick] = useState(0)
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const link = shareLink(store.session.workspaceId, shell.topMode === 'canvas' ? shell.activeSurface?.entity_id ?? null : null)
  const where = shell.topMode === 'canvas' && shell.activeSurface ? `画布「${shell.activeSurface.title ?? shell.activeSurface.name ?? ''}」` : '这个工作区'
  const canManage = store.session.info().capabilities.includes('manage')
  // the grants say whether the link is public; read when the panel opens, again after each change
  const loadGrants = useCallback(() => (open ? store.session.listGrants() : Promise.resolve(null as GrantList | null)), [store, open])
  const grants = useLoad(loadGrants, tick)
  const isPublic = (grants.data?.grants ?? []).some((g) => g.subject === PUBLIC_SUBJECT && !g.scope_entity_id)
  const copy = async () => {
    try { await navigator.clipboard.writeText(link); setCopied('copied') } catch { setCopied('manual') }
  }
  const change = (label: string, work: () => Promise<void>) => {
    setBusy(true)
    setFailure(null)
    work().then(copy).catch((error: unknown) => setFailure(`${label}失败：${describeError(error)}`)).finally(() => { setBusy(false); setTick((n) => n + 1) })
  }
  const publish = () => change('公开', () => store.session.grant(PUBLIC_SUBJECT, [...PUBLIC_LINK_CAPABILITIES]))
  const unpublish = () => change('停止公开', () => store.session.revoke(PUBLIC_SUBJECT))
  return (
    <span ref={bindAnchor} className="aiws-anchor">
      <button ref={bindTrigger} type="button" className={`aiws-tool${shell.size === 'narrow' || shell.phone ? '' : ' aiws-tool-text'} is-primary`} aria-label="分享链接" title="复制访问链接" data-testid="aiws-share" onClick={() => { setOpen(true); void copy() }}>
        <Link2 size={18} />{shell.size !== 'narrow' && !shell.phone && <span>分享</span>}
      </button>
      {open && (
        <PopoverPanel align="end" label="分享链接" testId="aiws-share-panel" className="aiws-share-panel">
          <div className="aiws-panel-title">{copied === 'copied' ? (isPublic ? '已复制公开链接' : '已复制访问链接') : '访问链接'}</div>
          <input readOnly aria-label="访问链接" value={link} data-testid="aiws-share-link" onFocus={(event) => event.target.select()} />
          {copied === 'manual' && <div className="aiws-muted">浏览器不允许自动复制，请手动复制上面的链接。</div>}
          {grants.data && (isPublic ? (
            <div className="aiws-share-public" data-testid="aiws-share-public-state" data-public="true">
              <Globe size={16} aria-hidden="true" />
              <span>公开链接已开启：任何拿到链接的人都能打开{where}并添加批注，无需登录；不能修改内容。</span>
            </div>
          ) : (
            <div className="aiws-muted" data-testid="aiws-share-public-state" data-public="false">链接定位到{where}。它不包含登录凭据，也不会授予权限：接收者需要登录，并且已有访问权限。</div>
          ))}
          {grants.error && <div className="aiws-error" role="alert">{grants.error}</div>}
          {failure && <div className="aiws-error" role="alert" data-testid="aiws-share-error">{failure}</div>}
          <div className="aiws-dialog-actions">
            {canManage && (isPublic
              ? <button type="button" data-testid="aiws-share-unpublish" disabled={busy || grants.loading} onClick={unpublish}>停止公开</button>
              : <button type="button" className="is-primary" data-testid="aiws-share-publish" disabled={busy || grants.loading} title="任何拿到链接的人都能打开并批注，无需登录" onClick={publish}><Globe size={16} aria-hidden="true" /> 一键公开分享</button>)}
            {canManage && <button type="button" data-testid="aiws-share-collab" onClick={() => { close(); shell.setSide('collab') }}>在协作中授权…</button>}
          </div>
          {!shell.identity.guest && <div className="aiws-muted">导出分享包请用主菜单的“导出…”。</div>}
        </PopoverPanel>
      )}
    </span>
  )
}
