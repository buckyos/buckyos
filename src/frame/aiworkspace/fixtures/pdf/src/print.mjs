/* Chromium-printed fixtures (`node print.mjs` from this directory, with the Desktop's Playwright installed):
 * `slides.pdf`, two 16:9 slides (a coloured background, a heading, a bulleted list with CJK, a box with a border, a
 * gradient, a picture, an SVG drawing, a wrapped paragraph), and `doc.pdf`, an A4 page set like a word processor's
 * (headings, paragraphs, a CJK paragraph, a table, inline code and italics). Both are tagged (structure tree). */

import { chromium } from '../../../../desktop/node_modules/@playwright/test/index.mjs'
import { writeFileSync } from 'node:fs'
const out = new URL('../', import.meta.url).pathname
const png = 'iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAGklEQVR4nGNk+M/wn4GBgYGJgYGBgYGBgQEAJgQDAZx6s4kAAAAASUVORK5CYII='
const slides = `<!doctype html><html><head><meta charset="utf-8"><style>
@page { size: 960px 540px; margin: 0 }
body { margin: 0; font-family: Arial, sans-serif }
.s { width: 960px; height: 540px; position: relative; overflow: hidden; page-break-after: always; background: #1e3a5f; color: #fff }
.s2 { background: #ffffff; color: #222 }
h1 { position: absolute; left: 60px; top: 40px; margin: 0; font-size: 44px }
ul { position: absolute; left: 60px; top: 140px; margin: 0; font-size: 24px; line-height: 1.5 }
.box { position: absolute; left: 600px; top: 160px; width: 240px; height: 140px; background: #f59e0b; border: 3px solid #b45309 }
.grad { position: absolute; left: 600px; top: 330px; width: 240px; height: 80px; background: linear-gradient(90deg, #22c55e, #3b82f6) }
img { position: absolute; left: 60px; top: 380px; width: 120px; height: 120px }
.line { position:absolute; left: 60px; top: 120px; width: 840px; height: 0; border-top: 2px solid #94a3b8 }
p.foot { position: absolute; left: 60px; bottom: 20px; margin: 0; font-size: 14px; font-style: italic }
</style></head><body>
<div class="s"><h1>季度回顾 Quarterly Review</h1><div class="line"></div><ul><li>收入增长 <b>23%</b></li><li>New customers: 1,204</li><li>Churn down to 2.1%</li></ul><div class="box"></div><div class="grad"></div><img src="data:image/png;base64,${png}"><p class="foot">Confidential — internal use</p></div>
<div class="s s2"><h1>Roadmap</h1><svg style="position:absolute;left:100px;top:150px" width="400" height="300" viewBox="0 0 400 300"><circle cx="100" cy="100" r="60" fill="#ef4444"/><path d="M200 250 C 250 150, 350 150, 380 250 Z" fill="#8b5cf6" stroke="#4c1d95" stroke-width="4"/><polyline points="20,280 80,200 140,240 200,180" fill="none" stroke="#0ea5e9" stroke-width="3"/></svg><p style="position:absolute;left:560px;top:180px;width:340px;font-size:20px;line-height:1.4;margin:0">This paragraph wraps over several lines so that the importer has to group the lines into one text object.</p></div>
</body></html>`
const doc = `<!doctype html><html><head><meta charset="utf-8"><style>
@page { size: A4; margin: 20mm }
body { font-family: 'Times New Roman', serif; font-size: 12pt; line-height: 1.3 }
h2 { font-family: Arial, sans-serif }
table { border-collapse: collapse } td { border: 1px solid #333; padding: 4px 8px }
code { font-family: monospace }
</style></head><body>
<h2>1. Introduction</h2>
<p>PDF import turns each page into editable objects. This paragraph is long enough to wrap onto at least two or three lines in the printed document so that paragraph grouping can be tested properly.</p>
<p>中文段落：导入后每一页都是一个页面组，文字可以双击编辑，图片是独立对象。</p>
<table><tr><td>Name</td><td>Value</td></tr><tr><td>Pages</td><td>3</td></tr></table>
<p>Inline <code>code()</code> and <i>italic</i> words.</p>
</body></html>`
const browser = await chromium.launch()
const page = await browser.newPage()
for (const [name, html, opts] of [['slides', slides, { width: '960px', height: '540px', printBackground: true }], ['doc', doc, { format: 'A4', printBackground: true }]]) {
  await page.setContent(html)
  const pdf = await page.pdf({ ...opts, tagged: true, outline: true })
  writeFileSync(`${out}${name}.pdf`, pdf)
  console.log(name, pdf.length)
}
await browser.close()
