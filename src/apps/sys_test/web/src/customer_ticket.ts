import { bindText, localize, t } from './i18n'
import { buckyos, type CurrentIdentity } from 'buckyos'

/** All operations use the customer's App session. MessageCenter verifies the ticket and Session ACL. */
export async function renderCustomerTicket() {
  bindText(document.querySelector('title')!, 'ticket.title')
  const query = new URLSearchParams(location.search)
  const group = query.get('group')
  const session = query.get('session')
  document.body.innerHTML = `<main style="max-width:42rem;margin:3rem auto;padding:1rem">
    <h1 data-i18n="ticket.title">Support ticket</h1><p id="ticket-status" role="status"></p>
    <button id="ticket-join" data-i18n="ticket.join">Join ticket</button><ol id="ticket-messages"></ol>
    <form id="ticket-message"><label><span data-i18n="ticket.message">Message</span> <textarea required></textarea></label><button data-i18n="ticket.send">Send</button></form>
    <form id="ticket-password"><label><span data-i18n="ticket.password">New password</span> <input type="password" minlength="8" required autocomplete="new-password"></label>
      <label><span data-i18n="ticket.confirm">Confirm password</span> <input type="password" minlength="8" required autocomplete="new-password"></label><button data-i18n="ticket.set">Set password</button></form>
  </main>`
  localize()
  const status = document.querySelector<HTMLElement>('#ticket-status')!
  const join = document.querySelector<HTMLButtonElement>('#ticket-join')!
  const send = document.querySelector<HTMLFormElement>('#ticket-message')!
  const setPassword = document.querySelector<HTMLFormElement>('#ticket-password')!
  send.hidden = setPassword.hidden = join.hidden = true
  try {
    if (!group || !session) throw new Error(t('ticket.badLink'))
    const cp = buckyos.getServiceRpcClient('control-panel')
    const msg = buckyos.getServiceRpcClient('msg-center')
    const identity = await cp.call('auth.current_identity', {}) as CurrentIdentity
    if (identity.kind !== 'account' || !identity.did) throw new Error(t('ticket.reopen'))
    const target = {group_did:group,session_id:session}
    const list = async () => {
      const page = await msg.call('group.list_messages',{...target, with_object:true,limit:100}) as {items: {msg?:{content?:{content?:string}}}[]}
      const items: HTMLElement[] = []
      for (const row of page.items ?? []) {
        if (typeof row.msg?.content?.content !== 'string') continue
        const li = document.createElement('li')
        li.textContent = row.msg.content.content
        items.push(li)
      }
      document.querySelector('#ticket-messages')!.replaceChildren(...items)
      join.hidden = true
      send.hidden = false
      delete status.dataset.i18n
      status.textContent = identity.show_name || identity.user_id
    }
    const failure = (err:unknown) => {delete status.dataset.i18n; status.textContent = err instanceof Error ? err.message : String(err)}
    join.onclick = async () => {
      join.disabled = true
      try { await msg.call('group.accept_session_invitation',target); await list() }
      catch (err) { failure(err) } finally { join.disabled = false }
    }
    send.onsubmit = async (event) => {
      event.preventDefault()
      const field = send.querySelector('textarea')!
      const message = field.value.trim()
      if (!message || field.disabled) return
      field.disabled = true
      try {
        await msg.call('msg.post_send',{idempotency_key:crypto.randomUUID(),msg:{
          from:identity.did,to:[group],kind:'group_msg',to_session:session,created_at_ms:Date.now(),
          nonce:crypto.getRandomValues(new Uint32Array(1))[0],content:{format:'text/plain',content:message},
        }})
        field.value = ''
        await list()
      } catch (err) {failure(err)} finally { field.disabled = false }
    }
    setPassword.hidden = identity.password_set
    setPassword.onsubmit = async event => {
      event.preventDefault()
      const [password,confirm] = setPassword.querySelectorAll('input')
      if (password.value !== confirm.value) {bindText(status, 'ticket.mismatch'); return}
      const submit = setPassword.querySelector('button')!
      submit.disabled = true
      try {
        const result = await cp.call('auth.invite.set_password',{new_password_hash:buckyos.hashPassword(identity.user_id,password.value)}) as {status:string}
        if (result.status !== 'ok') throw new Error(t('ticket.failed'))
        setPassword.reset()
        setPassword.hidden = true
        bindText(status, 'ticket.saved', { name: identity.user_id })
      } catch (err) {failure(err)} finally {submit.disabled = false}
    }
    try {await list()} catch {join.hidden = false; bindText(status, 'ticket.joinHint')}
  } catch (err) {status.textContent = err instanceof Error ? err.message : String(err)}
}
