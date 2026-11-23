/**
 * sys_test official example (notepads/done/self-host-group-extension-todo.md §11, EXT-10):
 * a customer-service group for alice, bob and an external customer.
 *
 * Three buttons. "Create group" uses the signed-in user's session on this App origin
 * (msg-center `group.create` with the group's custom page declared as an L2 panel served
 * by this App). "Customer link" asks the sys_test backend, which issues a 48-hour
 * `/sso/invite` link with the App identity (`user.app_login.issue {kind:"link"}`) and
 * pre-invites the customer's account into the Session. "Clean up" deletes the group.
 */
import { localize, locale, t } from './i18n'
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
    display: { announcement: t('support.announce', { name, session: SESSION_ID }) },
    extensions: { panels: [{ id: 'tickets', title: t('ticket.title'), entry_url: panelUrl, scope: 'group' }] },
    session_templates: {
      support: {
        membership: { roles: ['owner', 'admin'] },
        rules: { allow_guests: true, post: 'all_participants', panels: ['tickets'], display: { announcement: t('support.shared') } },
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
    <div class="group-header"><h2 class="group-title" data-i18n="support.title">Support group and customer links</h2><span class="group-id">support_group</span></div>
    <p class="group-desc" data-i18n="support.intro">Create a support group with two Zone users and this app’s custom panel. Generate a 48-hour link to let a customer join with a restricted account. The link works repeatedly until expiry. Once a password is set, it is required. Cleanup deletes the group. Only the installing account can generate links.</p>
    <p class="group-desc" data-i18n="support.enable">Before generating a link, the installing account must enable sign-in for external customers in the app details in App Service. This is off by default and creating a group does not enable it.</p>
    <div class="group-actions">
      <input id="sg-name" type="text" data-i18n-label="support.groupName" aria-label="Group name" data-i18n-default="support.defaultName" value="Support group (sys-test)" />
      <input id="sg-member-a" type="text" data-i18n-label="support.member1" aria-label="Member 1" value="alice" />
      <input id="sg-member-b" type="text" data-i18n-label="support.member2" aria-label="Member 2" value="bob" />
      <button id="sg-create" class="btn primary" data-testid="support-create" data-i18n="support.create">Create support group</button>
    </div>
    <div class="group-actions">
      <input id="sg-customer-name" type="text" data-i18n-label="support.customer" aria-label="Customer name" data-i18n-placeholder="support.nameExample" placeholder="Customer name, e.g. Alex" />
      <input id="sg-customer-id" type="text" data-i18n-label="support.externalId" aria-label="External customer ID" data-i18n-placeholder="support.idExample" placeholder="External ID, e.g. crm:1001" />
      <button id="sg-link" class="btn secondary" data-testid="support-link" disabled data-i18n="support.link">Generate customer link</button>
      <button id="sg-cleanup" class="btn" data-testid="support-cleanup" disabled data-i18n="support.cleanup">Clean up</button>
    </div>
    <div id="sg-results" class="results" data-testid="support-results"></div>
  `
  localize(card)
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
      head.innerHTML = `<span class="icon">●</span><span class="name">${escapeHtml(t('support.current', { name: state.name }))}</span>`
        + `<div class="details"><span data-i18n="support.did">Group DID</span>: <code>${escapeHtml(state.group_did)}</code> · <span data-i18n="support.session">Session</span>: <code>${escapeHtml(state.session_id)}</code> · <span data-i18n="support.members">Members</span>: ${escapeHtml(new Intl.ListFormat(locale).format(state.members))}<br>`
        + `<a href="https://${escapeHtml(buckyos.getZoneHostName() ?? '')}/messagehub" target="_blank" rel="noopener" data-i18n="support.panelHint">Open group details in Message Hub</a></div>`
      localize(head)
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
      const name = $<HTMLInputElement>('sg-name').value.trim() || t('support.defaultName')
      const members = [$<HTMLInputElement>('sg-member-a').value.trim(), $<HTMLInputElement>('sg-member-b').value.trim()].filter(Boolean)
      const me = await buckyos.getAccountInfo()
      if (!me?.user_id) throw new Error(t('auth.needed'))
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
        sessions: [{ session_id: SESSION_ID, template: 'support', title: t('ticket.title') }],
        invitations,
      })
      if (!isRecord(created) || typeof created.group_did !== 'string') throw new Error('invalid group.create response')
      state = { group_did: created.group_did, name, session_id: SESSION_ID, members: [me.user_id, ...members.filter(m => m !== me.user_id)] }
      saveState(state)
      log({ icon: '✓', label: t('support.created', { name, url: panelUrl }), status: 'ok', detail: t('support.invited', { names: new Intl.ListFormat(locale).format(members) }) })
    } catch (error) {
      fail(t('support.createFailed'), error)
    } finally {
      createBtn.disabled = false
    }
  })

  linkBtn.addEventListener('click', async () => {
    if (!state) return
    linkBtn.disabled = true
    try {
      const me = await buckyos.getAccountInfo()
      if (!me?.session_token) throw new Error(t('auth.needed'))
      const externalId = $<HTMLInputElement>('sg-customer-id').value.trim() || `systest:${Date.now().toString(36)}`
      const showName = $<HTMLInputElement>('sg-customer-name').value.trim()
      const response = await fetch('/sdk/appservice/support/customer_link', {
        method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${me.session_token}` },
        body: JSON.stringify({ group_did: state.group_did, session_id: state.session_id, external_account_id: externalId, show_name: showName, title: `${state.name} · ${t('ticket.title')}` }),
      })
      const data = await response.json() as JsonRecord
      if (!response.ok || data.ok !== true || typeof data.link !== 'string') {
        const denial = isRecord(data.data) ? data.data : {}
        if (data.code === 'access_denied' && denial.reason === 'zone_method_disabled' && denial.detail === 'app_login_disabled') {
          throw new Error(t('support.enable'))
        }
        throw new Error(String(data.error ?? `HTTP ${response.status}`))
      }
      const expires = new Date(Number(data.expires_at) * 1000).toLocaleString(locale)
      log({
        icon: '✓',
        label: t('support.linked', { id: String(data.user_id), kind: t(data.new_account ? 'support.createdAccount' : 'support.existingAccount') }),
        status: 'ok',
        html: `<code class="support-link" data-testid="support-customer-link" style="user-select:all;word-break:break-all">${escapeHtml(String(data.link))}</code>`
          + `<br/>${escapeHtml(t('support.expires', { expires }))}`,
      })
    } catch (error) {
      fail(t('support.linkFailed'), error)
    } finally {
      linkBtn.disabled = !state
    }
  })

  cleanupBtn.addEventListener('click', async () => {
    if (!state) return
    cleanupBtn.disabled = true
    try {
      await buckyos.getServiceRpcClient('msg-center').call('group.delete', { group_did: state.group_did })
      log({ icon: '✓', label: t('support.deleted', { name: state.name }), status: 'ok', detail: t('support.keep') })
      state = null
      saveState(null)
    } catch (error) {
      fail(t('support.cleanupFailed'), error)
      cleanupBtn.disabled = false
    }
  })

  render()
}
