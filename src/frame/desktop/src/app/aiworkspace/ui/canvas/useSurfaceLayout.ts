/* The laid-out BlockTree of one free Surface and its spatial index (phase two §9.3), kept current with the outline:
 * shared by the canvas, the path editor and a show's stage. Connector ends meet the outline and anchors each Block
 * declares; a definition deciding them from its payload is read once (连接线实现方案 §4.3). */

import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { useOutlineVersion, useStore } from '../../state/hooks'
import { distanceTo } from './connectors/geometry'
import { ShapeBook } from './connectors/layout'
import { layoutSurface } from './layout'
import { SpatialIndex } from './render/spatialIndex'

export function useSurfaceLayout(surfaceId: string) {
  const store = useStore()
  const outlineVersion = useOutlineVersion()
  const [shapeBook] = useState(() => new ShapeBook(store))
  const shapesVersion = useSyncExternalStore(shapeBook.subscribe, shapeBook.snapshot)
  const laid = useMemo(() => layoutSurface(store.outline, surfaceId, (id) => shapeBook.get(store.outline.get(id))),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion / shapesVersion are the invalidation signals
    [store, surfaceId, outlineVersion, shapesVersion, shapeBook])
  useEffect(() => {
    const targets = []
    for (const l of laid.values()) for (const end of [l.connector?.data.start, l.connector?.data.end]) { const target = end ? laid.get(end.entity_id)?.entity : undefined; if (target) targets.push(target) }
    shapeBook.ensure(targets)
  }, [laid, shapeBook])
  const index = useMemo(() => {
    const idx = new SpatialIndex()
    for (const [id, l] of laid) {
      const line = l.connector
      idx.insert({ id, rect: l.bounds, paint: l.paint, ...(l.rotation ? { turned: { rect: l.rect, rotation: l.rotation } } : {}),
        ...(line ? { line: { flat: line.geom.flat, label: line.geom.label.box, distance: (p: { x: number; y: number }) => distanceTo(line.geom, p) } } : {}) })
    }
    return idx
  }, [laid])
  return { laid, index, shapeBook }
}
