export function redirectToDesktopLogin() {
  const loginUrl = new URL('/login', window.location.origin)
  loginUrl.searchParams.set('redirect_url', window.location.href)
  window.location.replace(loginUrl.toString())
}
