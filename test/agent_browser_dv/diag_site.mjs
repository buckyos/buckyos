// The diagnostic pages of the test site (BC1 of notepads/agent-runtime-browser-compatibility-todo.md):
// a page reports what the browser shows of itself to the site, from the page, so a browser nobody is
// connected to can be sampled. Nothing is read over the DevTools protocol here.
//
//   GET  /diag?run=<id>                 the top page: probes, embeds a frame of another origin, offers a popup,
//                                       navigates once, and probes again whenever the run's phase changes
//   GET  /diag/frame?run=<id>           the same probe in a cross-origin iframe
//   GET  /diag/popup?run=<id>           the same probe in a window the page opened (on a click: a user gesture)
//   POST /diag/report?run=<id>          a probe's result (JSON); kept with the request headers the page was loaded with
//   GET  /diag/phase?run=<id>           the run's phase (the runner sets it with POST /diag/phase?run=&phase=)
//   GET  /diag/results?run=<id>         every report of the run
//
// The probe never sends anything of a real account: the pages are the site's own.

const runs = new Map()
const run = (id) => {
  if (!runs.has(id)) runs.set(id, { phase: 'load', reports: [], headers: {} })
  return runs.get(id)
}

const HEADERS = ['user-agent', 'accept-language', 'sec-ch-ua', 'sec-ch-ua-mobile', 'sec-ch-ua-platform', 'sec-ch-ua-platform-version',
  'sec-ch-ua-full-version-list', 'sec-ch-ua-arch', 'sec-ch-ua-bitness', 'sec-ch-ua-model', 'sec-ch-ua-wow64', 'sec-ch-ua-form-factors',
  'sec-fetch-site', 'sec-fetch-mode', 'sec-fetch-dest', 'sec-fetch-user', 'upgrade-insecure-requests', 'dnt', 'sec-gpc', 'accept-encoding']
// High-entropy hints are sent to an origin that asked for them, from its next request on (the probe's report is one).
const ACCEPT_CH = 'Sec-CH-UA-Full-Version-List, Sec-CH-UA-Arch, Sec-CH-UA-Bitness, Sec-CH-UA-Model, Sec-CH-UA-Platform-Version, Sec-CH-UA-WoW64, Sec-CH-UA-Form-Factors'
const picked = (req) => Object.fromEntries(HEADERS.filter((h) => req.headers[h] !== undefined).map((h) => [h, req.headers[h]]))

