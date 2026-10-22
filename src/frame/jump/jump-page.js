import {
  clearPreference,
  normalizeZoneHostname,
  plan,
  readPreference,
  targetUrl,
  writePreference,
} from './jump-core.js'

function storage() {
  try {
    return window.localStorage
  } catch {
    return null
  }
}

function render() {
  const fragment = window.location.hash
  const decision = plan(window.location.pathname, fragment, storage())
  const root = document.getElementById('app')
  if (decision.kind === 'unknown_action') {
    root.querySelector('[data-role=unknown]').hidden = false
    return
  }
  if (decision.kind === 'jump' && decision.url) {
    window.location.replace(decision.url)
    return
  }
  const form = root.querySelector('form')
  form.hidden = false
  const input = form.querySelector('input[name=zone]')
  input.value = decision.prefill
  const candidate = root.querySelector('[data-role=candidate]')
  if (decision.candidate) {
    candidate.hidden = false
    candidate.querySelector('button').textContent = decision.candidate
    candidate.querySelector('button').addEventListener('click', () => {
      input.value = decision.candidate
    })
  }
  const stored = readPreference(storage())
  const clear = root.querySelector('[data-role=clear]')
  if (stored) {
    clear.hidden = false
    clear.addEventListener('click', () => {
      clearPreference(storage())
      input.value = ''
      clear.hidden = true
    })
  }
  const error = root.querySelector('[data-role=error]')
  form.addEventListener('submit', (event) => {
    event.preventDefault()
    const host = normalizeZoneHostname(input.value)
    const url = host && targetUrl(host, decision.action, fragment)
    if (!url) {
      error.hidden = false
      return
    }
    const remember = form.querySelector('input[name=remember]').checked
    const auto = form.querySelector('input[name=auto]').checked
    if (remember || auto) writePreference(storage(), host, auto)
    window.location.assign(url)
  })
}

render()
