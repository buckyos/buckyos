/* DrawingML geometry (Office 导入 TODO O3): every preset evaluates to finite SVG path data at several sizes, the
 * formulas and arcs match hand-computed values, mirroring maps the points, custom geometry scales its own space.
 * Run: `deno test -A`. */

import { parseXml } from '../import-drawio/xml.ts'
import { customDef, evaluate, presetNames, presetPaths } from './mod.ts'
import { assert, equal } from './test_util.ts'

Deno.test('all 187 presets evaluate to finite paths', () => {
  const names = presetNames()
  equal(names.length, 187)
  for (const name of names) {
    for (const [w, h] of [[200, 100], [100, 200], [50, 50], [1, 300]]) {
      const paths = presetPaths(name, w, h, {})
      assert(paths && paths.length > 0, `${name}: no paths`)
      for (const p of paths) assert(/^M/.test(p.d) && !/NaN|Infinity/.test(p.d), `${name} ${w}x${h}: ${p.d.slice(0, 80)}`)
    }
  }
})

Deno.test('formulas, adjust values and arcs', () => {
  equal(presetPaths('rect', 200, 100, {})![0].d, 'M0 0L200 0L200 100L0 100Z')
  equal(presetPaths('rightArrow', 200, 100, {})![0].d, 'M0 25L150 25L150 0L200 50L150 100L150 75L0 75Z')
  // a quarter of the ellipse is cut out: from 0° clockwise to 270°
  equal(presetPaths('pie', 200, 100, {})![0].d, 'M200 50A100 50 0 1 1 100 0L100 50Z')
  equal(presetPaths('roundRect', 200, 100, { adj: 'val 50000' })![0].d.slice(0, 22), 'M0 50A50 50 0 0 1 50 0')
  const arc = presetPaths('arc', 200, 100, {})!
  equal(arc.map((p) => [p.fill, p.stroke]), [['norm', false], ['none', true]], 'fill-only and stroke-only paths')
  const disk = presetPaths('flowChartMagneticDisk', 100, 100, {})!
  equal(disk.length, 3)
})

Deno.test('flips mirror the points and the arc direction', () => {
  equal(presetPaths('rightArrow', 200, 100, {}, true)![0].d, 'M200 25L50 25L50 0L0 50L50 100L50 75L200 75Z')
  const pie = presetPaths('pie', 200, 100, {}, false, true)![0].d
  equal(pie, 'M200 50A100 50 0 1 0 100 100L100 50Z')
})

Deno.test('custom geometry: its own path space scaled into the box', () => {
  const geom = parseXml('<a:custGeom xmlns:a="a"><a:avLst/><a:gdLst/><a:pathLst><a:path w="10" h="10"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="10" y="5"/></a:lnTo><a:lnTo><a:pt x="0" y="10"/></a:lnTo><a:close/></a:path></a:pathLst></a:custGeom>')
  equal(evaluate(customDef(geom), 200, 100)[0].d, 'M0 0L200 50L0 100Z')
})
