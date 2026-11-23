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
import { AppFrameClient, embedderOrigin, originInZone } from 'buckyos/app-frame'
import { bindText, hasTranslation, localize, locale, setLocale, t } from './src/i18n'
import { buckyos, ndm, RuntimeType } from 'buckyos'
import { NfspClient } from 'buckyos/nfsp'
import { TEST_GROUPS, TestCase, TestContext, TestGroup } from './src/test_groups'
import { renderCustomerTicket } from './src/customer_ticket'
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
      bindText(status, 'auth.init')
      loginBtn.disabled = true
      logoutBtn.disabled = true
      break
    case 'logged-out':
      bindText(status, 'auth.loggedOut')
      loginBtn.disabled = false
      logoutBtn.disabled = true
      break
    case 'logged-in':
      bindText(status, 'auth.loggedIn', { name: state.userName, id: state.userId })
      status.classList.add('ok')
      loginBtn.disabled = true
      logoutBtn.disabled = false
      break
    case 'error':
      bindText(status, 'auth.error', { error: state.message })
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
    throw new Error(t('auth.needed'))
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
  bindText(badge, origin === 'in-page' ? 'group.browser' : 'group.server')
  header.appendChild(badge)

  const ts = document.createElement('span')
  ts.textContent = new Date().toLocaleTimeString(locale)
  header.appendChild(ts)

  block.appendChild(header)

  if (error) {
    const errEl = document.createElement('div')
    errEl.className = 'case err'
    errEl.innerHTML =
      '<span class="icon">✕</span>' +
      '<span class="name" data-i18n="group.fail">运行失败</span>' +
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
        : `<div class="details error">${escapeHtml(result.error ?? t('group.unknown'))}</div>`
      caseEl.innerHTML =
        `<span class="icon">${icon}</span>` +
        `<span class="name">${escapeHtml(result.name)}</span>` +
        `<span class="duration">${result.durationMs}ms</span>` +
        detailsHtml
      const labelKey = `cases.${result.name}`
      if (hasTranslation(labelKey)) bindText(caseEl.querySelector('.name')!, labelKey)
      block.appendChild(caseEl)
    }
  }

  localize(block)
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
    `<div class="run-header"><span class="badge ${origin}" data-i18n="${origin === 'in-page' ? 'group.browser' : 'group.server'}"></span><span data-i18n="group.running">运行中...</span></div>` +
    `<div class="case run"><span class="icon">⏳</span><span class="name" data-i18n="group.running">running</span><span class="duration"></span></div>`
  localize(block)
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
    if (group.titleKey) bindText(header.querySelector('h2')!, group.titleKey)
    card.appendChild(header)

    const desc = document.createElement('p')
    desc.className = 'group-desc'
    bindText(desc, `tests.${group.id}.description`)
    card.appendChild(desc)

    const actions = document.createElement('div')
    actions.className = 'group-actions'

    const runInPage = document.createElement('button')
    runInPage.className = 'btn primary'
    bindText(runInPage, 'group.runBrowser')
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
    bindText(runOnServer, 'group.runServer')
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
    bindText(placeholder, 'group.manual')
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
    <div class="group-header"><h2 class="group-title" data-i18n="ndm.title">NDM 文件 / 目录导入</h2></div>
    <p class="group-desc" data-i18n="ndm.about">SDK：QCID 查询 → 对象标识 → 缺失内容上传 → 元数据登记 → 强依赖就绪。默认只取得 ObjId，不落盘。相同内容可再次选择以验证去重。</p>
    <div class="group-actions">
      <button id="ndm-single-btn" class="btn primary" data-i18n="ndm.one">选择文件</button>
      <button id="ndm-multi-btn" class="btn" data-i18n="ndm.many">选择多个文件</button>
      <button id="ndm-folder-btn" class="btn" data-i18n="ndm.folder">选择文件夹</button>
      <button id="ndm-cancel-btn" class="btn" disabled data-i18n="cancel">取消</button>
      <button id="ndm-retry-btn" class="btn" disabled data-i18n="ndm.retry">重试此任务</button>
      <button id="ndm-release-btn" class="btn" disabled data-i18n="ndm.release">释放任务</button>
    </div>
    <p id="ndm-capability" class="group-desc"></p>
    <div class="group-actions">
      <input id="ndm-target-path" type="text" data-i18n-label="ndm.target" aria-label="目标父目录" data-i18n-placeholder="ndm.example" placeholder="目标父目录，例如 /home/alice" />
      <button id="ndm-extract-btn" class="btn" disabled data-i18n="ndm.extract">按 ObjId 本地提取到目标目录</button>
    </div>
    <p class="group-desc" data-i18n="ndm.help">本地提取通过 NFSP 提交，服务端重新校验权限及目标冲突，不覆盖已有路径。多根目录/文件分别提交；部分失败会保留已完成项，重试仅处理未确认项。响应丢失后的冲突需检查目标，不自动覆盖。</p>
    <div id="ndm-results" class="results"></div>
  `
  localize(card)
  $('groups').appendChild(card)
  const folderSupported = ndm.getImportProvider().getCapabilities().canPickDirectory
  bindText($('ndm-capability'), folderSupported ? 'ndm.canFolder' : 'ndm.noFolder')
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
    ndmState.entries.push({ icon: '✓', label: t('ndm.released'), status: 'ok' })
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
  bindText(badge, 'ndm.upload')
  header.appendChild(badge)
  const ts = document.createElement('span')
  ts.textContent = new Date().toLocaleTimeString(locale)
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
  localize(block)
  container.appendChild(block)
}

type NdmLogEntry = {
  progress?: boolean
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
    progress: true,
    icon: '⏳',
    label: t('ndm.progress', { pct, uploaded: formatBytes(progress.uploadedBytes), total: formatBytes(progress.totalBytes), objects: progress.uploadedObjects, count: progress.totalObjects }),
    status: 'run',
    extra: [speedStr, etaStr ? t('ndm.remaining', { time: etaStr }) : ''].filter(Boolean).join(' | '),
  }

  // Replace or append progress entry
  const idx = entries.findIndex(e => e.progress)
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
    ndmState.entries = [{ icon: '⏳', label: t('ndm.selecting'), status: 'run' }]
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
    entries.push({ icon: '✓', label: t('ndm.identify'), status: 'ok', detail: JSON.stringify({
      roots: snapshot.items.map(item => ({ kind: item.kind, name: item.name, objectId: item.objectId })),
      summary: snapshot.summary, materializationStatus: snapshot.materializationStatus,
    }, null, 2) })
    const status = await ndm.ensureUploaded(snapshot.sessionId, {
      signal: ndmState.abort.signal,
      onProgress: progress => renderProgressBar(container, entries, progress),
    })
    const progress = await ndm.getUploadProgress(snapshot.sessionId)
    ready = status.objectStatus === 'ready'
    entries.push({ icon: '✓', label: t('ndm.ready'), status: 'ok', detail: JSON.stringify({
      objectStatus: status.objectStatus, uploadStatus: status.uploadStatus,
      logicalBytes: progress.uploadedBytes, transmittedBytes: progress.transmittedBytes,
      roots: snapshot.items.map(item => item.objectId),
    }, null, 2) })
    for (const warning of status.warnings) entries.push({ icon: '!', label: t('ndm.warn'), status: 'run', detail: warning })
  } catch (error) {
    entries.push({ icon: '✕', label: ndmState.abort.signal.aborted ? t('ndm.cancelled') : t('ndm.notReady'), status: 'err', detail: error instanceof Error ? error.message : String(error) })
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
    ndmState.entries.push({ icon: '✕', label: t('ndm.needTarget'), status: 'err' })
    renderNdmLog($('ndm-results'), ndmState.entries)
    return
  }
  ndmState.busy = true
  updateNdmButtons(true)
  ;($('ndm-cancel-btn') as HTMLButtonElement).disabled = true // NFSP commit itself is not cancellable.
  const client = new NfspClient({ baseUrl: window.location.origin, sessionToken: async () => (await buckyos.getAccountInfo())?.session_token ?? null })
  try {
    const status = await ndm.getImportSessionStatus(snapshot.sessionId)
    if (status.objectStatus !== 'ready') throw new Error(t('ndm.objectNotReady'))
    await client.hello()
    const parent = await client.resolve(target)
    for (const item of snapshot.items) {
      const key = `${target}/${item.name}:${item.objectId}`
      if (ndmState.committed.has(key)) continue
      const committed = await client.commitObject(parent.ref, item.name, item.objectId, { overwrite: false })
      ndmState.committed.add(key)
      ndmState.entries.push({ icon: '✓', label: t('ndm.committed', { path: `${target}/${item.name}` }), status: 'ok', detail: JSON.stringify(committed, null, 2) })
    }
  } catch (error) {
    ndmState.entries.push({ icon: '✕', label: t('ndm.failed'), status: 'err', detail: error instanceof Error ? error.message : String(error) })
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
    const shellOrigin = embedderOrigin(origin => originInZone(origin, runtime.zoneHost))
    if (shellOrigin) new AppFrameClient({ shellOrigin, appId: APP_ID, onInit: init => setLocale(init.locale), onLocale: setLocale })
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
      message: t('auth.initFailed', { error: error instanceof Error ? error.message : String(error) }),
    })
    return
  }

  if (location.pathname === '/support/ticket') {
    await renderCustomerTicket()
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
