/* An image served from the asset route: fetched with the session token and shown through a blob
 * URL (the route serves unlisted types as octet-stream, so the real media type is reapplied). */

import { useEffect, useState } from 'react'
import { describeError } from '../../api/session'
import { useStore } from '../../state/hooks'

export function AssetBlobImage({ objectId, mediaType, alt, fit = 'contain' }: { objectId: string; mediaType: string; alt: string; fit?: 'contain' | 'cover' }) {
  const store = useStore()
  const [state, setState] = useState<{ url: string | null; error: string | null }>({ url: null, error: null })
  useEffect(() => {
    let live = true
    let url: string | null = null
    store.session.fetchAsset(objectId).then(
      (blob) => {
        if (!live) return
        url = URL.createObjectURL(blob.type === mediaType ? blob : new Blob([blob], { type: mediaType }))
        setState({ url, error: null })
      },
      (error: unknown) => { if (live) setState({ url: null, error: describeError(error) }) },
    )
    return () => { live = false; if (url) URL.revokeObjectURL(url) }
  }, [store, objectId, mediaType])
  if (state.error) return <div className="aiws-error">{state.error}</div>
  if (!state.url) return <div className="aiws-muted">读取图片…</div>
  return <img className="aiws-blob-image" src={state.url} alt={alt} style={{ objectFit: fit }} data-object-id={objectId} />
}
