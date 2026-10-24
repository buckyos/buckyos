/**
 * sys_test official example (notepads/self-host-group-extension-todo.md §11, EXT-10):
 * a customer-service group for alice, bob and an external customer.
 *
 * Three buttons. "Create group" uses the signed-in user's session on this App origin
 * (msg-center `group.create` with the group's custom page declared as an L2 panel served
 * by this App). "Customer link" asks the sys_test backend, which issues a 48-hour
 * `/sso/invite` link with the App identity (`user.app_login.issue {kind:"link"}`) and
 * pre-invites the customer's account into the Session. "Clean up" deletes the group.
 */
import { buckyos } from 'buckyos'

type JsonRecord = Record<string, unknown>

const STORAGE_KEY = 'buckyos.systest.support_group.v1'
const SESSION_ID = 'support'

interface SupportState {
  group_did: string
  name: string
  session_id: string
  members: string[]
}

function $<T extends HTMLElement>(id: string): T {
  const el = document.getElementById(id)
  if (!el) throw new Error(`element #${id} not found`)
  return el as T
}

function escapeHtml(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')
}

function loadState(): SupportState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? (JSON.parse(raw) as SupportState) : null
  } catch {
    return null
  }
}

function saveState(state: SupportState | null) {
  try {
    if (state) localStorage.setItem(STORAGE_KEY, JSON.stringify(state))
    else localStorage.removeItem(STORAGE_KEY)
  } catch {
    /* storage unavailable: the page still works for this visit */
  }
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function zoneUserDid(userId: string): Promise<string> {
  const listed = await buckyos.getServiceRpcClient('control-panel').call('user.list', {})
  const users = (isRecord(listed) && Array.isArray(listed.users) ? listed.users : []) as JsonRecord[]
  const did = users.find(user => user.user_id === userId && typeof user.did === 'string')?.did
  if (typeof did !== 'string') throw new Error(`user.list has no DID for '${userId}'`)
  return did
}

/** The group configuration: one Session template shared by staff and customers, the panel served by this App. */
function supportConfiguration(panelUrl: string, name: string) {
  return {
    required_features: ['sessions', 'session_guests', 'panels'],
    display: { announcement: `${name}：客服与客户在 Session「${SESSION_ID}」里交流。` },
    extensions: { panels: [{ id: 'tickets', title: '客服工单', entry_url: panelUrl, scope: 'group' }] },
    session_templates: {
      support: {
        membership: { roles: ['owner', 'admin'] },
        rules: { allow_guests: true, post: 'all_participants', panels: ['tickets'], display: { announcement: '本 Session 由客服与客户共享。' } },
      },
    },
  }
}

type LogEntry = { icon: string; label: string; status: 'ok' | 'err' | 'run'; detail?: string; html?: string }

export function renderSupportGroupPanel() {
  const card = document.createElement('section')
  card.className = 'group'
  card.dataset.group = 'support_group'
  card.innerHTML = `
    <div class="group-header"><h2 class="group-title">客服群与客户链接（Self-host Group 扩展官方例子）</h2><span class="group-id">support_group</span></div>
    <p class="group-desc">① 用当前登录用户建一个客服群：邀请两位本 Zone 用户，并把本应用提供的页面声明为群的自定义面板。② 为一位外部客户生成 48 小时链接：客户点开即以自己的受限账号进入该 Session，未设密码前链接可反复使用；设了密码之后同一链接要先输密码。③ 清理会删掉这个群。生成链接只允许本应用的安装账号。</p>
    <div class="group-actions">
      <input id="sg-name" type="text" aria-label="群名" value="客服群（sys-test）" />
      <input id="sg-member-a" type="text" aria-label="成员 1" value="alice" />
      <input id="sg-member-b" type="text" aria-label="成员 2" value="bob" />
      <button id="sg-create" class="btn primary" data-testid="support-create">创建客服群</button>
    </div>
    <div class="group-actions">
      <input id="sg-customer-name" type="text" aria-label="客户称呼" placeholder="客户称呼，例如 王女士" />
      <input id="sg-customer-id" type="text" aria-label="客户外部标识" placeholder="外部标识，例如 crm:1001" />
      <button id="sg-link" class="btn secondary" data-testid="support-link" disabled>生成客户链接</button>
      <button id="sg-cleanup" class="btn" data-testid="support-cleanup" disabled>清理</button>
    </div>
    <div id="sg-results" class="results" data-testid="support-results"></div>
  `
  $('groups').appendChild(card)

  let state = loadState()
  const entries: LogEntry[] = []
  const results = $('sg-results')
  const createBtn = $<HTMLButtonElement>('sg-create')
  const linkBtn = $<HTMLButtonElement>('sg-link')
  const cleanupBtn = $<HTMLButtonElement>('sg-cleanup')

  const render = () => {
    results.innerHTML = ''
    const block = document.createElement('div')
    block.className = 'run-block'
    if (state) {
      const head = document.createElement('div')
      head.className = 'case ok'
      head.setAttribute('data-testid', 'support-group-did')
      head.innerHTML = `<span class="icon">●</span><span class="name">当前群：${escapeHtml(state.name)}</span>`
        + `<div class="details">群 DID：<code>${escapeHtml(state.group_did)}</code>；Session：<code>${escapeHtml(state.session_id)}</code>；成员：${escapeHtml(state.members.join('、'))}。`
        + `成员在 <a href="https://${escapeHtml(buckyos.getZoneHostName() ?? '')}/messagehub" target="_blank" rel="noopener">Message Hub</a> 的群详情里看到「客服工单」面板。</div>`
      block.appendChild(head)
    }
    for (const entry of entries) {
      const el = document.createElement('div')
      el.className = `case ${entry.status}`
      el.innerHTML = `<span class="icon">${entry.icon}</span><span class="name">${escapeHtml(entry.label)}</span>`
        + (entry.html ? `<div class="details">${entry.html}</div>` : entry.detail ? `<div class="details${entry.status === 'err' ? ' error' : ''}">${escapeHtml(entry.detail)}</div>` : '')
      block.appendChild(el)
    }
    results.appendChild(block)
    linkBtn.disabled = !state
    cleanupBtn.disabled = !state
  }
  const log = (entry: LogEntry) => { entries.unshift(entry); render() }
  const fail = (label: string, error: unknown) => log({ icon: '✗', label, status: 'err', detail: error instanceof Error ? error.message : String(error) })

  createBtn.addEventListener('click', async () => {
    createBtn.disabled = true
    try {
      const name = $<HTMLInputElement>('sg-name').value.trim() || '客服群（sys-test）'
      const members = [$<HTMLInputElement>('sg-member-a').value.trim(), $<HTMLInputElement>('sg-member-b').value.trim()].filter(Boolean)
      const me = await buckyos.getAccountInfo()
      if (!me?.user_id) throw new Error('not logged in')
      const invitations = []
      for (const member of members) {
        if (member === me.user_id) continue
        invitations.push({ member_did: await zoneUserDid(member) })
      }
      const panelUrl = `${window.location.origin}/support/panel`
      const created = await buckyos.getServiceRpcClient('msg-center').call('group.create', {
        idempotency_key: crypto.randomUUID(),
        profile: { name },
        configuration: supportConfiguration(panelUrl, name),
        sessions: [{ session_id: SESSION_ID, template: 'support', title: '客服工单' }],
        invitations,
      })
      if (!isRecord(created) || typeof created.group_did !== 'string') throw new Error('invalid group.create response')
      state = { group_did: created.group_did, name, session_id: SESSION_ID, members: [me.user_id, ...members.filter(m => m !== me.user_id)] }
      saveState(state)
      log({ icon: '✓', label: `已创建群 ${name}，面板 ${panelUrl}`, status: 'ok', detail: `邀请：${members.join('、')}（同 Zone 用户自动接受）` })
    } catch (error) {
      fail('创建客服群失败', error)
    } finally {
      createBtn.disabled = false
    }
  })

  linkBtn.addEventListener('click', async () => {
    if (!state) return
    linkBtn.disabled = true
    try {
      const me = await buckyos.getAccountInfo()
      if (!me?.session_token) throw new Error('not logged in')
      const externalId = $<HTMLInputElement>('sg-customer-id').value.trim() || `systest:${Date.now().toString(36)}`
      const showName = $<HTMLInputElement>('sg-customer-name').value.trim()
      const response = await fetch('/sdk/appservice/support/customer_link', {
        method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${me.session_token}` },
        body: JSON.stringify({ group_did: state.group_did, session_id: state.session_id, external_account_id: externalId, show_name: showName, title: `${state.name} · 客服工单` }),
      })
      const data = await response.json() as JsonRecord
      if (!response.ok || data.ok !== true || typeof data.link !== 'string') throw new Error(String(data.error ?? `HTTP ${response.status}`))
      const expires = new Date(Number(data.expires_at) * 1000).toLocaleString()
      log({
        icon: '✓',
        label: `客户链接已生成（账号 ${String(data.user_id)}，${data.new_account ? '新建' : '复用'}）`,
        status: 'ok',
        html: `<code class="support-link" data-testid="support-customer-link" style="user-select:all;word-break:break-all">${escapeHtml(String(data.link))}</code>`
          + `<br/>客户未设密码前此链接在 ${escapeHtml(expires)} 前可直接进入；链接只显示这一次，请经你们自己的渠道发给客户。`,
      })
    } catch (error) {
      fail('生成客户链接失败', error)
    } finally {
      linkBtn.disabled = !state
    }
  })

  cleanupBtn.addEventListener('click', async () => {
    if (!state) return
    cleanupBtn.disabled = true
    try {
      await buckyos.getServiceRpcClient('msg-center').call('group.delete', { group_did: state.group_did })
      log({ icon: '✓', label: `已删除群 ${state.name}`, status: 'ok', detail: '客户账号与它的链接记录保留；链接到期后自动失效，再次为同一外部标识生成会复用该账号。' })
      state = null
      saveState(null)
    } catch (error) {
      fail('清理失败', error)
      cleanupBtn.disabled = false
    }
  })

  render()
}
