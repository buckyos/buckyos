import useSWR, { type SWRResponse } from 'swr'

export const REFRESH_MS = 4000

export const useQuery = <T>(key: unknown[] | null, fetcher: () => Promise<T>, poll = true): SWRResponse<T, Error> =>
  useSWR<T, Error>(key, fetcher, {
    refreshInterval: poll ? REFRESH_MS : 0,
    revalidateOnFocus: false,
    keepPreviousData: true,
    shouldRetryOnError: false,
  })

const pad = (n: number) => String(n).padStart(2, '0')

export const fmtTime = (ms?: number | null): string => {
  if (!ms) return '–'
  const d = new Date(ms)
  const day = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  return new Date().toDateString() === d.toDateString() ? time : `${day} ${time}`
}

export const fmtAgo = (ms?: number | null): string => {
  if (!ms) return '–'
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000))
  if (s < 60) return `${s}s ago`
  if (s < 3600) return `${Math.floor(s / 60)}m ago`
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`
  return `${Math.floor(s / 86400)}d ago`
}

export const toText = (value: unknown): string =>
  typeof value === 'string' ? value : JSON.stringify(value, null, 2) ?? ''

export const sessionHref = (sid: string) => `#/session/${encodeURIComponent(sid)}`
