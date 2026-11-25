import { useCallback, useEffect, useRef, useState } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { AppLaunchStatus } from '../../web-frame/AppLaunchStatus'
import { getReflowSource, type PageLoad } from './source'

interface Loaded { key: string; origin: string; page: PageLoad }

/** How long a page that waits for its owner's answer is given to take it in before it shows. */
const ANSWER_MS = 60
/** How long a page that loaded and does not say it is drawn stays covered. */
const SILENT_MS = 4000

/** What a frame's owner does with its page: takes its messages, and gets the way to write to it while it is there. */
export interface FramePage {
  /** The message with which the page says it is drawn. */
  ready: string
  /**
   * The page is shown only once it said it is drawn and the owner answered (with how to draw: the theme), so it does
   * not flash in another one; the frame's `load` alone shows it after a while. Without this, `load` counts as drawn.
   */
  awaitReady?: boolean
  onMessage(data: unknown): void
  setPage(post: ((message: { type: string }) => void) | null): void
}

/**
 * A page of `reflow.<zone>` in a frame: the personal site, or a page tidied by itself (Agent Browser Protocol §5.4,
 * Personal Site Protocol §6.2). Every load signs the user in on the pages' origin again (its session lasts 15
 * minutes). Messages are taken only from this frame's window and only from that origin; what goes to the page is
 * addressed to that origin.
 */
export function ReflowFrame({ view, frameKey, page: owner, title, testId = 'ab-reflow-frame' }: { view: string; frameKey: number; page: FramePage; title: string; testId?: string }) {
  const frame = useRef<HTMLIFrameElement>(null)
  const [attempt, setAttempt] = useState(0)
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [drawn, setDrawn] = useState<string | null>(null)
  const key = `${frameKey}\n${attempt}\n${view}`
  useEffect(() => {
    let live = true
    void getReflowSource()
      .then(async (source) => ({ origin: source.origin, page: await source.openPage(view) }))
      .catch((): Omit<Loaded, 'key'> => ({ origin: '', page: { kind: 'failed' } }))
      .then((result) => { if (live) setLoaded({ key, ...result }) })
    return () => { live = false }
  }, [key, view])
  const current = loaded?.key === key ? loaded : null
  const origin = current?.page.kind === 'ready' ? current.origin : null
  const reveal = useRef(0)
  /** The load whose page said it is drawn: its `load` event, which may come after, changes nothing then. */
  const ready = useRef('')
  const show = useCallback((after: number) => {
    window.clearTimeout(reveal.current)
    if (after > 0) reveal.current = window.setTimeout(() => setDrawn(key), after)
    else setDrawn(key)
  }, [key])
  useEffect(() => () => window.clearTimeout(reveal.current), [key])
  useEffect(() => {
    if (!origin) return
    const onMessage = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow || event.origin !== origin) return
      owner.onMessage(event.data)
      if ((event.data as { type?: unknown } | null)?.type !== owner.ready) return
      ready.current = key
      show(owner.awaitReady ? ANSWER_MS : 0)
    }
    window.addEventListener('message', onMessage)
    owner.setPage((message) => frame.current?.contentWindow?.postMessage(message, origin))
    return () => {
      window.removeEventListener('message', onMessage)
      owner.setPage(null)
    }
  }, [origin, key, owner, show])
  const retry = useCallback(() => setAttempt((value) => value + 1), [])
  const page = current?.page
  if (!page || page.kind !== 'ready') {
    return <div className="ab-reflow-frame-status" data-testid={`${testId}-status`}><AppLaunchStatus outcome={page ? { kind: page.kind } : undefined} title={title} onRetry={retry} /></div>
  }
  return (
    <>
      <iframe
        key={key}
        ref={frame}
        className="ab-reflow-frame"
        data-testid={testId}
        data-view={view}
        data-drawn={drawn === key ? 'true' : 'false'}
        title={title}
        {...('src' in page ? { src: page.src } : { srcDoc: page.srcdoc })}
        onLoad={() => { if (ready.current !== key) show(owner.awaitReady ? SILENT_MS : 0) }}
      />
      {drawn === key ? null : <div className="ab-reflow-frame-status" aria-busy="true"><LoadingOrb size={36} /></div>}
    </>
  )
}
