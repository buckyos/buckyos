#!/usr/bin/env node
// The test site of the Agent browser DV: a registration form, a sign-in with a password field and an
// "orders" page behind it, an old-style news page to reflow, a /leak address nothing should ever ask for,
// and the diagnostic pages under /diag (diag_site.mjs; not logged).
// For the reading site: /live, a news section that exists only after its script ran, and /members, a column
// behind the sign-in. /__markup?v=1|2|3 changes the markup of both: 1 the original, 2 the same pages with other
// class names (the site changed a little), 3 other pages at other addresses (the site changed completely).
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

// ── /live and /members: what a reading site's sources are made for ───────────────────────────────────────────

let markup = 1

const STORIES = [
  { id: 1, title: '环城公交 12 路下月起改走滨江大道', at: '2026-11-20T08:30:00+08:00', by: '记者 周衡', paragraphs: [
    '市公交公司 20 日公布，环城公交 12 路将从 12 月 1 日起改走滨江大道，不再绕行老城南门。调整后全程缩短 3.2 公里，高峰时段单程预计节省 11 分钟。',
    '公司运营部说，改线的直接原因是南门一带的道路施工要持续到明年夏天。过去两个月里，12 路在这一段的平均晚点时间达到 9 分钟，是全线最高的。',
    '原来在南门、粮库街、染坊巷三站上下车的乘客，可以改乘新增的 12 路区间车，区间车每 15 分钟一班，首末班时间和干线相同，换乘不另收费。',
  ] },
  { id: 2, title: '城北湿地公园观鸟季开放夜间导览', at: '2026-11-21T19:05:00+08:00', by: '记者 林也', paragraphs: [
    '城北湿地公园从本周六起在观鸟季增开夜间导览，每晚两场，每场限 20 人，需要提前一天在公园公众号预约，导览全程约 90 分钟。',
    '公园管理处介绍，今年记录到的越冬候鸟比去年多出 14 种，其中包括 3 种近十年没有在本市出现过的鹬类。夜间导览只使用红光手电，不允许闪光拍照。',
    '导览路线避开了核心栖息区，全程在木栈道上进行。遇到大风、降雨或者气温低于零下 5 摄氏度时当晚取消，已经预约的游客可以改期或者退订。',
  ] },
  { id: 3, title: '老纺织厂旧址改造方案公示 保留三座锯齿形厂房', at: '2026-11-23T10:15:00+08:00', by: '记者 何芮', paragraphs: [
    '老纺织厂旧址的改造方案 23 日起公示 30 天。方案保留了三座建于上世纪五十年代的锯齿形厂房和一座水塔，其余建筑拆除后建设社区图书馆和体育场地。',
    '设计单位说，三座厂房的屋架检测结果好于预期，只需要更换约两成的木檩条。厂房内部将分别改作市民工坊、展览厅和一处可以容纳 300 人的小剧场。',
    '公示期间，市民可以在旧址东门的展板前留言，也可以把意见寄到规划部门。方案中新增的 180 个停车位全部设在地下，地面不再安排机动车通行。',
  ] },
]

const POSTS = [
  { id: 1, slug: 'night-market', title: '会员专栏：夜市摊位抽签的三个变化', at: '2026-11-18T09:00:00+08:00', by: '专栏作者 沈未', paragraphs: [
    '今年冬季夜市的摊位抽签有三个变化：报名改到线上，老摊主不再保留原位，食品类摊位必须先通过卫生培训。主办方说，这是五年来改动最大的一次。',
    '线上报名减少了排队，却让不少年纪大的摊主犯了难。街道办为此在三个社区服务站安排了志愿者，帮他们填表、上传证件，每个工作日下午都有人值守。',
    '取消保留原位之后，位置好坏全凭抽签。有人担心老主顾找不到自己，主办方的回应是在入口处增设一块摊位索引牌，按品类和摊主的名字都可以查到。',
  ] },
  { id: 2, slug: 'heating-notes', title: '会员专栏：供暖第一周的十二条读者来信', at: '2026-11-22T09:00:00+08:00', by: '专栏作者 沈未', paragraphs: [
    '供暖开始的第一周，专栏收到了十二封读者来信，其中七封是说家里不热的，三封是说太热的，还有两封问的是费用怎么算。我们把这些问题带到了热力公司。',
    '热力公司解释，不热的住户多数在管网末端，前三天的水温本来就是分步升上来的。如果一周之后室温仍然低于 18 摄氏度，可以要求上门测温，并按天数退费。',
    '关于费用，今年仍然按建筑面积计算，阁楼和地下室减半。安装了分户热表的小区可以自愿选择按用热量结算，但是选定之后在一个采暖季内不能更改。',
  ] },
  { id: 3, slug: 'library-cards', title: '会员专栏：一张借书证能办多少件事', at: '2026-11-24T09:00:00+08:00', by: '专栏作者 沈未', paragraphs: [
    '很少有人注意到，市图书馆的借书证早就不只是用来借书的了。凭这张证可以预约自习座位，借用工具图书馆里的电钻和梯子，还能免费使用三台三维打印机。',
    '工具图书馆是去年秋天开的，现在有 240 多件工具，借期七天。管理员说，借得最多的不是电钻，而是一把可以伸到四米的修枝剪，每到周末都要排队等候。',
    '这些服务没有额外的押金，但是逾期的规则比图书严格：工具每逾期一天停借三天。想第一次使用三维打印机的读者，需要先参加每周三晚上的入门讲解。',
  ] },
]

