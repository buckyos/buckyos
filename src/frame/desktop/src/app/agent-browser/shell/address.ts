/** What the address bar's text becomes: an address as it is (a scheme is added when it has none), anything else a web search. */
export function toNavigableUrl(input: string): string | null {
  const text = input.trim()
  if (!text) return null
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(text) || /^(about|data|blob):/i.test(text)) return text
  const host = text.split(/[/?#]/, 1)[0]
  if (/\s/.test(text)) return searchUrl(text)
  if (/^localhost(:\d+)?$/i.test(host) || /^\d{1,3}(\.\d{1,3}){3}(:\d+)?$/.test(host)) return `http://${text}`
  return /^[^\s.]+(\.[^\s.]+)+(:\d+)?$/.test(host) ? `https://${text}` : searchUrl(text)
}

function searchUrl(text: string): string {
  return `https://www.bing.com/search?q=${encodeURIComponent(text)}`
}

/** The address as the bar shows it while it is not being edited: without the scheme of an ordinary page. */
export function displayUrl(url: string): string {
  return url.replace(/^https:\/\//i, '').replace(/\/$/, '')
}
