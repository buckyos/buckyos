export type ImageDownloadProgress = {
  loadedBytes: number
  totalBytes: number | null
  progressPercent: number | null
}

const parseTotalBytes = (raw: string | null) => {
  if (!raw) {
    return null
  }

  const value = Number.parseInt(raw, 10)
  if (!Number.isFinite(value) || value <= 0) {
    return null
  }

  return value
}

const readDownloadError = async (response: Response) => {
  const fallback = `Failed to load image (${response.status})`
  const contentType = response.headers.get('content-type') || ''

  if (contentType.includes('application/json')) {
    const payload = (await response.json().catch(() => null)) as { error?: unknown } | null
    if (payload && typeof payload.error === 'string' && payload.error.trim()) {
      return payload.error
    }
    return fallback
  }

  const text = await response.text().catch(() => '')
  return text.trim() || fallback
}

export const downloadImageWithProgress = async (
  url: string,
  signal: AbortSignal,
  onProgress: (progress: ImageDownloadProgress) => void,
) => {
  const response = await fetch(url, {
    signal,
    credentials: 'same-origin',
  })

  if (!response.ok) {
    throw new Error(await readDownloadError(response))
  }

  const totalBytes = parseTotalBytes(response.headers.get('content-length'))
  const contentType = response.headers.get('content-type') || 'application/octet-stream'

  if (!response.body) {
    const blob = await response.blob()
    const finalSize = blob.size
    onProgress({
      loadedBytes: finalSize,
      totalBytes: totalBytes ?? finalSize,
      progressPercent: 100,
    })
    return blob
  }

  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let loadedBytes = 0

  onProgress({
    loadedBytes,
    totalBytes,
    progressPercent: totalBytes === 0 ? 100 : totalBytes ? 0 : null,
  })

  while (true) {
    const { done, value } = await reader.read()
    if (done) {
      break
    }

    if (!value || value.byteLength === 0) {
      continue
    }

    chunks.push(value)
    loadedBytes += value.byteLength
    onProgress({
      loadedBytes,
      totalBytes,
      progressPercent: totalBytes ? Math.min(100, Math.round((loadedBytes / totalBytes) * 100)) : null,
    })
  }

  const finalTotalBytes = totalBytes ?? loadedBytes
  onProgress({
    loadedBytes,
    totalBytes: finalTotalBytes,
    progressPercent: 100,
  })

  const merged = new Uint8Array(loadedBytes)
  let offset = 0
  for (const chunk of chunks) {
    merged.set(chunk, offset)
    offset += chunk.byteLength
  }

  return new Blob([merged.buffer as ArrayBuffer], { type: contentType })
}
