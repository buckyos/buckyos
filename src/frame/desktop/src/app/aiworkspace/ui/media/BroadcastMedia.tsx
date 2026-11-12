/* eslint-disable react-refresh/only-export-components -- the player and the composite lookup it is chosen by */
/* A broadcast (控件机制 TODO §0.4 “电视屏”, G16, G17; manifest 4 `composition.media`): the video or sound in a composite's
 * media slot plays where the instance's config says — `{ pos_ms, at_ms, playing }` in `config.<anchor>`: at server time
 * `at_ms` it was at `pos_ms` — so every viewer is at `pos_ms + (server now − at_ms)` (paused: `pos_ms`), whatever their
 * device clock says. It plays by Range from the asset's stream address (no whole download). A viewer's own pause and
 * mute stay on their device, "Back to live" returns; the end holds the last frame unless `config.<loop>` is on. Who may
 * change the composite starts and pauses the broadcast, makes their own position the broadcast's, and changes the video
 * — the asset and a fresh anchor in one Commit, so nobody sees the new video at the old position. */

import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { Pause, Play, Radio, RotateCcw, Upload, Volume2, VolumeX } from 'lucide-react'
import { compositeOf, type CompositionMedia } from '@controls/composite'
import type { Json, KeyedContent, Operation } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useEntity, useStore } from '../../state/hooks'
import { serverClock } from '../../state/serverClock'
import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'
import { correction } from '../../session/media'
import { mediaBlockSize, stageMediaFile } from './stage'
import { createOp } from '../blocks/ops'
import { randomId } from '../../api/ids'
import { useFilePicker } from '../shell/filePicker'

export interface Anchor { pos_ms: number; at_ms: number; playing: boolean }

/** The composite whose media slot holds the Block `cellId`, with its media declaration. */
export function broadcastOf(store: WorkspaceStore, cellId: string | null): { compositeId: string; media: CompositionMedia; config: Record<string, unknown> } | null {
  if (!cellId) return null
  const cell = store.outline.get(cellId)
  const parent = cell?.parent_id ? store.outline.get(cell.parent_id) : undefined
  if (!cell || parent?.kind !== 'composite') return null
  const { composition, config } = compositeOf(store.outline, parent.entity_id)
  if (!composition?.media || composition.media.slot !== cell.slot) return null
  return { compositeId: parent.entity_id, media: composition.media, config }
}

export function anchorOf(v: unknown): Anchor | null {
  if (!v || typeof v !== 'object') return null
  const a = v as Record<string, unknown>
  return typeof a.pos_ms === 'number' && typeof a.at_ms === 'number' ? { pos_ms: Math.max(0, a.pos_ms), at_ms: a.at_ms, playing: a.playing === true } : null
}

/** Where the broadcast is now (ms): the anchor's position, run on by the server clock while it plays; at the end it
 * stays (or loops). */
export function broadcastPosition(anchor: Anchor, serverNow: number, durationMs: number | null, loop: boolean): number {
  const raw = anchor.playing ? anchor.pos_ms + Math.max(0, serverNow - anchor.at_ms) : anchor.pos_ms
  if (!durationMs || durationMs <= 0) return raw
  return loop ? raw % durationMs : Math.min(raw, durationMs)
}

async function writeAnchor(store: WorkspaceStore, compositeId: string, key: string, anchor: Anchor, label: string, extra: Operation[] = []) {
  const read = await store.session.read<KeyedContent<Record<string, Json>>>(compositeId)
  await store.submit({
    editId: `broadcast:${compositeId}`, label,
    operations: [...extra, { op: 'entity.set_props', entity_id: compositeId, key: 'config', props: { [key]: anchor as unknown as Json }, expect: { [key]: read.content.key_revs[`p:config.${key}`] ?? 0 } }],
  })
}

/** A video or sound into the empty media slot of a composite (an empty TV screen): the asset, its Block in the slot and a
 * fresh anchor (from 0, paused until started) in one Commit. */
