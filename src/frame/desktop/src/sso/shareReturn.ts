/** The invitation ID is a navigation hint. Control Panel rechecks identity, target and grant on return. */
const prefix = 'buckyos.sso.resource:'
export function rememberSharePage(target: string, inviteId: string) {
  try {
    window.localStorage.setItem(prefix + new URL(target).href, inviteId)
  } catch { /* Storage may be disabled. */ }
}
export function rememberedSharePage(target: string, fragment: string): string | null {
  try {
    const url = new URL(target)
    if (!url.hash && fragment) url.hash = fragment
    const id = window.localStorage.getItem(prefix + url.href)
    return id && /^inv-[A-Za-z0-9_-]{32}$/.test(id) ? '/sso/share/' + id : null
  } catch {
    return null
  }
}

/** Keep invitation query and fragment across sign-in retries. */
export function invitationReturnUrl(href: string): string {
  const url = new URL(href)
  if (url.searchParams.has('did_error')) {
    const keep = url.search.slice(1).split('&').filter(pair => !new URLSearchParams(pair).has('did_error'))
    url.search = keep.join('&')
  }
  return url.href
}
