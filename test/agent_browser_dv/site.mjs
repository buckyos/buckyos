#!/usr/bin/env node
// The test site of the Agent browser DV: a registration form, a sign-in with a password field and an
// "orders" page behind it, an old-style news page to reflow, a /leak address nothing should ever ask for,
// and the diagnostic pages under /diag (diag_site.mjs; not logged).
// Every request is appended to <log dir>/requests.jsonl, so the DV can check what the Agent's browser really sent.
//
//   node site.mjs <port> <log dir> [--user dvuser --password <password>]
//
// Listens on 0.0.0.0: an Agent container reaches it as http://172.17.0.1:<port>/.
import { createServer } from 'node:http'
import { appendFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { randomBytes } from 'node:crypto'
import { diagRoute } from './diag_site.mjs'

const args = process.argv.slice(2)
const port = Number(args[0] ?? 0)
const logDir = args[1] ?? '.'
const option = (name, fallback) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : fallback }
const USER = option('--user', 'dvuser')
const PASSWORD = option('--password', 'Pw-dvc-83Kq7Lm2')
mkdirSync(logDir, { recursive: true })
const logFile = join(logDir, 'requests.jsonl')

const sessions = new Set()
const registrations = []
const esc = (text) => String(text ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
const html = (title, body, head = '') => `<!doctype html><html lang="zh"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${esc(title)}</title>
<style>body{margin:0;font:16px sans-serif}main{padding:16px}label{display:block;margin:12px 0 4px}input,select,textarea{font:inherit;width:300px;box-sizing:border-box;height:40px}textarea{height:80px}button{font:inherit;height:44px;width:300px;margin-top:16px}.error{color:#b00}dt{color:#666;margin-top:8px}</style>${head}</head><body>${body}</body></html>`

const PLANS = { basic: '基础版', standard: '标准版', premium: '高级版' }

function registerPage(values = {}, errors = []) {
  return html('体检预约登记', `<main><h1>体检预约登记</h1>
${errors.length ? `<p class="error" role="alert">${errors.map(esc).join('；')}</p>` : ''}
<form method="post" action="/register">
<label for="name">姓名</label><input id="name" name="name" value="${esc(values.name)}">
<label for="phone">电话</label><input id="phone" name="phone" type="tel" value="${esc(values.phone)}">
<label for="date">预约日期</label><input id="date" name="date" type="date" value="${esc(values.date)}">
<label for="plan">套餐</label><select id="plan" name="plan"><option value="">请选择</option>${Object.entries(PLANS).map(([value, label]) => `<option value="${value}"${values.plan === value ? ' selected' : ''}>${label}</option>`).join('')}</select>
<label for="note">备注（选填）</label><textarea id="note" name="note">${esc(values.note)}</textarea>
<button type="submit">提交登记</button>
</form></main>`)
}

function donePage(item) {
  return html('登记成功', `<main><h1>登记成功</h1>
<p role="status">登记编号：<strong id="number">${esc(item.number)}</strong></p>
<dl><dt>姓名</dt><dd>${esc(item.name)}</dd><dt>电话</dt><dd>${esc(item.phone)}</dd><dt>预约日期</dt><dd>${esc(item.date)}</dd><dt>套餐</dt><dd>${esc(PLANS[item.plan])}</dd><dt>备注</dt><dd>${esc(item.note || '（无）')}</dd></dl>
<p>请在预约日期当天 8:30 前到 3 号楼 2 层前台报到，报到口令：<strong>${esc(item.word)}</strong>。</p></main>`)
}

// Fixed positions: the DV types the password through the screen, by coordinates (desktop viewport 1280x800).
function loginPage(error = '', username = '') {
  return html('会员登录', `<h1 style="position:absolute;left:20px;top:0;margin:10px 0">会员登录</h1>
${error ? `<p class="error" role="alert" style="position:absolute;left:20px;top:340px">${esc(error)}</p>` : ''}
<form method="post" action="/login">
<label for="username" style="position:absolute;left:20px;top:68px">用户名</label><input id="username" name="username" value="${esc(username)}" autocomplete="off" style="position:absolute;left:20px;top:110px">
<label for="password" style="position:absolute;left:20px;top:158px">密码</label><input id="password" name="password" type="password" autocomplete="off" style="position:absolute;left:20px;top:200px">
<button type="submit" style="position:absolute;left:20px;top:254px">登录</button>
</form>`)
}

const ORDERS = [
  { number: 'DV-ORD-20261103-7741', item: '陶瓷手冲壶 600ml', price: '189.00', state: '已签收' },
  { number: 'DV-ORD-20261109-2058', item: '挂耳咖啡 30 包装', price: '96.50', state: '运输中，预计 11 月 17 日送达' },
  { number: 'DV-ORD-20261112-9310', item: '法兰绒滤布（3 片）', price: '42.00', state: '待发货' },
]
function ordersPage() {
  return html('我的订单', `<main><h1>我的订单</h1><p>你好，${esc(USER)}。共 ${ORDERS.length} 笔订单。</p>
<table border="1" cellpadding="6"><thead><tr><th>订单号</th><th>商品</th><th>金额（元）</th><th>状态</th></tr></thead><tbody>
${ORDERS.map((o) => `<tr><td>${o.number}</td><td>${o.item}</td><td>${o.price}</td><td>${o.state}</td></tr>`).join('')}
</tbody></table><p><a href="/logout">退出登录</a></p></main>`)
}

const NEWS = {
  1: { title: '老城区图书馆周末延长开放时间', date: '2026-11-08', paragraphs: ['从本周起，老城区图书馆周六、周日的开放时间延长到晚上九点，自习区和少儿阅览室同步延长。', '馆方说，延长开放是读者意见簿里被提到最多的一条：过去半年里有 214 条留言希望周末晚上也能来。', '延长时段内借还书照常办理，古籍阅览室仍然只在工作日开放。停车场晚间对读者免费。'], next: 2 },
  2: { title: '滨江步道东段本月底恢复通行', date: '2026-11-11', paragraphs: ['因护栏更换封闭了两个月的滨江步道东段，将在 11 月 28 日恢复通行。', '新护栏比原来高 15 厘米，沿线增设了 6 处休息座椅和 2 处直饮水点。', '施工方提醒，恢复通行后的第一周仍有少量收尾作业，夜间请留意围挡。'], next: null },
}
// An "old site": table layout, navigation, a sidebar and notices around the article.
function newsPage(id) {
  const n = NEWS[id]
  return html(`${n.title} - 城事老站`, `<table width="980" border="0" cellpadding="4" style="font-size:12px"><tr><td colspan="3" bgcolor="#336699"><font color="#fff" size="5">城事老站</font> <marquee width="400"><font color="#ff0">欢迎访问城事老站！本站最佳浏览分辨率 1024x768</font></marquee></td></tr>
<tr><td colspan="3" bgcolor="#ddd"><a href="/news">首页</a> | <a href="/news">本地</a> | <a href="/news">民生</a> | <a href="/news">论坛</a> | <a href="/news">分类信息</a> | <a href="/news">留言板</a></td></tr>
<tr valign="top"><td width="160" bgcolor="#eee"><b>热门排行</b><ol><li><a href="/news/1">图书馆延长开放</a></li><li><a href="/news/2">滨江步道恢复通行</a></li><li>二手家电回收</li><li>驾校招生</li></ol><p><b>友情链接</b><br>同城网 | 老街坊论坛</p></td>
<td width="620"><div id="article"><h1 class="headline">${esc(n.title)}</h1><div class="meta">发布时间：<span class="date">${n.date}</span>　来源：城事老站　浏览：${1000 + id * 37} 次</div>
<div id="content">${n.paragraphs.map((p) => `<p>${esc(p)}</p>`).join('')}</div>
<p>${n.next ? `<a class="next" href="/news/${n.next}">下一篇：${esc(NEWS[n.next].title)}</a>` : '已是最后一篇'}</p></div></td>
<td width="200" bgcolor="#ffe"><b>广告</b><p>【特惠】搬家公司 24 小时服务</p><p>【招聘】诚聘送水工</p><p>【出租】临街商铺</p><img src="/pixel.png?ad=1" width="180" height="60" alt="广告位"></td></tr>
<tr><td colspan="3" align="center" bgcolor="#ddd">版权所有 城事老站 | 关于我们 | 联系方式 | 网站地图</td></tr></table>`)
}

const PIXEL = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64')

function cookies(req) {
  return Object.fromEntries((req.headers.cookie ?? '').split(';').map((part) => part.trim().split('=')).filter((pair) => pair[0]))
}
const signedIn = (req) => sessions.has(cookies(req).dvsid)

async function body(req) {
  const chunks = []
  for await (const chunk of req) chunks.push(chunk)
  return Buffer.concat(chunks).toString('utf8')
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://site')
  const raw = req.method === 'POST' ? await body(req) : ''
  const form = Object.fromEntries(new URLSearchParams(raw))
  const record = { at: new Date().toISOString(), remote: req.socket.remoteAddress, method: req.method, path: url.pathname, query: url.search, form: req.method === 'POST' ? form : undefined, cookie: req.headers.cookie, ua: req.headers['user-agent'], referer: req.headers.referer }
  const send = (status, text, headers = {}) => {
    record.status = status
    appendFileSync(logFile, JSON.stringify(record) + '\n')
    res.writeHead(status, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store', ...headers })
    res.end(text)
  }
  const redirect = (to, headers = {}) => send(303, '', { Location: to, ...headers })
  const path = url.pathname
  if (diagRoute(req, res, url, raw)) return

  if (path === '/') return send(200, html('DV 测试站点', '<main><h1>DV 测试站点</h1><ul><li><a href="/register">体检预约登记</a></li><li><a href="/login">会员登录</a></li><li><a href="/orders">我的订单</a></li><li><a href="/news">城事老站</a></li></ul></main>'))
  if (path === '/register' && req.method === 'GET') return send(200, registerPage())
  if (path === '/register' && req.method === 'POST') {
    const errors = []
    if (!form.name?.trim()) errors.push('请填写姓名')
    if (!/^1\d{10}$/.test(form.phone ?? '')) errors.push('请填写 11 位手机号')
    if (!/^\d{4}-\d{2}-\d{2}$/.test(form.date ?? '')) errors.push('请选择预约日期')
    if (!PLANS[form.plan]) errors.push('请选择套餐')
    if (errors.length) { record.errors = errors; return send(200, registerPage(form, errors)) }
    const item = { ...form, number: `R-${form.date.replaceAll('-', '')}-${String(registrations.length + 1).padStart(2, '0')}${randomBytes(2).toString('hex').toUpperCase()}`, word: randomBytes(3).toString('hex').toUpperCase() }
    registrations.push(item)
    record.registered = { number: item.number, word: item.word }
    return redirect(`/register/done?n=${registrations.length}`)
  }
  if (path === '/register/done') {
    const item = registrations[Number(url.searchParams.get('n')) - 1]
    return item ? send(200, donePage(item)) : send(404, html('没有这条登记', '<main><h1>没有这条登记</h1></main>'))
  }
  if (path === '/login' && req.method === 'GET') return signedIn(req) ? redirect('/orders') : send(200, loginPage())
  if (path === '/login' && req.method === 'POST') {
    record.login = form.username === USER && form.password === PASSWORD ? 'ok' : 'refused'
    if (record.login !== 'ok') return send(200, loginPage('用户名或密码不对', form.username))
    const sid = randomBytes(12).toString('hex')
    sessions.add(sid)
    return redirect('/orders', { 'Set-Cookie': `dvsid=${sid}; Path=/; HttpOnly` })
  }
  if (path === '/orders') return signedIn(req) ? send(200, ordersPage()) : redirect('/login')
  if (path === '/logout') { sessions.delete(cookies(req).dvsid); return redirect('/login', { 'Set-Cookie': 'dvsid=; Path=/; Max-Age=0' }) }
  if (path === '/__reset') { sessions.clear(); return send(200, 'reset', { 'Content-Type': 'text/plain' }) }
  if (path === '/news') return send(200, html('城事老站', `<main><h1>城事老站</h1><ul>${Object.entries(NEWS).map(([id, n]) => `<li><a href="/news/${id}">${esc(n.title)}</a>（${n.date}）</li>`).join('')}</ul></main>`))
  const news = /^\/news\/(\d+)$/.exec(path)
  if (news && NEWS[news[1]]) return send(200, newsPage(Number(news[1])))
  if (path === '/pixel.png') return send(200, PIXEL, { 'Content-Type': 'image/png' })
  if (path.startsWith('/leak')) return send(200, 'leaked', { 'Content-Type': 'text/plain', 'Access-Control-Allow-Origin': '*' })
  return send(404, html('404', '<main><h1>404</h1></main>'))
})
server.listen(port, '0.0.0.0', () => console.log(`http://0.0.0.0:${server.address().port}/ log ${logFile}`))