export async function placeBroadcast(store: WorkspaceStore, compositeId: string, media: CompositionMedia, file: File, label: string) {
  const { facts, payload } = await stageMediaFile(store, file)
  let surface = store.outline.get(compositeId)
  for (let i = 0; surface && surface.kind !== 'surface' && i < 64; i++) surface = surface.parent_id ? store.outline.get(surface.parent_id) : undefined
  const folder = surface?.content_folder_id
  if (!folder) throw new Error(aiwsT('aiworkspace.broadcast.noSurface'))
  const dataId = randomId('d')
  const size = mediaBlockSize(facts)
  const last = (id: string) => store.outline.childrenOf(id).at(-1)?.order_key ?? undefined
  const read = await store.session.read<KeyedContent<Record<string, Json>>>(compositeId)
  await store.submit({
    editId: `broadcast:${compositeId}`, label,
    operations: [
      createOp(dataId, 'buckyos.asset-ref', folder, store.core.order_key_between(last(folder), undefined), { ...payload } as Record<string, Json>, payload.file_name),
      { op: 'entity.create', entity_id: randomId('c'), type_id: 'buckyos.cell', parent_id: compositeId, order_key: store.core.order_key_between(last(compositeId), undefined),
        placement: { x: 'auto', y: 'auto', w: size.w, h: size.h }, payload: { view: { type: 'asset' }, source_ref: { entity_id: dataId }, slot: media.slot } },
      { op: 'entity.set_props', entity_id: compositeId, key: 'config', props: { [media.anchor]: { pos_ms: 0, at_ms: serverClock(store.session).now(), playing: false } }, expect: { [media.anchor]: read.content.key_revs[`p:config.${media.anchor}`] ?? 0 } },
    ],
  })
}

