import DOMPurify from 'dompurify'
import { marked } from 'marked'

const fallbackEscapeHtml = (value: string) =>
  value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')

export const renderMarkdownHtml = (content: string) => {
  try {
    const rawHtml = marked.parse(content, {
      async: false,
      gfm: true,
      breaks: true,
    }) as string

    return DOMPurify.sanitize(rawHtml, {
      USE_PROFILES: { html: true },
    })
  } catch {
    return `<pre>${fallbackEscapeHtml(content)}</pre>`
  }
}
