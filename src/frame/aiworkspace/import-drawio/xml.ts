/* A small XML reader and writer for draw.io files, mxStencil sets and SVG fragments. No DOM: the same code runs
 * in the browser, in Deno (the aiws program host) and in tests. Elements, attributes, text, CDATA, comments,
 * processing instructions and the DOCTYPE are understood; entities are the five predefined ones and numeric
 * character references (draw.io writes nothing else). Namespaces are kept as part of the names. */

export interface XmlElement {
  name: string
  attrs: Record<string, string>
  children: XmlNode[]
}
export type XmlNode = XmlElement | string

export class XmlError extends Error {}

const NAMED: Record<string, string> = { lt: '<', gt: '>', amp: '&', quot: '"', apos: "'", nbsp: ' ' }

export function decodeEntities(text: string): string {
  if (!text.includes('&')) return text
  return text.replace(/&(#x[0-9a-fA-F]+|#[0-9]+|[A-Za-z]+);/g, (all, ref: string) => {
    if (ref[0] === '#') {
      const code = ref[1] === 'x' || ref[1] === 'X' ? parseInt(ref.slice(2), 16) : parseInt(ref.slice(1), 10)
      return Number.isFinite(code) && code >= 0 && code <= 0x10ffff ? String.fromCodePoint(code) : all
    }
    return NAMED[ref] ?? all
  })
}

const NAME_END = /[\s/>=]/

/** Parse a document (or a fragment with one root element). */
export function parseXml(source: string): XmlElement {
  let i = 0
  const n = source.length
  const root: XmlElement = { name: '#document', attrs: {}, children: [] }
  const stack: XmlElement[] = [root]
  const fail = (message: string): never => { throw new XmlError(`${message} at ${i}`) }
  while (i < n) {
    const lt = source.indexOf('<', i)
    const textEnd = lt < 0 ? n : lt
    if (textEnd > i) {
      const text = source.slice(i, textEnd)
      if (stack.length > 1) stack[stack.length - 1].children.push(decodeEntities(text))
      else if (text.trim()) fail('text outside the root element')
    }
    if (lt < 0) break
    i = lt
    if (source.startsWith('<!--', i)) {
      const end = source.indexOf('-->', i + 4)
      if (end < 0) fail('unterminated comment')
      i = end + 3
    } else if (source.startsWith('<![CDATA[', i)) {
      const end = source.indexOf(']]>', i + 9)
      if (end < 0) fail('unterminated CDATA')
      stack[stack.length - 1].children.push(source.slice(i + 9, end))
      i = end + 3
    } else if (source.startsWith('<?', i)) {
      const end = source.indexOf('?>', i + 2)
      if (end < 0) fail('unterminated processing instruction')
      i = end + 2
    } else if (source.startsWith('<!', i)) {
      // DOCTYPE, possibly with an internal subset
      let depth = 0
      let j = i + 2
      for (; j < n; j++) {
        const c = source[j]
        if (c === '[') depth += 1
        else if (c === ']') depth -= 1
        else if (c === '>' && depth <= 0) break
      }
      i = j + 1
    } else if (source[i + 1] === '/') {
      const end = source.indexOf('>', i + 2)
      if (end < 0) fail('unterminated end tag')
      const name = source.slice(i + 2, end).trim()
      const top = stack.pop()
      if (!top || top === root || top.name !== name) fail(`unexpected </${name}>`)
      i = end + 1
    } else {
      let j = i + 1
      while (j < n && !NAME_END.test(source[j])) j++
      const name = source.slice(i + 1, j)
      if (!name) fail('empty tag name')
      const element: XmlElement = { name, attrs: {}, children: [] }
      let selfClosing = false
      for (;;) {
        while (j < n && /\s/.test(source[j])) j++
        if (j >= n) fail(`unterminated <${name}>`)
        if (source[j] === '>') { j += 1; break }
        if (source[j] === '/' && source[j + 1] === '>') { selfClosing = true; j += 2; break }
        let k = j
        while (k < n && !NAME_END.test(source[k])) k++
        const attr = source.slice(j, k)
        if (!attr) fail(`bad attribute in <${name}>`)
        while (k < n && /\s/.test(source[k])) k++
        if (source[k] !== '=') { element.attrs[attr] = ''; j = k; continue }
        k += 1
        while (k < n && /\s/.test(source[k])) k++
        const quote = source[k]
        if (quote !== '"' && quote !== "'") fail(`unquoted attribute ${attr}`)
        const close = source.indexOf(quote, k + 1)
        if (close < 0) fail(`unterminated attribute ${attr}`)
        element.attrs[attr] = decodeEntities(source.slice(k + 1, close))
        j = close + 1
      }
      stack[stack.length - 1].children.push(element)
      if (!selfClosing) stack.push(element)
      i = j
    }
  }
  if (stack.length > 1) throw new XmlError(`unclosed <${stack[stack.length - 1].name}>`)
  const first = root.children.find((c): c is XmlElement => typeof c !== 'string')
  if (!first) throw new XmlError('no root element')
  return first
}

export function isElement(node: XmlNode | undefined): node is XmlElement {
  return typeof node === 'object' && node !== null
}

export function elements(parent: XmlElement, name?: string): XmlElement[] {
  return parent.children.filter((c): c is XmlElement => typeof c !== 'string' && (name === undefined || c.name === name))
}

export function firstElement(parent: XmlElement, name?: string): XmlElement | undefined {
  return elements(parent, name)[0]
}

/** Depth-first search for elements with this name (the element itself included). */
export function findAll(parent: XmlElement, name: string, out: XmlElement[] = []): XmlElement[] {
  if (parent.name === name) out.push(parent)
  for (const child of parent.children) if (typeof child !== 'string') findAll(child, name, out)
  return out
}

/** The concatenated text content. */
export function textOf(node: XmlNode): string {
  return typeof node === 'string' ? node : node.children.map(textOf).join('')
}

export function escapeText(text: string): string {
  return text.replace(/[&<>]/g, (c) => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : '&gt;'))
}

export function escapeAttr(text: string): string {
  return text.replace(/[&<>"\n\r\t]/g, (c) => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : c === '>' ? '&gt;' : c === '"' ? '&quot;' : c === '\n' ? '&#10;' : c === '\r' ? '&#13;' : '&#9;'))
}

/** Serialise an element (attributes in insertion order; empty elements self-closed). */
export function toXml(node: XmlNode, indent = '', step = ''): string {
  if (typeof node === 'string') return escapeText(node)
  const attrs = Object.entries(node.attrs).map(([k, v]) => ` ${k}="${escapeAttr(v)}"`).join('')
  if (node.children.length === 0) return `${indent}<${node.name}${attrs}/>`
  const onlyElements = step !== '' && node.children.every((c) => typeof c !== 'string')
  if (!onlyElements) return `${indent}<${node.name}${attrs}>${node.children.map((c) => toXml(c)).join('')}</${node.name}>`
  const inner = node.children.map((c) => toXml(c, indent + step, step)).join('\n')
  return `${indent}<${node.name}${attrs}>\n${inner}\n${indent}</${node.name}>`
}

export function el(name: string, attrs: Record<string, string | number | undefined> = {}, children: XmlNode[] = []): XmlElement {
  const clean: Record<string, string> = {}
  for (const [k, v] of Object.entries(attrs)) if (v !== undefined) clean[k] = String(v)
  return { name, attrs: clean, children }
}