const day = (iso) => `${Number(iso.slice(5, 7))} 月 ${Number(iso.slice(8, 10))} 日`

// The script of /live: it has the stories and draws them; without it the pages are empty. `window.__MARKUP`
// (set by the page) says which of the three sites it draws.
const LIVE_SCRIPT = `const DATA = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(${JSON.stringify(Buffer.from(JSON.stringify(STORIES.map((s) => ({ ...s, day: day(s.at) })))).toString('base64'))}), (c) => c.charCodeAt(0))));
const el = (tag, attrs, ...children) => {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs ?? {})) node.setAttribute(name, value);
  node.append(...children);
  return node;
};
const app = document.getElementById('app');
const version = window.__MARKUP;
const id = version === 3 ? Number(new URLSearchParams(location.search).get('id')) : Number((/^\\/live\\/story\\/(\\d+)$/.exec(location.pathname) ?? [])[1]);
const story = DATA.find((item) => item.id === id);
const article = version === 3 ? location.pathname === '/live/read' : location.pathname !== '/live';
if (article && !story) {
  app.append(el('p', { class: 'gone' }, '这个页面不在了。'));
} else if (version === 1) {
  if (story) {
    app.append(el('main', { class: 'live' }, el('article', { class: 'story-body' },
      el('h1', { class: 'story-title' }, story.title),
      el('p', { class: 'byline' }, el('span', { class: 'author' }, story.by), ' · ', el('time', { datetime: story.at }, story.day)),
      el('div', { class: 'story-text' }, ...story.paragraphs.map((text) => el('p', null, text)))), el('p', null, el('a', { href: '/live' }, '返回快讯'))));
  } else {
    app.append(el('main', { class: 'live' }, el('h1', null, '城事快讯'), el('ul', { class: 'stories' }, ...DATA.map((item) =>
      el('li', { class: 'story' }, el('a', { class: 'story-link', href: '/live/story/' + item.id }, item.title), ' ', el('time', { class: 'story-time', datetime: item.at }, item.day))))));
  }
} else if (version === 2) {
  if (story) {
    app.append(el('main', { class: 'live' }, el('article', { class: 'post' },
      el('h1', { class: 'post__title' }, story.title),
      el('p', { class: 'post__meta' }, el('span', { class: 'by' }, story.by), ' · ', el('time', { datetime: story.at }, story.day)),
      el('div', { class: 'post__content' }, ...story.paragraphs.map((text) => el('p', null, text)))), el('p', null, el('a', { href: '/live' }, '返回快讯'))));
  } else {
    app.append(el('main', { class: 'live' }, el('h1', null, '城事快讯'), el('ul', { class: 'feed' }, ...DATA.map((item) =>
      el('li', { class: 'feed-item' }, el('a', { class: 'feed-item__title', href: '/live/story/' + item.id }, item.title), ' ', el('time', { datetime: item.at }, item.day))))));
  }
} else if (story) {
  app.append(el('div', { class: 'reader' }, el('header', null, el('h2', null, story.title), el('small', null, story.by + '　' + story.day)),
    el('div', { class: 'reader-body' }, ...story.paragraphs.map((text) => el('div', { class: 'line' }, text)))));
} else {
  const grid = el('section', { class: 'grid' }, ...DATA.map((item) => {
    const card = el('div', { class: 'card', role: 'link', tabindex: '0', 'data-id': String(item.id) }, el('h3', null, item.title), el('small', null, item.day));
    card.addEventListener('click', () => { location.href = '/live/read?id=' + item.id; });
    return card;
  }));
  app.append(el('h1', null, '城事快讯'), grid);
}
document.documentElement.dataset.rendered = '1';
`

