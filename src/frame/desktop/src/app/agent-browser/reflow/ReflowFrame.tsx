import { useCallback, useEffect, useRef, useState } from 'react'
import { LoadingOrb } from '../../../components/LoadingOrb'
import { useI18n } from '../../../i18n/provider'
import { AppLaunchStatus } from '../../web-frame/AppLaunchStatus'
import { getReflowSource, type PageLoad } from './source'
import { useReflowController } from './useReflow'

interface Loaded { key: string; origin: string; page: PageLoad }

/**
 * The reflow page in a frame (protocol §5.4–5.5). Every load signs the user in on the pages' origin again (its
 * session lasts 15 minutes). Messages are taken only from this frame's window and only from that origin; what goes
 * to the page is addressed to that origin.
 */
export function ReflowFrame({ view, frameKey }: { view: string; frameKey: number }) {
  const controller = useReflowController()
  const { t } = useI18n()
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
  useEffect(() => {
    if (!origin) return
    const onMessage = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow || event.origin !== origin) return
      if ((event.data as { type?: unknown } | null)?.type === 'reflow.ready') setDrawn(key)
      controller.onPageMessage(event.data)
    }
    window.addEventListener('message', onMessage)
    controller.setPage((message) => frame.current?.contentWindow?.postMessage(message, origin))
    return () => {
      window.removeEventListener('message', onMessage)
      controller.setPage(null)
    }
  }, [origin, key, controller])
  const retry = useCallback(() => setAttempt((value) => value + 1), [])
  const title = t('agentBrowser.reflow.frameTitle', 'Reflow page')
  const page = current?.page
  if (!page || page.kind !== 'ready') {
    return <div className="ab-reflow-frame-status" data-testid="ab-reflow-frame-status"><AppLaunchStatus outcome={page ? { kind: page.kind } : undefined} title={title} onRetry={retry} /></div>
  }
  return (
    <>
      <iframe
        key={key}
        ref={frame}
        className="ab-reflow-frame"
        data-testid="ab-reflow-frame"
        data-view={view}
        data-drawn={drawn === key ? 'true' : 'false'}
        title={title}
        {...('src' in page ? { src: page.src } : { srcDoc: page.srcdoc })}
        onLoad={() => setDrawn(key)}
      />
      {drawn === key ? null : <div className="ab-reflow-frame-status" aria-busy="true"><LoadingOrb size={36} /></div>}
    </>
  )
}
