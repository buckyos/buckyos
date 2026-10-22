// jump: pick the target Zone of a registered action and navigate there.
// No credentials ever pass through this page; parameters stay in the fragment.

export const ACTION_PATHS = Object.freeze({
  app_installer: '/sysdlg/app_installer',
})

export const STORAGE_KEY = 'buckyos.jump'

export function normalizeZoneHostname(input) {
  const value = String(input ?? '').trim().toLowerCase().replace(/\.$/, '')
  if (!value || value.length > 253) return null
  if (/[^a-z0-9.-]/.test(value)) return null
  const labels = value.split('.')
  if (labels.length < 2) return null
  for (const label of labels) {
    if (!label || label.length > 63 || label.startsWith('-') || label.endsWith('-')) return null
  }
  return value
}

export function actionForPath(pathname) {
  const path = String(pathname ?? '').replace(/\/+$/, '').replace(/\/index\.html$/, '')
  for (const [action, actionPath] of Object.entries(ACTION_PATHS)) {
    if (path === actionPath) return action
  }
  return null
}

// The action's parameters reached jump in the fragment (never in its HTTP request); the
// target Zone page reads them from its query. The `zone` candidate is jump's own and is dropped.
export function targetUrl(hostname, action, fragment) {
  const host = normalizeZoneHostname(hostname)
  const path = ACTION_PATHS[action]
  if (!host || !path) return null
  const params = new URLSearchParams(String(fragment ?? '').replace(/^#/, ''))
  params.delete('zone')
  const query = params.toString()
  return `https://${host}${path}${query ? `?${query}` : ''}`
}

// The stored preference is the only thing that may auto-jump; a hostname a
// third party passes in (`zone` fragment parameter) is a candidate only.
export function readPreference(storage) {
  try {
    const raw = storage?.getItem(STORAGE_KEY)
    if (!raw) return null
    const value = JSON.parse(raw)
    const host = normalizeZoneHostname(value?.current_zone_hostname)
    if (!host) return null
    return { current_zone_hostname: host, auto_jump: value.auto_jump === true }
  } catch {
    return null
  }
}

export function writePreference(storage, hostname, autoJump) {
  const host = normalizeZoneHostname(hostname)
  if (!host) return false
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify({ current_zone_hostname: host, auto_jump: autoJump === true }))
    return true
  } catch {
    return false
  }
}

export function clearPreference(storage) {
  try {
    storage?.removeItem(STORAGE_KEY)
  } catch {
    // Storage disabled: nothing was kept.
  }
}

export function candidateFromFragment(fragment) {
  const params = new URLSearchParams(String(fragment ?? '').replace(/^#/, ''))
  return normalizeZoneHostname(params.get('zone'))
}

/** What the page does on load: `{kind:'jump', url}` or `{kind:'choose', prefill, candidate}` or `{kind:'unknown_action'}`. */
export function plan(pathname, fragment, storage) {
  const action = actionForPath(pathname)
  if (!action) return { kind: 'unknown_action' }
  const preference = readPreference(storage)
  const candidate = candidateFromFragment(fragment)
  if (preference?.auto_jump) {
    return { kind: 'jump', url: targetUrl(preference.current_zone_hostname, action, fragment) }
  }
  return {
    kind: 'choose',
    action,
    prefill: preference?.current_zone_hostname ?? '',
    candidate: candidate && candidate !== preference?.current_zone_hostname ? candidate : null,
  }
}