export function BroadcastMedia({ assetId, objectId, kind, name, compositeId, media, config }: {
  assetId: string; objectId: string; kind: 'video' | 'audio'; name: string; compositeId: string; media: CompositionMedia; config: Record<string, unknown>
}) {
  const { t } = useAiwsText()
  const store = useStore()
  const clock = serverClock(store.session)
  useSyncExternalStore(clock.subscribe, clock.snapshot)
  const composite = useEntity(compositeId)
  const canRun = Boolean(composite?.capabilities.includes('update'))
  const anchor = anchorOf(config[media.anchor])
  const loop = media.loop ? config[media.loop] === true : false
  const [url, setUrl] = useState<string | null>(null)
  const [own, setOwn] = useState(false)
  const [muted, setMuted] = useState(true)
  const [blocked, setBlocked] = useState(false)
  const ref = useRef<HTMLVideoElement & HTMLAudioElement>(null)
  useEffect(() => {
    let live = true
    void store.session.assetStreamUrl(objectId).then(async (u) => {
      // offline (no stream address): the bytes, as a picture's
      const src = u ?? URL.createObjectURL(await store.session.fetchAsset(objectId))
      if (live) setUrl(src)
    })
    return () => { live = false }
  }, [store, objectId])
  // following the broadcast: seek beyond 400 ms off, nudge the rate between 80 and 400 ms, pause with it
  useEffect(() => {
    const el = ref.current
    if (!el || !url || own) return
    const apply = () => {
      if (!anchor) { if (!el.paused) el.pause(); return }
      const duration = Number.isFinite(el.duration) ? el.duration * 1000 : null
      const target = broadcastPosition(anchor, clock.now(), duration, loop)
      if (!anchor.playing || (duration !== null && !loop && target >= duration - 50)) {
        if (!el.paused) el.pause()
        if (Math.abs(el.currentTime * 1000 - target) > 80) el.currentTime = target / 1000
        return
      }
      const c = correction(el.currentTime * 1000, target, 1)
      if (c.seek !== undefined) el.currentTime = c.seek / 1000
      if (Math.abs(el.playbackRate - c.playbackRate) > 1e-3) el.playbackRate = c.playbackRate
      if (el.paused) el.play().then(() => setBlocked(false), () => setBlocked(true))
    }
    apply()
    const timer = window.setInterval(apply, 500)
    el.addEventListener('loadedmetadata', apply)
    return () => { window.clearInterval(timer); el.removeEventListener('loadedmetadata', apply) }
  }, [url, own, anchor?.pos_ms, anchor?.at_ms, anchor?.playing, loop, clock]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (ref.current) ref.current.muted = muted }, [muted, url])

  const start = (playing: boolean, posMs?: number) => {
    const el = ref.current
    const duration = el && Number.isFinite(el.duration) ? el.duration * 1000 : null
    const pos = posMs ?? (anchor ? broadcastPosition(anchor, clock.now(), duration, loop) : 0)
    void writeAnchor(store, compositeId, media.anchor, { pos_ms: Math.round(pos), at_ms: clock.now(), playing }, playing ? t('aiworkspace.broadcast.start') : t('aiworkspace.broadcast.pause'))
    setOwn(false)
  }
  const replace = async (file: File) => {
    const { payload } = await stageMediaFile(store, file)
    const read = await store.session.read<KeyedContent<Record<string, Json>>>(assetId)
    const keys = Object.entries(payload).map(([key, value]) => ({ key, value: value as Json, expect: { rev: read.content.key_revs[key] ?? 0 } }))
    // the new video starts from 0, now, playing: in the same Commit as the asset (§0.4)
    await writeAnchor(store, compositeId, media.anchor, { pos_ms: 0, at_ms: clock.now(), playing: true }, t('aiworkspace.broadcast.replace'), [{ op: 'entity.set_keys', entity_id: assetId, keys }])
  }
  // the new video comes from the Zone first (the system picker; this device is a choice in it)
  const picker = useFilePicker((message) => store.notify('error', message))
  const chooseReplacement = async () => {
    const file = await picker.file(kind === 'video' ? 'video/*' : 'audio/*')
    if (file) await replace(file).catch((error: unknown) => store.notify('error', String(error instanceof Error ? error.message : error)))
  }
  if (!url) return <div className="aiws-muted aiws-media-loading">{aiwsT('aiworkspace.session.media.loading')}</div>
  const common = { ref, src: url, preload: 'auto' as const, playsInline: true, muted, controls: own, 'data-testid': `aiws-broadcast-${assetId}`, 'aria-label': name }
  return (
    <div className={`aiws-media-player aiws-broadcast is-${kind}`} data-role="editor" data-live={own ? 'false' : 'true'} data-playing={anchor?.playing ? 'true' : 'false'} onPointerDown={(e) => e.stopPropagation()}>
      {kind === 'video' ? <video {...common} /> : <audio {...common} />}
      <div className="aiws-broadcast-bar">
        {!own ? (
          <span className="aiws-broadcast-live" data-testid="aiws-broadcast-live"><Radio size={13} aria-hidden="true" />{anchor?.playing ? t('aiworkspace.broadcast.live') : t('aiworkspace.broadcast.paused')}</span>
        ) : (
          <button type="button" data-testid="aiws-broadcast-back" onClick={() => setOwn(false)}><Radio size={13} aria-hidden="true" />{t('aiworkspace.broadcast.backToLive')}</button>
        )}
        <button type="button" aria-label={muted ? t('aiworkspace.broadcast.unmute') : t('aiworkspace.broadcast.mute')} title={muted ? t('aiworkspace.broadcast.unmute') : t('aiworkspace.broadcast.mute')} data-testid="aiws-broadcast-mute"
          onClick={() => { setMuted(!muted); setBlocked(false); if (ref.current?.paused && anchor?.playing && !own) void ref.current.play().catch(() => setBlocked(true)) }}>
          {muted ? <VolumeX size={14} aria-hidden="true" /> : <Volume2 size={14} aria-hidden="true" />}
        </button>
        {!own && <button type="button" data-testid="aiws-broadcast-own" title={t('aiworkspace.broadcast.ownHint')} onClick={() => { setOwn(true); ref.current?.pause() }}><Pause size={13} aria-hidden="true" /></button>}
        {canRun && (
          <>
            <span className="aiws-broadcast-sep" />
            {anchor?.playing
              ? <button type="button" data-testid="aiws-broadcast-pause" onClick={() => start(false)}><Pause size={13} aria-hidden="true" />{t('aiworkspace.broadcast.pause')}</button>
              : <button type="button" data-testid="aiws-broadcast-start" onClick={() => start(true)}><Play size={13} aria-hidden="true" />{t('aiworkspace.broadcast.start')}</button>}
            {own && <button type="button" data-testid="aiws-broadcast-set" onClick={() => start(anchor?.playing ?? true, (ref.current?.currentTime ?? 0) * 1000)}>{t('aiworkspace.broadcast.setHere')}</button>}
            <button type="button" aria-label={t('aiworkspace.broadcast.fromStart')} title={t('aiworkspace.broadcast.fromStart')} onClick={() => start(true, 0)}><RotateCcw size={13} aria-hidden="true" /></button>
            <button type="button" aria-label={t('aiworkspace.broadcast.replace')} title={t('aiworkspace.broadcast.replace')} data-testid="aiws-broadcast-replace" onClick={() => void chooseReplacement()}><Upload size={13} aria-hidden="true" /></button>
          </>
        )}
      </div>
      {blocked && <button type="button" className="aiws-session-unmute" data-testid="aiws-broadcast-tap" onClick={() => { setBlocked(false); void ref.current?.play().catch(() => setBlocked(true)) }}>{t('aiworkspace.broadcast.tapToPlay')}</button>}
    </div>
  )
}