// The probe: what a site can read without asking the user. Runs in the top page, the frame and the popup.
const PROBE = String.raw`
const settle = (promise, ms = 3000) => Promise.race([Promise.resolve(promise).then((v) => v, (e) => ({ error: String(e && e.name || e) })), new Promise((r) => setTimeout(() => r({ error: 'timeout' }), ms))])
const safe = (f) => { try { return f() } catch (e) { return { error: String(e && e.name || e) } } }
const isNative = (f) => typeof f === 'function' ? /\{\s*\[native code\]\s*\}\s*$/.test(Function.prototype.toString.call(f)) : null
const getter = (proto, name) => { const d = proto && Object.getOwnPropertyDescriptor(proto, name); return d && (d.get || d.value) }
const NATIVE = {
  'Function.prototype.toString': () => Function.prototype.toString,
  'Navigator.webdriver(get)': () => getter(Navigator.prototype, 'webdriver'),
  'Navigator.userAgent(get)': () => getter(Navigator.prototype, 'userAgent'),
  'Navigator.platform(get)': () => getter(Navigator.prototype, 'platform'),
  'Navigator.languages(get)': () => getter(Navigator.prototype, 'languages'),
  'Navigator.requestMIDIAccess': () => Navigator.prototype.requestMIDIAccess,
  'Geolocation.getCurrentPosition': () => window.Geolocation && Geolocation.prototype.getCurrentPosition,
  'Geolocation.watchPosition': () => window.Geolocation && Geolocation.prototype.watchPosition,
  'MediaDevices.getUserMedia': () => window.MediaDevices && MediaDevices.prototype.getUserMedia,
  'Notification.requestPermission': () => window.Notification && Notification.requestPermission,
  'Clipboard.readText': () => window.Clipboard && Clipboard.prototype.readText,
  'Clipboard.read': () => window.Clipboard && Clipboard.prototype.read,
  'Permissions.query': () => window.Permissions && Permissions.prototype.query,
  'EventTarget.addEventListener': () => EventTarget.prototype.addEventListener,
  'HTMLSelectElement.showPicker': () => HTMLSelectElement.prototype.showPicker,
  'HTMLInputElement.showPicker': () => HTMLInputElement.prototype.showPicker,
  'HTMLElement.focus': () => HTMLElement.prototype.focus,
  'Document.hasFocus': () => Document.prototype.hasFocus,
  'Window.open': () => window.open,
  'Window.alert': () => window.alert,
  'Window.confirm': () => window.confirm,
  'Window.prompt': () => window.prompt,
  'Screen.width(get)': () => getter(Screen.prototype, 'width'),
  'Window.devicePixelRatio(get)': () => getter(window, 'devicePixelRatio') || getter(Window.prototype, 'devicePixelRatio'),
  'HTMLCanvasElement.toDataURL': () => HTMLCanvasElement.prototype.toDataURL,
  'WebGLRenderingContext.getParameter': () => window.WebGLRenderingContext && WebGLRenderingContext.prototype.getParameter,
  'Intl.DateTimeFormat.resolvedOptions': () => Intl.DateTimeFormat.prototype.resolvedOptions,
  'Date.getTimezoneOffset': () => Date.prototype.getTimezoneOffset,
}
const FONTS = ['Arial', 'Helvetica', 'Times New Roman', 'Courier New', 'DejaVu Sans', 'Liberation Sans', 'Noto Sans', 'Noto Sans CJK SC', 'Noto Color Emoji', 'Roboto', 'Ubuntu', 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', 'SimSun', 'WenQuanYi Micro Hei', 'Source Han Sans SC']
const fonts = () => {
  const ctx = document.createElement('canvas').getContext('2d')
  const width = (family) => { ctx.font = '72px ' + family; return ctx.measureText('mmmmmmmmmmlli 永和の가').width }
  const base = Object.fromEntries(['monospace', 'serif', 'sans-serif'].map((g) => [g, width(g)]))
  return Object.fromEntries(FONTS.map((f) => [f, Object.keys(base).some((g) => width('"' + f + '",' + g) !== base[g])]))
}
const gl = (kind) => safe(() => {
  const c = document.createElement('canvas').getContext(kind)
  if (!c) return null
  const info = c.getExtension('WEBGL_debug_renderer_info')
  return {
    vendor: c.getParameter(c.VENDOR), renderer: c.getParameter(c.RENDERER), version: c.getParameter(c.VERSION),
    unmaskedVendor: info ? c.getParameter(info.UNMASKED_VENDOR_WEBGL) : null, unmaskedRenderer: info ? c.getParameter(info.UNMASKED_RENDERER_WEBGL) : null,
    maxTextureSize: c.getParameter(c.MAX_TEXTURE_SIZE), extensions: (c.getSupportedExtensions() || []).length,
  }
})
const hash = (text) => { let h = 2166136261; for (let i = 0; i < text.length; i++) { h ^= text.charCodeAt(i); h = Math.imul(h, 16777619) } return (h >>> 0).toString(16) }
const canvasHash = () => safe(() => {
  const c = document.createElement('canvas'); c.width = 240; c.height = 60
  const x = c.getContext('2d'); x.textBaseline = 'top'; x.font = '16px sans-serif'; x.fillStyle = '#f60'; x.fillRect(100, 1, 62, 20)
  x.fillStyle = '#069'; x.fillText('BuckyOS diag 浏览器 ✓', 2, 15); x.fillStyle = 'rgba(102,204,0,0.7)'; x.fillText('BuckyOS diag 浏览器 ✓', 4, 17)
  return hash(c.toDataURL())
})
const audioHash = () => settle((async () => {
  const ctx = new OfflineAudioContext(1, 5000, 44100); const osc = ctx.createOscillator(); osc.type = 'triangle'; osc.frequency.value = 1000
  const comp = ctx.createDynamicsCompressor(); osc.connect(comp); comp.connect(ctx.destination); osc.start(0)
  const buf = await ctx.startRendering(); let sum = 0; const d = buf.getChannelData(0); for (let i = 4500; i < 5000; i++) sum += Math.abs(d[i])
  return sum.toFixed(6)
})())
const CODECS = { 'video/mp4; codecs="avc1.42E01E"': 0, 'video/mp4; codecs="hev1.1.6.L93.B0"': 0, 'video/webm; codecs="vp9"': 0, 'video/mp4; codecs="av01.0.05M.08"': 0, 'audio/mp4; codecs="mp4a.40.2"': 0, 'audio/mpeg': 0, 'audio/ogg; codecs="opus"': 0 }
const PERMISSIONS = ['geolocation', 'notifications', 'camera', 'microphone', 'clipboard-read', 'clipboard-write', 'storage-access', 'top-level-storage-access', 'midi', 'persistent-storage', 'idle-detection', 'window-management', 'local-fonts', 'screen-wake-lock', 'payment-handler', 'background-sync', 'accelerometer']
const mq = (q) => matchMedia(q).matches
// A known check for a DevTools client with the Runtime domain on: it serializes what is logged, reading an error's stack.
const consoleSerialized = () => safe(() => { let read = false; const e = new Error('diag'); Object.defineProperty(e, 'stack', { get() { read = true; return '' } }); console.debug(e); return read })
async function probe() {
  const n = navigator
  const out = { at: Date.now(), href: location.href, where: window.__diagWhere }
  out.navigator = {
    userAgent: n.userAgent, appVersion: n.appVersion, platform: n.platform, vendor: n.vendor, webdriver: n.webdriver,
    language: n.language, languages: n.languages, hardwareConcurrency: n.hardwareConcurrency, deviceMemory: n.deviceMemory,
    maxTouchPoints: n.maxTouchPoints, cookieEnabled: n.cookieEnabled, pdfViewerEnabled: n.pdfViewerEnabled, doNotTrack: n.doNotTrack,
    plugins: Array.from(n.plugins || []).map((p) => p.name), mimeTypes: (n.mimeTypes || []).length, onLine: n.onLine,
    connection: n.connection ? { effectiveType: n.connection.effectiveType, rtt: n.connection.rtt, downlink: n.connection.downlink, saveData: n.connection.saveData } : null,
    userActivation: n.userActivation ? { isActive: n.userActivation.isActive, hasBeenActive: n.userActivation.hasBeenActive } : null,
    gpu: 'gpu' in n, keyboard: 'keyboard' in n, bluetooth: 'bluetooth' in n, usb: 'usb' in n, hid: 'hid' in n, serial: 'serial' in n,
  }
  out.uaData = n.userAgentData ? {
    brands: n.userAgentData.brands, mobile: n.userAgentData.mobile, platform: n.userAgentData.platform,
    high: await settle(n.userAgentData.getHighEntropyValues(['architecture', 'bitness', 'model', 'platformVersion', 'fullVersionList', 'wow64', 'formFactors'])),
  } : null
  const tz = Intl.DateTimeFormat().resolvedOptions()
  out.locale = { timeZone: tz.timeZone, locale: tz.locale, offset: new Date().getTimezoneOffset(), numbers: (1234567.891).toLocaleString() }
  out.screen = { width: screen.width, height: screen.height, availWidth: screen.availWidth, availHeight: screen.availHeight, availLeft: screen.availLeft, availTop: screen.availTop, colorDepth: screen.colorDepth, pixelDepth: screen.pixelDepth, orientation: screen.orientation && screen.orientation.type, isExtended: screen.isExtended }
  out.window = { innerWidth, innerHeight, outerWidth, outerHeight, screenX, screenY, devicePixelRatio, visualScale: window.visualViewport && visualViewport.scale, historyLength: history.length, isSecureContext, crossOriginIsolated, opener: !!window.opener, framed: window !== window.top }
  out.media_queries = { pointerFine: mq('(pointer: fine)'), pointerCoarse: mq('(pointer: coarse)'), anyPointerCoarse: mq('(any-pointer: coarse)'), hover: mq('(hover: hover)'), dark: mq('(prefers-color-scheme: dark)'), reducedMotion: mq('(prefers-reduced-motion: reduce)'), standalone: mq('(display-mode: standalone)'), touchEvents: 'ontouchstart' in window }
  out.focus = { hasFocus: document.hasFocus(), visibilityState: document.visibilityState, hidden: document.hidden }
  out.chrome = { present: typeof window.chrome, keys: window.chrome ? Object.keys(window.chrome).sort() : null }
  out.consoleSerialized = consoleSerialized()
  out.native = Object.fromEntries(Object.entries(NATIVE).map(([k, f]) => [k, safe(() => isNative(f()))]))
  out.injected = { opendanPermissions: !!window[Symbol.for('opendan.permissions')], windowSymbols: Object.getOwnPropertySymbols(window).map(String), opendanGlobals: Object.getOwnPropertyNames(window).filter((k) => /opendan|playwright|puppeteer|__pw|cdc_|webdriver/i.test(k)) }
  out.webgl = gl('webgl'); out.webgl2 = gl('webgl2')
  out.canvas = canvasHash(); out.audio = await audioHash()
  out.audioContext = safe(() => { const a = new AudioContext(); const r = { sampleRate: a.sampleRate, baseLatency: a.baseLatency, maxChannels: a.destination.maxChannelCount }; a.close(); return r })
  out.fonts = fonts()
  out.codecs = Object.fromEntries(Object.keys(CODECS).map((c) => [c, document.createElement('video').canPlayType(c)]))
  out.mediaCapabilities = await settle(Promise.all(['avc1.42E01E', 'vp09.00.10.08', 'av01.0.05M.08'].map((codec) => navigator.mediaCapabilities.decodingInfo({ type: 'media-source', video: { contentType: 'video/' + (codec.startsWith('avc') ? 'mp4' : 'webm') + '; codecs="' + codec + '"', width: 1920, height: 1080, bitrate: 4000000, framerate: 30 } }).then((r) => [codec, r.supported, r.smooth, r.powerEfficient]))))
  out.eme = await settle(n.requestMediaKeySystemAccess ? n.requestMediaKeySystemAccess('com.widevine.alpha', [{ initDataTypes: ['cenc'], videoCapabilities: [{ contentType: 'video/mp4; codecs="avc1.42E01E"' }] }]).then(() => 'supported', (e) => 'unsupported:' + e.name) : 'none')
  out.permissions = Object.fromEntries(await Promise.all(PERMISSIONS.map(async (name) => [name, await settle(navigator.permissions.query({ name }).then((s) => s.state), 1500)])))
  out.notificationPermission = window.Notification ? Notification.permission : null
  out.storage = await settle(navigator.storage && navigator.storage.estimate().then((e) => ({ quotaMiB: Math.round(e.quota / 1048576) })))
  out.devices = await settle(navigator.mediaDevices ? navigator.mediaDevices.enumerateDevices().then((d) => d.map((x) => x.kind)) : null)
  out.voices = safe(() => speechSynthesis.getVoices().length)
  return out
}
`

