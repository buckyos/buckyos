/* OPML ⇄ outline (脑图方案 §9, TODO M4): the outliners' common format, the same shape as a Markdown outline.
 * `<outline text>` nests; `_note` and other attributes are not kept. */

import { el, elements, firstElement, parseXml, toXml, XmlError } from '../import-drawio/xml.ts'
import type { OutlineNode } from './outline.ts'

export class OpmlError extends Error {}

/** The outline of an OPML document and its head title. */
export function parseOpml(source: string): { title: string | null; nodes: OutlineNode[] } {
  let doc
  try { doc = parseXml(source) } catch (e) { throw new OpmlError(e instanceof XmlError ? e.message : String(e)) }
  const opml = doc.name === 'opml' ? doc : firstElement(doc, 'opml')
  const body = opml && firstElement(opml, 'body')
  if (!opml || !body) throw new OpmlError('not an OPML document (no <opml><body>)')
  const head = firstElement(opml, 'head')
  const titleEl = head && firstElement(head, 'title')
  const title = titleEl ? titleEl.children.filter((c) => typeof c === 'string').join('').trim() || null : null
  const read = (parent: typeof body): OutlineNode[] => elements(parent, 'outline').map((o) => ({
    text: (o.attrs.text ?? o.attrs.title ?? '').replace(/\s+/g, ' ').trim(),
    children: read(o),
  }))
  return { title, nodes: read(body) }
}

/** An OPML 2.0 document of `nodes`. */
export function formatOpml(nodes: OutlineNode[], title = ''): string {
  const outline = (n: OutlineNode): ReturnType<typeof el> => el('outline', { text: n.text }, n.children.map(outline))
  const doc = el('opml', { version: '2.0' }, [el('head', {}, [el('title', {}, title ? [title] : [])]), el('body', {}, nodes.map(outline))])
  return `<?xml version="1.0" encoding="UTF-8"?>\n${toXml(doc, '', '  ')}\n`
}
