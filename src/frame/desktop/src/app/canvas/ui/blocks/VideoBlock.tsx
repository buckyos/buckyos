/* ── Video block: real <video> when a file exists, otherwise a frame-sequence player ── */

import { Pause, Play, SkipBack } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { CanvasBlockOf } from '../../domain/types'
import { IconBtn } from '../primitives'

function fmt(ms: number): string {
  const s = Math.round(ms / 1000)
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

export function VideoBlockView({ block }: { block: CanvasBlockOf<'video'> }) {
  const c = block.content
  const frames = useMemo(() => c.frames ?? [], [c.frames])
  const [playing, setPlaying] = useState(false)
  const [index, setIndex] = useState(0)
  const [frameKey, setFrameKey] = useState(frames.map((f) => f.src.length).join(','))
  const key = frames.map((f) => f.src.length).join(',')
  if (key !== frameKey) {
    // frames were regenerated → restart from the first one
    setFrameKey(key)
    setIndex(0)
    setPlaying(false)
  }
  const safeIndex = Math.min(index, Math.max(0, frames.length - 1))
  const current = frames[safeIndex]

  useEffect(() => {
    if (!playing || !frames.length) return
    const t = setTimeout(() => setIndex((i) => (i + 1) % frames.length), frames[safeIndex]?.durationMs ?? 1000)
    return () => clearTimeout(t)
  }, [playing, safeIndex, frames])

  if (c.src) {
    return (
      <div className="flex h-full w-full flex-col bg-black">
        <video src={c.src} poster={c.poster} controls className="min-h-0 flex-1 w-full" data-no-drag data-testid="aic-video" />
        {c.caption ? <div className="truncate px-2 py-1 text-[11px] text-[color:var(--cp-muted)]">{c.caption}</div> : null}
      </div>
    )
  }
  if (!frames.length) {
    return <div className="flex h-full items-center justify-center p-3 text-center text-xs text-[color:var(--cp-muted)]">这个视频块还没有内容。运行生成它的许愿格后，这里会出现逐帧预览或视频文件。</div>
  }
  const total = c.durationMs ?? frames.reduce((n, f) => n + f.durationMs, 0)
  const elapsed = frames.slice(0, safeIndex).reduce((n, f) => n + f.durationMs, 0)
  const endPct = ((elapsed + (current?.durationMs ?? 0)) / total) * 100
  const startPct = (elapsed / total) * 100
  return (
    <div className="flex h-full w-full flex-col bg-black" data-testid="aic-video-player">
      <button type="button" className="relative min-h-0 flex-1 overflow-hidden" data-no-drag onClick={() => setPlaying((p) => !p)} aria-label={playing ? '暂停' : '播放'}>
        <img src={current?.src} alt={current?.caption ?? ''} draggable={false} className="h-full w-full select-none object-contain" />
        {!playing ? (
          <span className="absolute inset-0 flex items-center justify-center">
            <span className="inline-flex h-12 w-12 items-center justify-center rounded-full bg-[color:color-mix(in_srgb,#000_55%,transparent)] text-white [&>svg]:size-[22px]"><Play /></span>
          </span>
        ) : null}
        {current?.caption ? <span className="pointer-events-none absolute inset-x-0 bottom-0 truncate bg-[color:color-mix(in_srgb,#000_60%,transparent)] px-2 py-1 text-left text-[11px] text-white">{current.caption}</span> : null}
      </button>
      <div className="flex flex-none items-center gap-1 border-t border-[color:color-mix(in_srgb,#fff_12%,transparent)] px-2 py-1 text-[11px] text-[color:#e5e7eb]" data-no-drag>
        <IconBtn icon={playing ? <Pause /> : <Play />} label={playing ? '暂停' : '播放'} size={24} className="!text-white" onClick={() => setPlaying((p) => !p)} />
        <IconBtn icon={<SkipBack />} label="回到开头" size={24} className="!text-white" onClick={() => { setIndex(0); setPlaying(false) }} />
        <div className="relative mx-1 h-[6px] flex-1 overflow-hidden rounded bg-[color:color-mix(in_srgb,#fff_18%,transparent)]" role="progressbar" aria-valuenow={Math.round(startPct)}>
          <i key={`${safeIndex}-${playing}`} className="absolute inset-y-0 left-0 block bg-[color:var(--cp-accent)]" style={{ width: `${playing ? endPct : startPct}%`, transition: playing ? `width ${current?.durationMs ?? 0}ms linear` : 'none' }} />
        </div>
        <span className="tabular-nums">{fmt(elapsed)} / {fmt(total)}</span>
        <span className="text-[color:#9ca3af]">· 镜头 {safeIndex + 1}/{frames.length}</span>
      </div>
    </div>
  )
}
