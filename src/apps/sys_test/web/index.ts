/**
 * sys_test browser entry point.
 *
 * Mirrors the SDK lifecycle described in ../../buckyos-websdk/SDK.md:
 *
 *   Step 1. initBuckyOS(appId)             — bind the runtime to this app
 *   Step 2. login() / logout()             — drive Browser SSO via the SDK
 *   Step 3. getXxxClient().foo()           — exercise individual ServiceClients
 *
 * The page is split into "service groups". Each group offers two run buttons:
 *   - Run tests in page → run the cases through the in-page browser SDK
 *   - Run tests on backend service → POST /sdk/appservice/selftest so the AppService
 *     runtime in main.ts runs the same cases on the server side
 *
 * Each group also leaves a placeholder area for future per-service manual
 * call panels (point 4 of the task brief).
 */
import { buckyos, ndm, RuntimeType } from 'buckyos'
import { NfspClient } from 'buckyos/nfsp'
import { TEST_GROUPS, TestCase, TestContext, TestGroup } from './src/test_groups'
import { renderSupportGroupPanel } from './src/support_group'

const APP_ID = 'buckyos-systest.buckyos.bns.did'
const SELFTEST_BASE_URL = '/sdk/appservice/selftest'

type CaseResult = {
  name: string
  ok: boolean
  durationMs: number
  error?: string
  details?: Record<string, unknown> | null
}

type RunOrigin = 'in-page' | 'in-server'

type AuthState =
  | { kind: 'init' }
  | { kind: 'logged-out' }
  | { kind: 'logged-in'; userId: string; userName: string }
  | { kind: 'error'; message: string }

let authState: AuthState = { kind: 'init' }

function $<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id)
  if (!el) {
    throw new Error(`element #${id} not found`)
  }
  return el as T
}

function setAuthStatus(state: AuthState) {
  authState = state
  const status = $('auth-status') as HTMLSpanElement
  const loginBtn = $('btn-login') as HTMLButtonElement
  const logoutBtn = $('btn-logout') as HTMLButtonElement

  status.classList.remove('ok', 'err')
  switch (state.kind) {
    case 'init':
      status.textContent = '正在初始化 SDK...'
      loginBtn.disabled = true
      logoutBtn.disabled = true
      break
    case 'logged-out':
      status.textContent = '未登录'
      loginBtn.disabled = false
      logoutBtn.disabled = true
      break
    case 'logged-in':
      status.textContent = `已登录: ${state.userName} (${state.userId})`
      status.classList.add('ok')
      loginBtn.disabled = true
      logoutBtn.disabled = false
      break
    case 'error':
      status.textContent = `错误: ${state.message}`
      status.classList.add('err')
      loginBtn.disabled = false
      logoutBtn.disabled = true
      break
  }
}

async function refreshAuthFromSdk(): Promise<void> {
  try {
    const accountInfo = await buckyos.getAccountInfo()
    if (accountInfo && accountInfo.user_id) {
      setAuthStatus({
        kind: 'logged-in',
        userId: accountInfo.user_id,
        userName: accountInfo.user_name ?? accountInfo.user_id,
      })
    } else {
      setAuthStatus({ kind: 'logged-out' })
    }
  } catch (error) {
    setAuthStatus({
      kind: 'error',
      message: error instanceof Error ? error.message : String(error),
    })
  }
}

async function ensureLoggedInContext(): Promise<TestContext> {
  if (authState.kind !== 'logged-in') {
    await refreshAuthFromSdk()
  }
  if (authState.kind !== 'logged-in') {
    throw new Error('not logged in — click Login to start an SSO session first')
  }
  return {
    sdk: buckyos,
    userId: authState.userId,
    appId: APP_ID,
  }
}

