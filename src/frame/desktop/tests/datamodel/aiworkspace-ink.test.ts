/* AI Workspace pen ink (notepads/aiworkspace-handwriting-todo.md D0–D2): the stroke format, simplification, segments
 * within the config budget, decoding, hit tests and the input state machine; writes the fixture the core replays.
 *
 *   deno test --no-check --allow-env --allow-read --allow-write --node-modules-dir=manual --unstable-sloppy-imports tests/datamodel/aiworkspace-ink.test.ts
 *
 * `AIWS_REGEN=1` rewrites src/frame/aiworkspace/fixtures/ink/strokes.json (the core's `tests/ink.rs` commits it and
 * reads it back); otherwise the encoder must still produce it byte for byte. */

import { InkInput, type InkHost, type InkPointerEvent } from '../../src/app/aiworkspace/ink/input.ts'
import {
  configBytes, encodeStroke, MAX_CONFIG_BYTES, pieces, RAW_BUDGET, rawBytes, readInk, runs, samplesOf, settleChannels, simplify, SEGMENT_BUDGET,
  touches, distanceToInk, widePoints, widthAt, worldPoints, type InkPoint, type InkStyle, type PointerSample,
} from '../../src/app/aiworkspace/ink/model.ts'

function equal(actual: unknown, expected: unknown, what = '') {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`${what} expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}
function ok(value: unknown, what: string) {
  if (!value) throw new Error(what)
}
const near = (a: number, b: number, eps = 0.011) => Math.abs(a - b) <= eps

const PEN: InkStyle = { color: '#111827', width: 4, pressure: true }
const MOUSE: InkStyle = { color: '#2563eb', width: 2, pressure: false }

Deno.test('width: half pressure is the base width, zero pressure stays visible, no pressure is the base width', () => {
  equal(widthAt(PEN, 0.5), 4)
  ok(near(widthAt(PEN, 0), 1.2, 1e-9), 'zero pressure is 0.3 of the width, not swallowed into 0.5')
  ok(near(widthAt(PEN, 1), 6.8, 1e-9), 'full pressure')
  equal(widthAt(PEN, undefined), 4)
  equal(widthAt({ ...PEN, pressure: false }, 1), 4)
})

Deno.test('sampling: coalesced events or the event itself, never both, oldest first', () => {
  const base: PointerSample = { pointerId: 1, pointerType: 'pen', clientX: 0, clientY: 0, pressure: 0.4, timeStamp: 30 }
  equal(samplesOf(base).length, 1, 'no API')
  equal(samplesOf({ ...base, getCoalescedEvents: () => [] }).length, 1, 'empty list')
  const list = [{ ...base, timeStamp: 20, clientX: 2 }, { ...base, timeStamp: 10, clientX: 1 }, base]
  equal(samplesOf({ ...base, getCoalescedEvents: () => list }).map((s) => s.timeStamp), [10, 20, 30])
})

Deno.test('channels: a pen without pressure (always 0.5) and a flat pen keep neither pressure nor tilt', () => {
  const flat: InkPoint[] = [{ x: 0, y: 0, t: 0, p: 0.5 }, { x: 1, y: 1, t: 5, p: 0.5 }]
  equal(settleChannels(flat), [{ x: 0, y: 0, t: 0 }, { x: 1, y: 1, t: 5 }])
  const real: InkPoint[] = [{ x: 0, y: 0, t: 0, p: 0.2 }, { x: 1, y: 1, t: 5, p: 0.5, tx: 10, ty: -5 }]
  equal(settleChannels(real), real)
  // a point without tilt in a tilted stroke is stored flat
  equal(encodeStroke(real, PEN, 'ktilt')[0].config.points.tilt_x, [0, 10])
})

Deno.test('simplify: a straight line keeps its ends, a corner stays, a pressure change at rest stays, the cost is bounded', () => {
  const line: InkPoint[] = Array.from({ length: 200 }, (_, i) => ({ x: i, y: 0, t: i }))
  equal(simplify(line, 0.25, MOUSE).map((q) => q.x), [0, 199])
  const corner: InkPoint[] = [...Array.from({ length: 50 }, (_, i) => ({ x: i, y: 0, t: i })), ...Array.from({ length: 50 }, (_, i) => ({ x: 49, y: i + 1, t: 50 + i }))]
  ok(simplify(corner, 0.25, MOUSE).some((q) => q.x === 49 && q.y === 0), 'the corner is kept')
  const resting: InkPoint[] = [{ x: 0, y: 0, t: 0, p: 0.2 }, { x: 0, y: 0, t: 10, p: 0.9 }, { x: 0, y: 0, t: 20, p: 0.2 }]
  equal(simplify(resting, 0.25, PEN).length, 3, 'pressure changing while the pen rests')
  const spiral: InkPoint[] = Array.from({ length: 40_000 }, (_, i) => ({ x: Math.cos(i / 7) * i / 10, y: Math.sin(i / 7) * i / 10, t: i, p: 0.5 + 0.4 * Math.sin(i / 50) }))
  const started = performance.now()
  const kept = simplify(spiral, 0.25, PEN)
  ok(performance.now() - started < 3000, 'bounded')
  equal([kept[0], kept.at(-1)], [spiral[0], spiral.at(-1)])
})

Deno.test('encode: a tap is a dot inside a padded frame; a flat line has a frame of some height', () => {
  const [tap] = encodeStroke([{ x: 100.123, y: 50.5, t: 0, p: 0.7 }], PEN, 'ktap')
  ok(tap.config.frame.w >= 4 && tap.config.frame.h >= 4, 'padded by the widest half-width')
  equal(tap.config.points.x.length, 1)
  ok(near(tap.rect.x + tap.config.points.x[0], 100.12), 'world x survives (2 decimals)')
  const [flat] = encodeStroke([{ x: 0, y: 10, t: 0 }, { x: 300, y: 10, t: 40 }], MOUSE, 'kflat')
  ok(flat.config.frame.h > 0 && flat.config.frame.w > 300, 'no zero size')
  equal(flat.config.points.p, undefined, 'no pressure channel for a mouse')
  equal(Number.isInteger(flat.rect.x) && Number.isInteger(flat.rect.w), true)
})

/** A long scribble with real pressure. */
function scribble(n: number): InkPoint[] {
  return Array.from({ length: n }, (_, i) => ({ x: 1000 + Math.cos(i / 3) * 80 + i * 0.37, y: -500 + Math.sin(i / 2.7) * 60 + i * 0.11, t: i * 4, p: Math.round((0.35 + 0.3 * Math.sin(i / 13)) * 1000) / 1000 }))
}

Deno.test('encode: a long stroke is cut into segments that stay within the budget and share their boundary points', () => {
  const points = scribble(12_000)
  const segments = encodeStroke(points, PEN, 'klong')
  ok(segments.length > 1, `segments: ${segments.length}`)
  for (const [i, s] of segments.entries()) {
    const bytes = configBytes(s.config)
    ok(bytes <= SEGMENT_BUDGET && bytes < MAX_CONFIG_BYTES, `segment ${i}: ${bytes} bytes`)
    equal([s.config.seg, s.config.segs, s.config.stroke], [i, segments.length, 'klong'])
    for (let k = 0; k < s.config.points.x.length; k++) ok(s.config.points.x[k] >= 0 && s.config.points.x[k] <= s.config.frame.w && s.config.points.y[k] >= 0 && s.config.points.y[k] <= s.config.frame.h, 'inside the frame')
  }
  for (let i = 1; i < segments.length; i++) {
    const a = segments[i - 1], b = segments[i]
    const last = a.config.points.x.length - 1
    ok(near(a.rect.x + a.config.points.x[last], b.rect.x + b.config.points.x[0]) && near(a.rect.y + a.config.points.y[last], b.rect.y + b.config.points.y[0]), 'continuous boundary')
    equal(a.config.points.t[last], b.config.points.t[0], 'boundary time')
  }
  const total = segments.reduce((n, s) => n + s.config.points.x.length, 0)
  equal(total, points.length + segments.length - 1, 'no point dropped, boundaries shared')
  // what is read back is what was written
  const back = readInk(JSON.parse(JSON.stringify(segments[1].config)))
  ok(back, 'decodes')
  const world = worldPoints(back!, segments[1].rect, 0)
  const first = points.findIndex((q) => near(q.x, segments[1].rect.x + segments[1].config.points.x[0]))
  ok(near(world[0].x, points[first].x) && near(world[0].y, points[first].y), 'world position')
  ok(near(world[0].w, widthAt(PEN, points[first].p), 0.01), 'pressure width')
})

Deno.test('decode: malformed configs are refused', () => {
  const [seg] = encodeStroke([{ x: 0, y: 0, t: 0 }, { x: 10, y: 5, t: 9 }], MOUSE, 'kbad')
  const good = JSON.parse(JSON.stringify(seg.config))
  ok(readInk(good), 'good')
  for (const broken of [{ ...good, format: 2 }, { ...good, color: 'red' }, { ...good, seg: 1 }, { ...good, points: { ...good.points, y: [1] } }, { ...good, points: { ...good.points, p: [0.5] } }, { ...good, frame: { w: 0, h: 1 } }]) {
    equal(readInk(broken), null, JSON.stringify(broken).slice(0, 80))
  }
})

Deno.test('geometry: a resized and turned box moves the points with it; the eraser hits the line, not the box', () => {
  const [seg] = encodeStroke([{ x: 0, y: 0, t: 0 }, { x: 100, y: 100, t: 50 }], MOUSE, 'kdiag')
  const ink = readInk(seg.config)!
  const at = worldPoints(ink, seg.rect, 0)
  ok(near(at[0].x, 0) && near(at[1].y, 100), 'identity')
  const doubled = worldPoints(ink, { ...seg.rect, w: seg.rect.w * 2, h: seg.rect.h * 2 }, 0)
  ok(near(doubled[1].x - doubled[0].x, 200) && near(doubled[1].w, 4, 1e-9), 'scaled points and width')
  const turned = worldPoints(ink, seg.rect, 180)
  ok(near(turned[0].x, 100) && near(turned[0].y, 100), 'turned about the centre')
  // the segment between two far samples counts, a corner of its box does not
  ok(touches(at, { x: 60, y: 40 }, { x: 40, y: 60 }, 2), 'crossing between the samples')
  ok(!touches(at, { x: 90, y: 10 }, { x: 90, y: 10 }, 5), 'empty corner of the box')
  ok(distanceToInk(at, { x: 50, y: 50 }) === 0 && distanceToInk(at, { x: 90, y: 10 }) > 40, 'distance to the edge')
})

Deno.test('pieces: one per point, a dot for a tap; runs merge pieces of the same width', () => {
  const dot = pieces(widePoints([{ x: 5, y: 5, t: 0 }], MOUSE))
  equal(dot.length, 1)
  equal([dot[0].a, dot[0].b], [{ x: 5, y: 5, w: 2 }, { x: 5, y: 5, w: 2 }])
  const flat = pieces(widePoints(Array.from({ length: 20 }, (_, i) => ({ x: i * 3, y: Math.sin(i), t: i })), MOUSE))
  equal(runs(flat).length, 1, 'a stroke without pressure is one path')
  // handwriting: the pressure rises as the pen lands, holds, and falls as it lifts
  const word = pieces(widePoints(Array.from({ length: 240 }, (_, i) => ({ x: i, y: Math.sin(i / 5) * 8, t: i * 4, p: Math.min(0.55, i / 40, (239 - i) / 40) })), PEN))
  const paths = runs(word).length
  ok(paths < word.length / 4, `a pressure stroke is far fewer paths than points: ${paths} of ${word.length}`)
})

// ---- the input state machine

interface Log { strokes: { points: InkPoint[]; split: boolean }[]; drawn: InkPoint[]; predicted: InkPoint[]; dropped: number; erased: number; eraseEnds: boolean[]; captured: number[]; released: number[]; interrupts: number }

function harness(tool: 'pen' | 'eraser' = 'pen') {
  const log: Log = { strokes: [], drawn: [], predicted: [], dropped: 0, erased: 0, eraseEnds: [], captured: [], released: [], interrupts: 0 }
  const host: InkHost = {
    toWorld: (x, y) => ({ x, y }), zoom: () => 1, tool: () => tool, style: () => PEN,
    capture: (id) => log.captured.push(id), release: (id) => log.released.push(id), interrupt: () => { log.interrupts += 1 },
    begin: () => undefined, draw: (points, predicted) => { log.drawn.push(...points); log.predicted.push(...predicted) },
    stroke: (points, _style, _zoom, split) => log.strokes.push({ points: [...points], split }), dropped: () => { log.dropped += 1 },
    erase: () => { log.erased += 1 }, eraseEnd: (cancel) => log.eraseEnds.push(cancel),
  }
  return { input: new InkInput(host), log }
}
const ev = (id: number, x: number, y: number, t: number, extra: Partial<InkPointerEvent> = {}): InkPointerEvent =>
  ({ pointerId: id, pointerType: 'pen', clientX: x, clientY: y, pressure: 0.6, timeStamp: t, button: 0, buttons: 1, ...extra })

Deno.test('input: one pen owns the stroke; the up saves once with the pressure it had; the lost capture after it changes nothing', () => {
  const { input, log } = harness()
  ok(input.down(ev(1, 0, 0, 100)), 'starts')
  equal(log.interrupts, 1, 'running gestures stop first')
  input.move(ev(1, 10, 0, 108, { pressure: 0.7 }))
  ok(!input.down(ev(2, 50, 50, 109)), 'a second pen does not start or take over')
  input.move(ev(2, 60, 60, 110))
  input.up(ev(2, 60, 60, 111, { pressure: 0, buttons: 0 }))
  input.cancelPointer(2)
  input.lost(2)
  equal(log.strokes.length, 0, 'other pointers end nothing')
  input.up(ev(1, 20, 0, 116, { pressure: 0, buttons: 0 }))
  input.lost(1)
  equal(log.strokes.length, 1, 'saved once')
  const pts = log.strokes[0].points
  equal(pts.map((q) => [q.x, q.t, q.p]), [[0, 0, 0.6], [10, 8, 0.7], [20, 16, 0.7]], 'the up keeps the last pressure')
  equal([log.captured, log.released], [[1], [1]])
  ok(input.down(ev(1, 5, 5, 200)), 'the same pointer id writes the next stroke')
})

Deno.test('input: coalesced samples are used once; samples after the lift and predictions are never stored', () => {
  const { input, log } = harness()
  input.down(ev(7, 0, 0, 0))
  const samples = [ev(7, 1, 0, 2), ev(7, 2, 0, 4), ev(7, 3, 0, 6), ev(7, 3, 0, 7, { buttons: 0, pressure: 0 })]
  input.move({ ...ev(7, 3, 0, 6), getCoalescedEvents: () => samples, getPredictedEvents: () => [ev(7, 9, 0, 20)] })
  input.move(ev(7, 3, 0, 6))
  equal(log.predicted.map((q) => q.x), [9], 'drawn as a temporary tip')
  input.up(ev(7, 3, 0, 9, { buttons: 0, pressure: 0 }))
  equal(log.strokes[0].points.map((q) => q.x), [0, 1, 2, 3], 'no duplicate, no lifted sample, no prediction')
})

Deno.test('input: cancel drops the stroke; the mouse writes with its primary button only; a touch never starts one', () => {
  const { input, log } = harness()
  input.down(ev(1, 0, 0, 0))
  input.move(ev(1, 4, 4, 5))
  input.cancel()
  equal([log.strokes.length, log.dropped, log.released], [0, 1, [1]])
  ok(!input.down(ev(3, 0, 0, 0, { pointerType: 'mouse', button: 2 })), 'right button')
  ok(!input.down(ev(4, 0, 0, 0, { pointerType: 'touch' })), 'a finger')
  ok(input.down(ev(5, 0, 0, 0, { pointerType: 'mouse', pressure: 0.5 })), 'primary button')
  input.up(ev(5, 0, 0, 3, { pointerType: 'mouse', buttons: 0 }))
  equal(log.strokes[0].points, [{ x: 0, y: 0, t: 0 }], 'a click is a dot, without pressure')
})

Deno.test("input: a pen's eraser end erases; an erase is put back when cancelled", () => {
  const { input, log } = harness('pen')
  input.down(ev(1, 0, 0, 0, { button: 5, buttons: 32 }))
  input.move(ev(1, 10, 0, 5, { buttons: 32 }))
  input.up(ev(1, 20, 0, 9, { buttons: 0 }))
  equal([log.strokes.length, log.erased > 1, log.eraseEnds], [0, true, [false]])
  input.down(ev(1, 0, 0, 20, { button: 5, buttons: 32 }))
  input.cancelPointer(1)
  equal(log.eraseEnds, [false, true])
})

Deno.test('input: a stroke reaching its budget is saved and goes on from the same point', () => {
  const { input, log } = harness()
  input.down(ev(1, 0, 0, 0))
  let x = 0
  let t = 0
  while (log.strokes.length === 0) { x += 1.37; t += 1; input.move(ev(1, x, Math.sin(x) * 30, t, { pressure: 0.3 + (t % 50) / 100 })) }
  ok(log.strokes[0].split && rawBytes(log.strokes[0].points) > RAW_BUDGET * 0.99, 'saved at the budget')
  input.move(ev(1, x + 5, 0, t + 1))
  input.up(ev(1, x + 5, 0, t + 2, { buttons: 0 }))
  const [first, second] = log.strokes
  equal([second.split, second.points[0].x, second.points[0].y, second.points[0].t], [false, first.points.at(-1)!.x, first.points.at(-1)!.y, 0], 'continues where it was cut')
})

// ---- the fixture the core replays (straight line, tap, pressure, long stroke)

Deno.test('fixture: the core commits these strokes and reads them back unchanged', async () => {
  const surface = [
    { op: 'entity.create', entity_id: 'ink-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'h', payload: { kind: 'folder', system: 'surface_content', surface_id: 'ink', title: 'Ink' } },
    { op: 'entity.create', entity_id: 'ink', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'h', name: 'ink', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Ink', content_folder_id: 'ink-content' } },
  ]
  const strokes: [string, InkPoint[], InkStyle][] = [
    ['line', Array.from({ length: 40 }, (_, i) => ({ x: 20 + i * 5, y: 30, t: i * 8 })), MOUSE],
    ['tap', [{ x: 260.5, y: 41.25, t: 0, p: 0.62 }], PEN],
    ['pressure', Array.from({ length: 60 }, (_, i) => ({ x: 40 + i * 4, y: 120 + Math.sin(i / 6) * 25, t: i * 5, p: Math.round((0.1 + 0.85 * Math.sin((i / 59) * Math.PI)) * 1000) / 1000, tx: 30, ty: -10 })), PEN],
    ['long', scribble(6000), PEN],
  ]
  const commits = [{ message: 'an ink canvas', operations: surface }]
  for (const [name, points, style] of strokes) {
    const segments = encodeStroke(simplify(points, 0.25, style), style, `k${name}`)
    commits.push({
      message: name,
      operations: segments.map((s, i) => ({ op: 'entity.create', entity_id: `c-${name}-${i}`, type_id: 'buckyos.cell', parent_id: 'ink', order_key: `${'mnpq'[strokes.findIndex((x) => x[0] === name)]}${i + 1}`, placement: s.rect, payload: { view: { type: 'ink', version: 1 }, config: s.config } })) as typeof surface,
    })
  }
  ok(commits.at(-1)!.operations.length > 1, 'the long stroke has segments')
  const description = 'Pen ink (notepads/aiworkspace-handwriting-todo.md D0): a straight mouse line, a pen tap, a pressure stroke with tilt and a long stroke in several segments, as tests/datamodel/aiworkspace-ink.test.ts encodes them; core tests/ink.rs commits and reads them back.'
  const text = `{"description":${JSON.stringify(description)},"commits":[\n${commits.map((c) => JSON.stringify(c)).join(',\n')}\n]}\n`
  const path = new URL('../../../aiworkspace/fixtures/ink/strokes.json', import.meta.url)
  if (Deno.env.get('AIWS_REGEN') === '1') {
    await Deno.mkdir(new URL('.', path), { recursive: true })
    await Deno.writeTextFile(path, text)
  }
  equal(await Deno.readTextFile(path), text, 'fixture (AIWS_REGEN=1 rewrites it)')
})
