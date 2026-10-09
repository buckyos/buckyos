import { useEffect, useLayoutEffect, useRef, type RefObject } from 'react'

interface Anchor {
  ids: string[]
  offset: number
  scrollTop: number
}

const memories = new Map<string, Map<string, Anchor>>()

function memoryFor(scope: string) {
  let memory = memories.get(scope)
  if (!memory) {
    memory = new Map()
    memories.set(scope, memory)
  }
  return memory
}

function capture(container: HTMLElement): Anchor {
  const top = container.getBoundingClientRect().top
  const cards = [...container.querySelectorAll<HTMLElement>('[data-objid]')]
  const index = cards.findIndex(card => card.getBoundingClientRect().bottom > top + 1)
  if (index < 0) return { ids: [], offset: 0, scrollTop: container.scrollTop }
  const first = cards[index]
  return {
    ids: cards.slice(index, index + 6).map(card => card.dataset.objid ?? ''),
    offset: first.getBoundingClientRect().top - top,
    scrollTop: container.scrollTop,
  }
}

function restore(container: HTMLElement, anchor: Anchor | undefined) {
  if (!anchor) {
    container.scrollTop = 0
    return
  }
  const top = container.getBoundingClientRect().top
  for (const id of anchor.ids) {
    const element = container.querySelector<HTMLElement>(`[data-objid="${CSS.escape(id)}"]`)
    if (element) {
      container.scrollTop += element.getBoundingClientRect().top - top - anchor.offset
      return
    }
  }
  container.scrollTop = anchor.scrollTop
}

export function useScrollMemory(containerRef: RefObject<HTMLElement | null>, scope: string, viewKey: string, signature: string, ready: boolean) {
  const lastView = useRef<string | null>(null)

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    let frame = 0
    const onScroll = () => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        if (lastView.current) memoryFor(scope).set(lastView.current, capture(container))
      })
    }
    container.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      cancelAnimationFrame(frame)
      container.removeEventListener('scroll', onScroll)
    }
  }, [containerRef, scope])

  useLayoutEffect(() => {
    const container = containerRef.current
    if (!container) return
    if (!ready) {
      lastView.current = null
      return
    }
    const memory = memoryFor(scope)
    restore(container, memory.get(viewKey))
    lastView.current = viewKey
    memory.set(viewKey, capture(container))
  }, [containerRef, ready, scope, signature, viewKey])
}