const STYLE = '<style>body{margin:0;font:15px sans-serif}main{padding:12px}#popup{position:fixed;left:0;top:0;width:220px;height:120px;font:inherit}iframe{width:320px;height:80px;border:1px solid #999}pre{font-size:11px;white-space:pre-wrap}</style>'

const pageScript = (where, next) => `<script>
window.__diagWhere = ${JSON.stringify(where)}
${PROBE}
const params = new URLSearchParams(location.search)
const runId = params.get('run') || 'default'
const report = async (phase) => {
  const result = await probe()
  result.phase = phase
  await fetch('/diag/report?run=' + encodeURIComponent(runId) + '&where=' + encodeURIComponent(window.__diagWhere), { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(result) }).catch(() => {})
  const status = document.getElementById('status'); if (status) status.textContent = window.__diagWhere + ' reported (' + phase + ')'
}
let phase = null
const poll = async () => {
  try {
    const now = await (await fetch('/diag/phase?run=' + encodeURIComponent(runId), { cache: 'no-store' })).text()
    if (now !== phase) { phase = now; await report(now) }
  } catch (_) {}
  setTimeout(poll, 700)
}
${next}
poll()
</script>`

function topPage(url) {
  const runId = url.searchParams.get('run') || 'default'
  const alt = url.searchParams.get('alt')
  const step = url.searchParams.get('step') || 'first'
  const where = step === 'first' ? 'top' : 'top-after-navigation'
  const next = step === 'first'
    ? `
const here = new URL(location.href)
const other = ${JSON.stringify(alt)} || (here.hostname === '127.0.0.1' ? 'http://localhost:' + here.port : 'http://127.0.0.1:' + here.port)
const frame = document.createElement('iframe'); frame.src = other + '/diag/frame?run=' + encodeURIComponent(runId); document.querySelector('main').appendChild(frame)
let opened = false
const openPopup = () => { if (opened) return; opened = true; const w = window.open('/diag/popup?run=' + encodeURIComponent(runId), 'diagpopup', 'popup,width=600,height=500'); document.getElementById('status2').textContent = w ? 'popup opened' : 'popup blocked' }
document.getElementById('popup').addEventListener('click', openPopup)
// The navigation that follows the popup (or a while without one): a new document of the same tab.
setTimeout(() => { const u = new URL(location.href); u.searchParams.set('step', 'navigated'); location.assign(u) }, Number(params.get('navigate_after') || 15000))`
    : ''
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>diag ${where}</title>${STYLE}</head><body>
<button id="popup">open popup</button><main style="margin-top:130px"><h1>Browser diagnostics (${where})</h1><p id="status">probing…</p><p id="status2"></p></main>${pageScript(where, next)}</body></html>`
}

const smallPage = (where) => `<!doctype html><html><head><meta charset="utf-8"><title>diag ${where}</title>${STYLE}</head><body><p id="status">${where}: probing…</p>${pageScript(where, '')}</body></html>`

/** Answers a request of the diagnostic pages; false when it is not one. */
export function diagRoute(req, res, url, raw) {
  if (!url.pathname.startsWith('/diag')) return false
  const id = url.searchParams.get('run') || 'default'
  const r = run(id)
  const send = (status, body, type = 'text/html; charset=utf-8', extra = {}) => {
    res.writeHead(status, { 'Content-Type': type, 'Cache-Control': 'no-store', ...extra })
    res.end(body)
  }
  const page = (where, html) => {
    r.headers[where] = picked(req)
    send(200, html, 'text/html; charset=utf-8', { 'Accept-CH': ACCEPT_CH, 'Critical-CH': 'Sec-CH-UA-Full-Version-List, Sec-CH-UA-Arch, Sec-CH-UA-Platform-Version' })
  }
  switch (url.pathname) {
    case '/diag': {
      const step = url.searchParams.get('step') || 'first'
      page(step === 'first' ? 'top' : 'top-after-navigation', topPage(url))
      return true
    }
    case '/diag/frame': page('frame', smallPage('frame')); return true
    case '/diag/popup': page('popup', smallPage('popup')); return true
    case '/diag/report': {
      let body = null
      try { body = JSON.parse(raw) } catch (_) { return send(400, 'bad json', 'text/plain'), true }
      const where = url.searchParams.get('where') || body.where
      r.reports.push({ ...body, where, report_headers: picked(req), page_headers: r.headers[where] ?? null })
      send(204, '', 'text/plain', { 'Access-Control-Allow-Origin': '*' })
      return true
    }
    case '/diag/phase':
      if (req.method === 'POST') { r.phase = url.searchParams.get('phase') || r.phase; send(200, r.phase, 'text/plain') } else send(200, r.phase, 'text/plain', { 'Access-Control-Allow-Origin': '*' })
      return true
    case '/diag/results': send(200, JSON.stringify(r.reports), 'application/json'); return true
    default: return false
  }
}
