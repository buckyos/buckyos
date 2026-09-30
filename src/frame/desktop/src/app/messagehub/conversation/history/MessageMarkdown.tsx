import type { ReactNode } from 'react'

const FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([\w+#.-]*)/
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/
const RULE = /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/
const QUOTE = /^\s{0,3}>\s?(.*)$/
const LIST_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/
const TABLE_DIVIDER = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/
const INLINE = /`([^`\n]+)`|\*\*(.+?)\*\*|__(.+?)__|~~(.+?)~~|\*(?![\s*])(.+?)(?<![\s*])\*|\[([^\]\n]+)\]\(\s*([^)\s]+)(?:\s+"[^"]*")?\s*\)|(https?:\/\/[^\s<>\u3000-\u9fff\uff00-\uffef]*[^\s<>.,;:!?'")\]}\u3000-\u9fff\uff00-\uffef])/g

function safeHref(href: string): string | null {
  return /^(https?:|mailto:)/i.test(href) ? href : null
}

function renderInline(text: string, key: string): ReactNode[] {
  const nodes: ReactNode[] = []
  let last = 0
  let index = 0
  for (const match of text.matchAll(INLINE)) {
    const start = match.index ?? 0
    if (start > last) nodes.push(text.slice(last, start))
    const id = `${key}.${index++}`
    const [token, code, strong, strongAlt, strike, emphasis, label, href, url] = match
    if (code !== undefined) nodes.push(<code key={id}>{code}</code>)
    else if (strong !== undefined || strongAlt !== undefined) nodes.push(<strong key={id}>{renderInline(strong ?? strongAlt, id)}</strong>)
    else if (strike !== undefined) nodes.push(<del key={id}>{renderInline(strike, id)}</del>)
    else if (emphasis !== undefined) nodes.push(<em key={id}>{renderInline(emphasis, id)}</em>)
    else if (label !== undefined) {
      const target = safeHref(href)
      nodes.push(target ? <a key={id} href={target} target="_blank" rel="noreferrer noopener">{renderInline(label, id)}</a> : label)
    } else if (url !== undefined) nodes.push(<a key={id} href={url} target="_blank" rel="noreferrer noopener">{url}</a>)
    else nodes.push(token)
    last = start + token.length
  }
  if (last < text.length) nodes.push(text.slice(last))
  return nodes
}

function renderLines(lines: string[], key: string): ReactNode[] {
  return lines.flatMap((line, index) => index === 0 ? renderInline(line, `${key}.${index}`) : [<br key={`${key}.br${index}`} />, ...renderInline(line, `${key}.${index}`)])
}

function indentOf(line: string): number {
  return line.length - line.trimStart().length
}

function splitRow(line: string): string[] {
  return line.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map(cell => cell.trim())
}

function startsBlock(lines: string[], index: number): boolean {
  const line = lines[index]
  return FENCE.test(line) || HEADING.test(line) || RULE.test(line) || QUOTE.test(line) || (LIST_ITEM.exec(line)?.[1].length ?? 99) < 4 || (line.includes('|') && TABLE_DIVIDER.test(lines[index + 1] ?? '') && (lines[index + 1] ?? '').includes('-'))
}

function parseBlocks(source: string[], key: string): ReactNode[] {
  const blocks: ReactNode[] = []
  let index = 0
  while (index < source.length) {
    const line = source[index]
    const id = `${key}.${blocks.length}`
    if (!line.trim()) { index++; continue }

    const fence = FENCE.exec(line)
    if (fence) {
      const body: string[] = []
      index++
      while (index < source.length && !source[index].trimStart().startsWith(fence[1])) body.push(source[index++])
      index++
      blocks.push(<pre key={id}><code data-language={fence[2] || undefined}>{body.join('\n')}</code></pre>)
      continue
    }

    const heading = HEADING.exec(line)
    if (heading) {
      blocks.push(<p key={id} className="mh-md-heading" data-level={heading[1].length}>{renderInline(heading[2], id)}</p>)
      index++
      continue
    }

    if (RULE.test(line)) {
      blocks.push(<hr key={id} />)
      index++
      continue
    }

    if (QUOTE.test(line)) {
      const body: string[] = []
      while (index < source.length && source[index].trim() && QUOTE.test(source[index])) body.push(QUOTE.exec(source[index++])![1])
      blocks.push(<blockquote key={id}>{parseBlocks(body, id)}</blockquote>)
      continue
    }

    const first = LIST_ITEM.exec(line)
    if (first && first[1].length < 4) {
      const indent = first[1].length
      const ordered = /\d/.test(first[2])
      const items: string[][] = []
      while (index < source.length) {
        const current = source[index]
        const item = LIST_ITEM.exec(current)
        if (item && item[1].length === indent && /\d/.test(item[2]) === ordered) {
          items.push([item[3]])
          index++
          continue
        }
        if (!current.trim()) {
          const next = source.slice(index + 1).find(value => value.trim())
          const nextItem = next ? LIST_ITEM.exec(next) : null
          if (next && (indentOf(next) > indent || (nextItem && nextItem[1].length === indent && /\d/.test(nextItem[2]) === ordered))) {
            items.at(-1)!.push('')
            index++
            continue
          }
          break
        }
        if (indentOf(current) > indent) {
          items.at(-1)!.push(current.slice(Math.min(indentOf(current), indent + first[2].length + 1)))
          index++
          continue
        }
        break
      }
      const children = items.map((item, position) => {
        const itemId = `${id}.${position}`
        const simple = item.every(value => value.trim() && !startsBlock([value], 0))
        return <li key={itemId}>{simple ? renderLines(item, itemId) : parseBlocks(item, itemId)}</li>
      })
      const start = ordered ? Number.parseInt(first[2], 10) : 1
      blocks.push(ordered ? <ol key={id} start={start === 1 ? undefined : start}>{children}</ol> : <ul key={id}>{children}</ul>)
      continue
    }

    if (line.includes('|') && TABLE_DIVIDER.test(source[index + 1] ?? '') && (source[index + 1] ?? '').includes('-')) {
      const header = splitRow(line)
      const rows: string[][] = []
      index += 2
      while (index < source.length && source[index].trim() && source[index].includes('|')) rows.push(splitRow(source[index++]))
      blocks.push(
        <div key={id} className="mh-md-table">
          <table>
            <thead><tr>{header.map((cell, column) => <th key={column}>{renderInline(cell, `${id}.h${column}`)}</th>)}</tr></thead>
            <tbody>{rows.map((row, rowIndex) => <tr key={rowIndex}>{header.map((_, column) => <td key={column}>{renderInline(row[column] ?? '', `${id}.${rowIndex}.${column}`)}</td>)}</tr>)}</tbody>
          </table>
        </div>,
      )
      continue
    }

    const paragraph: string[] = [line]
    index++
    while (index < source.length && source[index].trim() && !startsBlock(source, index)) paragraph.push(source[index++])
    blocks.push(<p key={id}>{renderLines(paragraph, id)}</p>)
  }
  return blocks
}

export function MessageMarkdown({ text }: { text: string }) {
  return <div className="mh-markdown">{parseBlocks(text.replace(/\r\n?/g, '\n').split('\n'), 'md')}</div>
}
