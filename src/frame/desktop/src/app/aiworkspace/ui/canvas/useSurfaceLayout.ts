/* The laid-out BlockTree of one free Surface and its spatial index (phase two §9.3), kept current with the outline and the
 * reader's folded mind map nodes:
 * shared by the canvas, the path editor and a show's stage. Connector ends meet the outline and anchors each Block
 * declares; a definition deciding them from its payload is read once (连接线实现方案 §4.3). */

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { inkBook } from '../../ink/book'
import { distanceToInk, INK_VIEW } from '../../ink/model'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { distanceTo } from './connectors/geometry'
import { ShapeBook } from './connectors/layout'
import { foldBook } from '../controls/folds'
import { layoutSurface } from './layout'
import { SpatialIndex } from './render/spatialIndex'

export function useSurfaceLayout(surfaceId: string) {
  const store = useStore()
  const outlineVersion = useOutlineVersion()
  const [shapeBook] = useState(() => new ShapeBook(store))
  const shapesVersion = useSyncExternalStore(shapeBook.subscribe, shapeBook.snapshot)
  // the nodes this reader folded in a control (user state, 脑图方案 R8) are part of the layout
  const folds = foldBook(store)
  const foldsVersion = useSyncExternalStore(folds.subscribe, folds.snapshot)
  const laid = useMemo(() => layoutSurface(store.outline, surfaceId, (id) => shapeBook.get(store.outline.get(id)), (id) => folds.folded(id)),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion / shapesVersion / foldsVersion are the invalidation signals
    [store, surfaceId, outlineVersion, shapesVersion, shapeBook, folds, foldsVersion])
  useEffect(() => {
    const targets = []
    for (const l of laid.values()) for (const end of [l.connector?.data.start, l.connector?.data.end]) { const target = end ? laid.get(end.entity_id)?.entity : undefined; if (target) targets.push(target) }
    shapeBook.ensure(targets)
  }, [laid, shapeBook])
  // ink hits where its line is once its Block drew it (手写笔 TODO D3): a circled object stays reachable inside the circle
  const book = inkBook(store)
  const inkVersion = useSyncExternalStore(book.subscribe, book.snapshot)
  const index = useMemo(() => {
    const idx = new SpatialIndex()
    for (const [id, l] of laid) {
      const line = l.connector
      const ink = l.entity.view_type === INK_VIEW ? book.world(id, l.rect, l.rotation) : null
      idx.insert({ id, rect: l.bounds, paint: l.paint, ...(l.rotation ? { turned: { rect: l.rect, rotation: l.rotation } } : {}),
        ...(line ? { line: { flat: line.geom.flat, label: line.geom.label.box, distance: (p: { x: number; y: number }) => distanceTo(line.geom, p) } } : {}),
        ...(ink ? { line: { flat: ink, label: null, distance: (p: { x: number; y: number }) => distanceToInk(ink, p) } } : {}) })
    }
    return idx
    // eslint-disable-next-line react-hooks/exhaustive-deps -- inkVersion is the invalidation signal of the decoded ink
  }, [laid, book, inkVersion])
  return { laid, index, shapeBook }
}
