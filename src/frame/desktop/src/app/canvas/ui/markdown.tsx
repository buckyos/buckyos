/* ── Tiny markdown renderer (headings, lists, bold, code, links, quotes) ── */

import type { ReactNode } from 'react'

function inline(text: string, key: number): ReactNode {
  const parts: ReactNode[] = []
  const re = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g
  let last = 0
  let m: RegExpExecArray | null
  let i = 0
  while ((m = re.exec(text))) {
    if (m.index > last) parts.push(text.slice(last, m.index))
    const tok = m[0]
    if (tok.startsWith('**')) parts.push(<strong key={i++}>{tok.slice(2, -2)}</strong>)
    else if (tok.startsWith('`')) parts.push(<code key={i++}>{tok.slice(1, -1)}</code>)
    else {
      const mm = /\[([^\]]+)\]\(([^)]+)\)/.exec(tok)!
      parts.push(
        <a key={i++} href={mm[2]} target="_blank" rel="noreferrer" onClick={(e) => e.stopPropagation()}>
          {mm[1]}
        </a>,
      )
    }
    last = m.index + tok.length
  }
  if (last < text.length) parts.push(text.slice(last))
  return <span key={key}>{parts}</span>
}

export function Markdown({ text }: { text: string }) {
  const lines = text.split(/\r?\n/)
  const out: ReactNode[] = []
  let list: ReactNode[] = []
  let para: string[] = []
  const flushList = () => {
    if (list.length) out.push(<ul key={`ul${out.length}`}>{list}</ul>)
    list = []
  }
  const flushPara = () => {
    if (para.length) out.push(<p key={`p${out.length}`}>{inline(para.join(' '), 0)}</p>)
    para = []
  }
  lines.forEach((raw, idx) => {
    const line = raw.trimEnd()
    if (!line.trim()) {
      flushList()
      flushPara()
      return
    }
    const h = /^(#{1,3})\s+(.*)$/.exec(line)
    if (h) {
      flushList()
      flushPara()
      const Tag = (`h${h[1].length}`) as 'h1' | 'h2' | 'h3'
      out.push(<Tag key={`h${idx}`}>{inline(h[2], 0)}</Tag>)
      return
    }
    const li = /^\s*[-*•]\s+(.*)$/.exec(line) ?? /^\s*\d+[.、]\s+(.*)$/.exec(line)
    if (li) {
      flushPara()
      list.push(<li key={`li${idx}`}>{inline(li[1], 0)}</li>)
      return
    }
    const q = /^>\s?(.*)$/.exec(line)
    if (q) {
      flushList()
      flushPara()
      out.push(<blockquote key={`q${idx}`}>{inline(q[1], 0)}</blockquote>)
      return
    }
    flushList()
    para.push(line)
  })
  flushList()
  flushPara()
  return <div className="aic-md">{out}</div>
}