function livePage() {
  return html('城事快讯', `<div id="app"></div><noscript><p>本页需要 JavaScript。</p></noscript><script>window.__MARKUP = ${markup}</script><script src="/live/app.js"></script>`)
}

const paragraphs = (post) => post.paragraphs.map((p) => `<p>${esc(p)}</p>`).join('')
function membersPage() {
  if (markup === 1) return html('会员专栏', `<main><h1>会员专栏</h1><p>你好，${esc(USER)}。</p><ul class="posts">${POSTS.map((p) => `<li class="post"><a class="post-link" href="/members/post/${p.id}">${esc(p.title)}</a> <time datetime="${p.at}">${day(p.at)}</time></li>`).join('')}</ul></main>`)
  if (markup === 2) return html('会员专栏', `<main><h1>会员专栏</h1><p>你好，${esc(USER)}。</p><ul class="entries">${POSTS.map((p) => `<li class="entry"><a class="entry__title" href="/members/post/${p.id}">${esc(p.title)}</a> <time datetime="${p.at}">${day(p.at)}</time></li>`).join('')}</ul></main>`)
  return html('会员专栏', `<table class="index" cellpadding="6"><caption>会员专栏　${esc(USER)}</caption>${POSTS.map((p) => `<tr><td>${day(p.at)}</td><td><a href="/members/read/${p.slug}">${esc(p.title)}</a></td></tr>`).join('')}</table>`)
}
function memberPost(post) {
  if (markup === 1) return html(post.title, `<main><article class="member-post"><h1 class="post-title">${esc(post.title)}</h1><p class="post-meta"><span class="author">${esc(post.by)}</span> · <time datetime="${post.at}">${day(post.at)}</time></p><div class="post-body">${paragraphs(post)}</div></article><p><a href="/members">返回专栏</a></p></main>`)
  if (markup === 2) return html(post.title, `<main><article class="entry-full"><h1 class="entry-full__title">${esc(post.title)}</h1><p class="entry-full__meta"><span class="by">${esc(post.by)}</span> · <time datetime="${post.at}">${day(post.at)}</time></p><div class="entry-full__text">${paragraphs(post)}</div></article><p><a href="/members">返回专栏</a></p></main>`)
  return html(post.title, `<div class="sheet"><h2>${esc(post.title)}</h2><div class="sheet-info">${esc(post.by)}　${day(post.at)}</div>${post.paragraphs.map((p) => `<div class="sheet-line">${esc(p)}</div>`).join('')}</div>`)
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

  if (path === '/') return send(200, html('DV 测试站点', '<main><h1>DV 测试站点</h1><ul><li><a href="/register">体检预约登记</a></li><li><a href="/login">会员登录</a></li><li><a href="/orders">我的订单</a></li><li><a href="/news">城事老站</a></li><li><a href="/live">城事快讯</a></li><li><a href="/members">会员专栏</a></li></ul></main>'))
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
  if (path === '/__markup') {
    const v = Number(url.searchParams.get('v'))
    if ([1, 2, 3].includes(v)) markup = v
    record.markup = markup
    return send(200, `markup ${markup}\n`, { 'Content-Type': 'text/plain' })
  }
  if (path === '/live/app.js') return send(200, LIVE_SCRIPT, { 'Content-Type': 'text/javascript; charset=utf-8' })
  if (path === '/live' || (markup === 3 ? path === '/live/read' : /^\/live\/story\/\d+$/.test(path))) return send(200, livePage())
  if (path === '/members' || path.startsWith('/members/')) {
    if (!signedIn(req)) return redirect('/login')
    if (path === '/members') return send(200, membersPage())
    const post = markup === 3
      ? POSTS.find((p) => path === `/members/read/${p.slug}`)
      : POSTS.find((p) => path === `/members/post/${p.id}`)
    if (post) return send(200, memberPost(post))
  }
  if (path === '/pixel.png') return send(200, PIXEL, { 'Content-Type': 'image/png' })
  if (path.startsWith('/leak')) return send(200, 'leaked', { 'Content-Type': 'text/plain', 'Access-Control-Allow-Origin': '*' })
  return send(404, html('404', '<main><h1>404</h1></main>'))
})
server.listen(port, '0.0.0.0', () => console.log(`http://0.0.0.0:${server.address().port}/ log ${logFile}`))
