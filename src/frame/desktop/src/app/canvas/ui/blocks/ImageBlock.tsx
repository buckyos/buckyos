/* ── Image block: picture with fit mode; empty state accepts file pick / drop ── */

import clsx from 'clsx'
import { ImagePlus } from 'lucide-react'
import { useState } from 'react'
import { isImageFile } from '../../data/image'
import type { CanvasBlockOf } from '../../domain/types'
import { useEditorActions } from '../actions'
import { Btn } from '../primitives'

export function ImageBlockView({ block }: { block: CanvasBlockOf<'image'> }) {
  const actions = useEditorActions()
  const c = block.content
  const [over, setOver] = useState(false)
  const [broken, setBroken] = useState(false)

  const onDrop = (e: React.DragEvent) => {
    const file = [...e.dataTransfer.files].find(isImageFile)
    if (!file) return
    e.preventDefault()
    e.stopPropagation()
    setOver(false)
    actions.setBlockImage(block.id, file)
  }
  const dragProps = block.locked
    ? {}
    : {
        onDragOver: (e: React.DragEvent) => {
          if ([...e.dataTransfer.items].some((it) => it.kind === 'file')) {
            e.preventDefault()
            e.stopPropagation()
            setOver(true)
          }
        },
        onDragLeave: () => setOver(false),
        onDrop,
      }

  if (!c.src) {
    return (
      <div
        className={clsx('flex h-full w-full flex-col items-center justify-center gap-2 p-3 text-center text-xs text-[color:var(--cp-muted)]', over && 'bg-[color:color-mix(in_srgb,var(--cp-accent)_10%,transparent)]')}
        data-no-drag
        {...dragProps}
        data-testid="aic-image-empty"
      >
        <ImagePlus className="size-[22px]" />
        <p>{over ? '松开以放入图片' : '还没有图片'}</p>
        {!block.locked ? (
          <Btn variant="primary" className="!py-[3px]" onClick={() => actions.pickImageFor(block.id)}>
            选择图片…
          </Btn>
        ) : null}
        <p className="text-[10px] leading-4">也可以把图片文件拖到这里，或在画布上直接 Ctrl/⌘+V 粘贴图片</p>
      </div>
    )
  }

  return (
    <div className={clsx('relative h-full w-full overflow-hidden bg-[color:var(--cp-surface-2-opaque)]', over && 'outline outline-2 outline-[color:var(--cp-accent)]')} {...dragProps} onDoubleClick={(e) => { e.stopPropagation(); if (!block.locked) actions.pickImageFor(block.id) }} title={block.locked ? undefined : '双击替换图片'}>
      {broken ? (
        <div className="flex h-full items-center justify-center p-3 text-center text-xs text-[color:var(--cp-danger)]">图片无法显示（来源可能已失效）</div>
      ) : (
        <img src={c.src} alt={c.alt ?? block.title ?? ''} draggable={false} className="h-full w-full select-none" style={{ objectFit: c.fit }} onError={() => setBroken(true)} data-testid="aic-image" />
      )}
      {c.caption ? <div className="pointer-events-none absolute inset-x-0 bottom-0 truncate bg-[color:color-mix(in_srgb,#000_55%,transparent)] px-2 py-1 text-[11px] text-white">{c.caption}</div> : null}
    </div>
  )
}
