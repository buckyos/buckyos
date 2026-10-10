/* Cross-app notifications: an Agent or the signed-in user's own profile changed. */

const AGENTS_CHANGED = 'buckyos:agents-changed'
const OWN_PROFILE_CHANGED = 'buckyos:own-profile-changed'

export function notifyAgentsChanged() {
  window.dispatchEvent(new Event(AGENTS_CHANGED))
}

export function onAgentsChanged(listener: () => void) {
  window.addEventListener(AGENTS_CHANGED, listener)
  return () => window.removeEventListener(AGENTS_CHANGED, listener)
}

export function notifyOwnProfileChanged() {
  window.dispatchEvent(new Event(OWN_PROFILE_CHANGED))
}

export function onOwnProfileChanged(listener: () => void) {
  window.addEventListener(OWN_PROFILE_CHANGED, listener)
  return () => window.removeEventListener(OWN_PROFILE_CHANGED, listener)
}
