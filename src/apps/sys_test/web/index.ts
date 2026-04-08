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
 *   - 在页面中运行检测 → run the cases through the in-page browser SDK
 *   - 在后台服务中运行检测 → POST /sdk/appservice/selftest so the AppService
 *     runtime in main.ts runs the same cases on the server side
 *
 * Each group also leaves a placeholder area for future per-service manual
 * call panels (point 4 of the task brief).
 */
import { buckyos } from 'buckyos'
import { TEST_GROUPS, TestCase, TestContext, TestGroup } from './src/test_groups'

const APP_ID = 'buckyos_systest'
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
    // Try to refresh once in case the cookie just landed.
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
  if (!response.ok && (!payload || !Array.isArray(payload.results))) {
    throw new Error(
      payload?.error ?? `selftest endpoint returned status ${response.status}`,
    )
  }
  return payload.results ?? []
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
    actions.appendChild(runInPage)

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

async function main() {
  renderGroups()
  setAuthStatus({ kind: 'init' })

  try {
    await buckyos.initBuckyOS(APP_ID)
  } catch (error) {
    setAuthStatus({
      kind: 'error',
      message: `initBuckyOS 失败: ${error instanceof Error ? error.message : String(error)}`,
    })
    return
  }

  await refreshAuthFromSdk()

  ;($('btn-login') as HTMLButtonElement).addEventListener('click', async () => {
    try {
      // Pass autoLogin=false: when the user explicitly clicks Login we want
      // the SDK to skip its localStorage account_info cache and force a real
      // SSO redirect. With the default (autoLogin=true), a stale cached
      // entry under `buckyos.account_info.${appId}` makes loginByBrowserSSO
      // return early without ever calling AuthClient.login(), so the page
      // never redirects. See sdk_core.ts:loginByBrowserSSO.
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