async function runCaseInPage(testCase: TestCase, ctx: TestContext): Promise<CaseResult> {
  const startedAt = Date.now()
  try {
    const details = (await testCase.run(ctx)) ?? null
    return {
      name: testCase.name,
      ok: true,
      durationMs: Date.now() - startedAt,
      details,
    }
  } catch (error) {
    return {
      name: testCase.name,
      ok: false,
      durationMs: Date.now() - startedAt,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function runGroupInPage(group: TestGroup): Promise<CaseResult[]> {
  const ctx = await ensureLoggedInContext()
  const results: CaseResult[] = []
  for (const testCase of group.cases) {
    results.push(await runCaseInPage(testCase, ctx))
  }
  return results
}

interface SelftestResponse {
  ok: boolean
  group?: string
  appId?: string
  ownerUserId?: string
  results: CaseResult[]
  error?: string
}

async function runGroupOnServer(group: TestGroup): Promise<CaseResult[]> {
  // Each group has its own backend endpoint, e.g.
  //   POST /sdk/appservice/selftest/system_config
  // See main.ts for the route table.
  const url = `${SELFTEST_BASE_URL}/${group.id}`
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: '{}',
  })
  const text = await response.text()
  let payload: SelftestResponse
  try {
    payload = JSON.parse(text) as SelftestResponse
  } catch {
    throw new Error(
      `non-json response from ${url} (status=${response.status}): ${text.slice(0, 200)}`,
    )
  }
  if (!Array.isArray(payload?.results) || payload.results.length === 0) {
    throw new Error(
      payload?.error ?? `selftest endpoint returned status ${response.status}`,
    )
  }
  return payload.results
}

function renderResults(container: HTMLElement, origin: RunOrigin, results: CaseResult[] | null, error?: string) {
  // Replace any prior block for this origin only, keeping the other side visible.
  const existing = container.querySelector(`.run-block[data-origin="${origin}"]`)
  if (existing) {
    existing.remove()
  }

  const block = document.createElement('div')
  block.className = 'run-block'
  block.dataset.origin = origin

  const header = document.createElement('div')
  header.className = 'run-header'
  const badge = document.createElement('span')
  badge.className = `badge ${origin}`
  badge.textContent = origin === 'in-page' ? '在页面中运行' : '在后台服务中运行'
  header.appendChild(badge)

  const ts = document.createElement('span')
  ts.textContent = new Date().toLocaleTimeString()
  header.appendChild(ts)

  block.appendChild(header)

  if (error) {
    const errEl = document.createElement('div')
    errEl.className = 'case err'
    errEl.innerHTML =
      '<span class="icon">✕</span>' +
      '<span class="name">运行失败</span>' +
      '<span class="duration"></span>' +
      `<div class="details error">${escapeHtml(error)}</div>`
    block.appendChild(errEl)
  } else if (results) {
    for (const result of results) {
      const caseEl = document.createElement('div')
      caseEl.className = `case ${result.ok ? 'ok' : 'err'}`
      const icon = result.ok ? '✓' : '✕'
      const detailsHtml = result.ok
        ? result.details && Object.keys(result.details).length > 0
          ? `<div class="details">${escapeHtml(JSON.stringify(result.details, null, 2))}</div>`
          : ''
        : `<div class="details error">${escapeHtml(result.error ?? 'unknown error')}</div>`
      caseEl.innerHTML =
        `<span class="icon">${icon}</span>` +
        `<span class="name">${escapeHtml(result.name)}</span>` +
        `<span class="duration">${result.durationMs}ms</span>` +
        detailsHtml
      block.appendChild(caseEl)
    }
  }

  container.appendChild(block)
}

function setRunningPlaceholder(container: HTMLElement, origin: RunOrigin) {
  const existing = container.querySelector(`.run-block[data-origin="${origin}"]`)
  if (existing) {
    existing.remove()
  }
  const block = document.createElement('div')
  block.className = 'run-block'
  block.dataset.origin = origin
  block.innerHTML =
    `<div class="run-header"><span class="badge ${origin}">${
      origin === 'in-page' ? '在页面中运行' : '在后台服务中运行'
    }</span><span>运行中...</span></div>` +
    `<div class="case run"><span class="icon">⏳</span><span class="name">running</span><span class="duration"></span></div>`
  container.appendChild(block)
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function renderGroups() {
  const container = $('groups') as HTMLElement
  container.innerHTML = ''

  for (const group of TEST_GROUPS) {
    const card = document.createElement('section')
    card.className = 'group'
    card.dataset.group = group.id

    const header = document.createElement('div')
    header.className = 'group-header'
    header.innerHTML =
      `<h2 class="group-title">${escapeHtml(group.title)}</h2>` +
      `<span class="group-id">${escapeHtml(group.id)}</span>`
    card.appendChild(header)

    const desc = document.createElement('p')
    desc.className = 'group-desc'
    desc.textContent = group.description
    card.appendChild(desc)

    const actions = document.createElement('div')
    actions.className = 'group-actions'

    const runInPage = document.createElement('button')
    runInPage.className = 'btn primary'
    runInPage.textContent = '在页面中运行检测'
    runInPage.addEventListener('click', async () => {
      runInPage.disabled = true
      try {
        setRunningPlaceholder(results, 'in-page')
        const out = await runGroupInPage(group)
        renderResults(results, 'in-page', out)
      } catch (error) {
        renderResults(results, 'in-page', null, error instanceof Error ? error.message : String(error))
      } finally {
        runInPage.disabled = false
      }
    })
    if (!group.backendOnly) actions.appendChild(runInPage)

    const runOnServer = document.createElement('button')
    runOnServer.className = 'btn secondary'
    runOnServer.textContent = '在后台服务中运行检测'
    runOnServer.addEventListener('click', async () => {
      runOnServer.disabled = true
      try {
        setRunningPlaceholder(results, 'in-server')
        const out = await runGroupOnServer(group)
        renderResults(results, 'in-server', out)
      } catch (error) {
        renderResults(results, 'in-server', null, error instanceof Error ? error.message : String(error))
      } finally {
        runOnServer.disabled = false
      }
    })
    actions.appendChild(runOnServer)

    card.appendChild(actions)

    const results = document.createElement('div')
    results.className = 'results'
    card.appendChild(results)

    // Placeholder for the future "manual call" sub-panel for each service.
    const placeholder = document.createElement('div')
    placeholder.className = 'placeholder'
    placeholder.textContent = '手动调用面板（占位）：后续将在此暴露该 service 的逐个方法调用 UI。'
    card.appendChild(placeholder)

    container.appendChild(card)
  }
}

// ── NDM Client Demo Panel ──────────────────────────────────────────

type NdmDemoState = {
  snapshot: ndm.ImportSessionSnapshot<unknown> | null
  abort: AbortController | null
  busy: boolean
  entries: NdmLogEntry[]
  committed: Set<string>
}

const ndmState: NdmDemoState = {
  snapshot: null, abort: null, busy: false, entries: [], committed: new Set(),
}

function renderNdmDemoPanel() {
  const card = document.createElement('section')
  card.className = 'group'
  card.dataset.group = 'ndm_client'
  card.innerHTML = `
    <div class="group-header"><h2 class="group-title">NDM 文件 / 目录导入</h2></div>
    <p class="group-desc">SDK：QCID 查询 → 对象标识 → 缺失内容上传 → 元数据登记 → 强依赖就绪。默认只取得 ObjId，不落盘。相同内容可再次选择以验证去重。</p>
    <div class="group-actions">
      <button id="ndm-single-btn" class="btn primary">选择文件</button>
      <button id="ndm-multi-btn" class="btn">选择多个文件</button>
      <button id="ndm-folder-btn" class="btn">选择文件夹</button>
      <button id="ndm-cancel-btn" class="btn" disabled>取消</button>
      <button id="ndm-retry-btn" class="btn" disabled>重试此任务</button>
      <button id="ndm-release-btn" class="btn" disabled>释放任务</button>
    </div>
    <p id="ndm-capability" class="group-desc"></p>
    <div class="group-actions">
      <input id="ndm-target-path" type="text" aria-label="目标父目录" placeholder="目标父目录，例如 /home/alice" />
      <button id="ndm-extract-btn" class="btn" disabled>按 ObjId 本地提取到目标目录</button>
    </div>
    <p class="group-desc">本地提取通过 NFSP 提交，服务端重新校验权限及目标冲突，不覆盖已有路径。多根目录/文件分别提交；部分失败会保留已完成项，重试仅处理未确认项。响应丢失后的冲突需检查目标，不自动覆盖。</p>
    <div id="ndm-results" class="results"></div>
  `
  $('groups').appendChild(card)
  const folderSupported = ndm.getImportProvider().getCapabilities().canPickDirectory
  $('ndm-capability').textContent = folderSupported
    ? '目录 picker 保留根目录及路径；浏览器 FileList 无法枚举空目录。支持目录节点的应用可调用 SDK importDirectory 保留空目录。'
    : '当前浏览器不支持目录选择；仍可上传单文件或多文件。'
  ;($('ndm-folder-btn') as HTMLButtonElement).disabled = !folderSupported
  ;(['single', 'multi', 'folder'] as const).forEach(kind => {
    $(`ndm-${kind}-btn`).addEventListener('click', () => void handleNdmUpload(kind === 'single' ? 'single_file' : kind === 'multi' ? 'multi_file' : 'single_dir'))
  })
  $('ndm-cancel-btn').addEventListener('click', () => {
    ndmState.abort?.abort()
    if (ndmState.snapshot) ndm.cancelImportSession(ndmState.snapshot.sessionId)
  })
  $('ndm-retry-btn').addEventListener('click', () => void handleNdmUpload())
  $('ndm-release-btn').addEventListener('click', () => {
    if (ndmState.snapshot) ndm.releaseImportSession(ndmState.snapshot.sessionId)
    ndmState.snapshot = null
    ndmState.committed.clear()
    ndmState.entries.push({ icon: '✓', label: '已释放本地任务；再次操作请重新选择', status: 'ok' })
    renderNdmLog($('ndm-results'), ndmState.entries)
    updateNdmButtons(false)
  })
  $('ndm-extract-btn').addEventListener('click', () => void extractNdmObjects())
}

function updateNdmButtons(ready: boolean) {
  for (const kind of ['single', 'multi', 'folder']) {
    ;($(`ndm-${kind}-btn`) as HTMLButtonElement).disabled = ndmState.busy || (kind === 'folder' && !ndm.getImportProvider().getCapabilities().canPickDirectory)
  }
  ;($('ndm-cancel-btn') as HTMLButtonElement).disabled = !ndmState.busy
  ;($('ndm-retry-btn') as HTMLButtonElement).disabled = ndmState.busy || !ndmState.snapshot || ready
  ;($('ndm-release-btn') as HTMLButtonElement).disabled = ndmState.busy || !ndmState.snapshot
  ;($('ndm-extract-btn') as HTMLButtonElement).disabled = ndmState.busy || !ready
}

function renderNdmLog(container: HTMLElement, entries: NdmLogEntry[]) {
  container.innerHTML = ''
  const block = document.createElement('div')
  block.className = 'run-block'

  const header = document.createElement('div')
  header.className = 'run-header'
  const badge = document.createElement('span')
  badge.className = 'badge in-page'
  badge.textContent = 'NDM 上传'
  header.appendChild(badge)
  const ts = document.createElement('span')
  ts.textContent = new Date().toLocaleTimeString()
  header.appendChild(ts)
  block.appendChild(header)

  for (const entry of entries) {
    const el = document.createElement('div')
    el.className = `case ${entry.status}`
    el.innerHTML =
      `<span class="icon">${entry.icon}</span>` +
      `<span class="name">${escapeHtml(entry.label)}</span>` +
      `<span class="duration">${entry.extra ?? ''}</span>` +
      (entry.detail ? `<div class="details${entry.status === 'err' ? ' error' : ''}">${escapeHtml(entry.detail)}</div>` : '')
    block.appendChild(el)
  }
  container.appendChild(block)
}

type NdmLogEntry = {
  icon: string
  label: string
  status: 'ok' | 'err' | 'run'
  extra?: string
  detail?: string
}

function renderProgressBar(container: HTMLElement, entries: NdmLogEntry[], progress: {
  uploadedBytes: number
  totalBytes: number
  uploadedObjects: number
  totalObjects: number
  speedBps?: number
  estimatedRemainingMs?: number
}) {
  const pct = progress.totalBytes > 0
    ? Math.round((progress.uploadedBytes / progress.totalBytes) * 100)
    : 0

  const speedStr = progress.speedBps != null
    ? formatBytes(progress.speedBps) + '/s'
    : ''
  const etaStr = progress.estimatedRemainingMs != null && progress.estimatedRemainingMs > 0
    ? formatDuration(progress.estimatedRemainingMs)
    : ''

  const progressEntry: NdmLogEntry = {
    icon: '⏳',
    label: `上传进度: ${pct}%  (${formatBytes(progress.uploadedBytes)} / ${formatBytes(progress.totalBytes)})  对象: ${progress.uploadedObjects}/${progress.totalObjects}`,
    status: 'run',
    extra: [speedStr, etaStr ? `剩余 ${etaStr}` : ''].filter(Boolean).join(' | '),
  }

  // Replace or append progress entry
  const idx = entries.findIndex(e => e.label.startsWith('上传进度:'))
  if (idx >= 0) {
    entries[idx] = progressEntry
  } else {
    entries.push(progressEntry)
  }

  renderNdmLog(container, entries)

  // Also render actual progress bar element
  const block = container.querySelector('.run-block')
  if (block) {
    let bar = block.querySelector('.ndm-progress-bar') as HTMLDivElement | null
    if (!bar) {
      bar = document.createElement('div')
      bar.className = 'ndm-progress-bar'
      bar.innerHTML = '<div class="ndm-progress-fill"></div>'
      block.appendChild(bar)
    }
    const fill = bar.querySelector('.ndm-progress-fill') as HTMLDivElement
    fill.style.width = `${pct}%`
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`
}

function formatDuration(ms: number): string {
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  return `${m}m${s % 60}s`
}

async function handleNdmUpload(mode?: 'single_file' | 'multi_file' | 'single_dir') {
  if (ndmState.busy) return
  ndmState.busy = true
  ndmState.abort = new AbortController()
  if (mode) {
    if (ndmState.snapshot) ndm.releaseImportSession(ndmState.snapshot.sessionId)
    ndmState.snapshot = null
    ndmState.committed.clear()
    ndmState.entries = [{ icon: '⏳', label: '正在选择并导入...', status: 'run' }]
  }
  const entries = ndmState.entries
  const container = $('ndm-results')
  updateNdmButtons(false)
  let ready = false
  try {
    if (mode) {
      ndmState.snapshot = await ndm.pickupAndImport({ mode, signal: ndmState.abort.signal,
        onProgress: progress => {
          entries[0] = { icon: '⏳', label: `${progress.phase}: ${progress.fileName} (${progress.fileIndex + 1}/${progress.fileCount})`, status: 'run' }
          renderNdmLog(container, entries)
        },
      })
    }
    const snapshot = ndmState.snapshot
    if (!snapshot) return
    entries.push({ icon: '✓', label: '对象标识已生成；尚未视为就绪', status: 'ok', detail: JSON.stringify({
      roots: snapshot.items.map(item => ({ kind: item.kind, name: item.name, objectId: item.objectId })),
      summary: snapshot.summary, materializationStatus: snapshot.materializationStatus,
    }, null, 2) })
    const status = await ndm.ensureUploaded(snapshot.sessionId, {
      signal: ndmState.abort.signal,
      onProgress: progress => renderProgressBar(container, entries, progress),
    })
    const progress = await ndm.getUploadProgress(snapshot.sessionId)
    ready = status.objectStatus === 'ready'
    entries.push({ icon: '✓', label: '对象及全部强依赖已就绪；可直接消费 ObjId', status: 'ok', detail: JSON.stringify({
      objectStatus: status.objectStatus, uploadStatus: status.uploadStatus,
      logicalBytes: progress.uploadedBytes, transmittedBytes: progress.transmittedBytes,
      roots: snapshot.items.map(item => item.objectId),
    }, null, 2) })
    for (const warning of status.warnings) entries.push({ icon: '!', label: '能力 / 查重优化提示', status: 'run', detail: warning })
  } catch (error) {
    entries.push({ icon: '✕', label: ndmState.abort.signal.aborted ? '已取消；保留任务可重试' : '导入未就绪', status: 'err', detail: error instanceof Error ? error.message : String(error) })
  } finally {
    ndmState.busy = false
    ndmState.abort = null
    renderNdmLog(container, entries)
    updateNdmButtons(ready)
  }
}

async function extractNdmObjects() {
  const snapshot = ndmState.snapshot
  if (!snapshot || ndmState.busy) return
  const target = ($('ndm-target-path') as HTMLInputElement).value.trim()
  if (!target) {
    ndmState.entries.push({ icon: '✕', label: '请填写目标父目录后再提取', status: 'err' })
    renderNdmLog($('ndm-results'), ndmState.entries)
    return
  }
  ndmState.busy = true
  updateNdmButtons(true)
  ;($('ndm-cancel-btn') as HTMLButtonElement).disabled = true // NFSP commit itself is not cancellable.
  const client = new NfspClient({ baseUrl: window.location.origin, sessionToken: async () => (await buckyos.getAccountInfo())?.session_token ?? null })
  try {
    const status = await ndm.getImportSessionStatus(snapshot.sessionId)
    if (status.objectStatus !== 'ready') throw new Error('对象尚未就绪')
    await client.hello()
    const parent = await client.resolve(target)
    for (const item of snapshot.items) {
      const key = `${target}/${item.name}:${item.objectId}`
      if (ndmState.committed.has(key)) continue
      const committed = await client.commitObject(parent.ref, item.name, item.objectId, { overwrite: false })
      ndmState.committed.add(key)
      ndmState.entries.push({ icon: '✓', label: `本地提取并提交: ${target}/${item.name}`, status: 'ok', detail: JSON.stringify(committed, null, 2) })
    }
  } catch (error) {
    ndmState.entries.push({ icon: '✕', label: '目标提交失败；已就绪对象与已完成路径仍保留', status: 'err', detail: error instanceof Error ? error.message : String(error) })
  } finally {
    await client.bye().catch(() => undefined)
    ndmState.busy = false
    updateNdmButtons(true)
    renderNdmLog($('ndm-results'), ndmState.entries)
  }
}

// ── Main ───────────────────────────────────────────────────────────

async function main() {
  renderGroups()
  renderNdmDemoPanel()
  setAuthStatus({ kind: 'init' })

  try {
    const response = await fetch('/sdk/appservice/runtime')
    const runtime = await response.json()
    if (!response.ok || !runtime.ok) {
      throw new Error(runtime.error ?? `AppService runtime returned ${response.status}`)
    }
    if (
      runtime.appId !== APP_ID
      || typeof runtime.ownerUserId !== 'string'
      || !runtime.ownerUserId
      || runtime.appInstanceId !== `${APP_ID}@${runtime.ownerUserId}`
      || typeof runtime.zoneHost !== 'string'
      || !runtime.zoneHost
    ) {
      throw new Error('AppService runtime is missing its app instance identity or Zone hostname')
    }
    await buckyos.initBuckyOS(APP_ID, {
      appId: APP_ID,
      appInstanceId: runtime.appInstanceId,
      ownerUserId: runtime.ownerUserId,
      zoneHost: runtime.zoneHost,
      runtimeType: RuntimeType.Browser,
      defaultProtocol: `${window.location.protocol}//`,
    })
  } catch (error) {
    setAuthStatus({
      kind: 'error',
      message: `initBuckyOS 失败: ${error instanceof Error ? error.message : String(error)}`,
    })
    return
  }

  renderSupportGroupPanel()
  await refreshAuthFromSdk()

  ;($('btn-login') as HTMLButtonElement).addEventListener('click', async () => {
    try {
      await buckyos.login()
      // The line above may navigate away (window.location.assign in
      // AuthClient.login). If the SDK returns synchronously, refresh state.
      await refreshAuthFromSdk()
    } catch (error) {
      setAuthStatus({
        kind: 'error',
        message: error instanceof Error ? error.message : String(error),
      })
    }
  })

  ;($('btn-logout') as HTMLButtonElement).addEventListener('click', () => {
    try {
      buckyos.logout()
    } finally {
      setAuthStatus({ kind: 'logged-out' })
    }
  })
}

window.addEventListener('DOMContentLoaded', () => {
  void main()
})
