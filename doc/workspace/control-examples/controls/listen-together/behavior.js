/* Listen together (控件机制 TODO §0.4, H1–H6): everyone hears the playlist at the same moment.
 * Where the broadcast is: config.anchor = { pos_ms, at_ms, playing } (at service time at_ms the track config.track was at
 * pos_ms), so a listener plays at pos_ms + (aiws.serverNow() − at_ms). The owner changes it from the toolbar — `start`
 * and the play mode are written by the host without this page; `pause`, `resume` and `next` are forwarded here
 * (aiws.on('action:<id>')). A listener's own listening ("On my own"), their position in it and their volume are their
 * own user state (aiws.userState), never the shared document. */
(function () {
  var DEFAULTS = { title: 'Listen together', mode: 'order', track: '' }
  var AUDIO = /\.(mp3|m4a|aac|ogg|oga|opus|wav|flac|webm)$/i
  function isAudio(name, member) { return member.kind === 'file' && (/^audio\//.test(member.media_type || '') || AUDIO.test(name)) }

  // ---- before it is placed or re-bound (host API 5, on_bind "script", 控件 on_bind TODO §2.9): run without a page, so
  // nothing below this branch. Only folders that hold audio are offered; none: a new empty "Playlist" folder to fill.
  if (aiws.context.role === 'bind') {
    aiws.on('bind', async function (ctx) {
      if (ctx.reason === 'insert' && ctx.given.playlist) return { bindings: { playlist: ctx.given.playlist } }
      var folders = await ctx.find({ accepts: ['buckyos.dir'] })
      var playable = []
      for (var i = 0; i < folders.length && i < 64; i++) {
        var read = await ctx.read(folders[i].entity_id)
        var members = (read && read.content && read.content.payload && read.content.payload.members) || {}
        if (Object.keys(members).some(function (n) { return isAudio(n, members[n]) })) playable.push(folders[i].entity_id)
      }
      if (playable.length === 0) return { bindings: { playlist: 'new' }, create: [{ ref: 'new', kind: 'dir', name: 'Playlist' }] }
      if (playable.length === 1 && ctx.reason === 'insert') return { bindings: { playlist: playable[0] } }
      var picked = await ctx.dialogs.pickOne({
        title: /^zh/.test(aiws.locale) ? '要播放的文件夹' : 'Folder to play', // the host shows "Choose: …"
        query: { accepts: ['buckyos.dir'], ids: playable },
        current: ctx.given.playlist || null,
      })
      return picked ? { bindings: { playlist: picked } } : null
    })
    return
  }

  var $ = function (id) { return document.getElementById(id) }
  var audio = $('audio'), wave = $('wave'), lyricsEl = $('lyrics')
  var config = Object.assign({}, DEFAULTS, aiws.context.config || {})
  var tracks = []          // [{ name, lrc }] in folder order
  var loaded = null        // the track name whose address the player has
  var lyrics = []          // [{ t (ms), text }]
  var own = false          // listening on my own
  var ownTrack = null
  var dragging = false

  // ---- where the broadcast is

  function anchor() {
    var a = config.anchor
    return a && typeof a.pos_ms === 'number' && typeof a.at_ms === 'number' ? a : null
  }
  function livePosition() {
    var a = anchor()
    if (!a) return 0
    return a.playing ? a.pos_ms + Math.max(0, aiws.serverNow() - a.at_ms) : a.pos_ms
  }
  function liveTrack() {
    for (var i = 0; i < tracks.length; i++) if (tracks[i].name === config.track) return tracks[i]
    return tracks[0] || null
  }
  function current() { return own ? (find(ownTrack) || liveTrack()) : liveTrack() }
  function find(name) { for (var i = 0; i < tracks.length; i++) if (tracks[i].name === name) return tracks[i]; return null }

  // ---- the playlist (the bound folder) and a track's address (Range playable: no whole download, H6)

  async function loadPlaylist() {
    var b = aiws.bindings.playlist
    if (!b) { $('track').textContent = 'Bind a folder of audio'; return }
    var dir = await aiws.read(b.entity_id)
    var members = (dir && dir.content && dir.content.payload && dir.content.payload.members) || {}
    var names = Object.keys(members).sort()
    tracks = names.filter(function (n) { return isAudio(n, members[n]) }).map(function (n) {
      var lrc = n.replace(/\.[^.]+$/, '') + '.lrc'
      return { name: n, lrc: members[lrc] ? lrc : null }
    })
  }

  async function ensureLoaded(track) {
    if (!track || loaded === track.name) return
    loaded = track.name
    lyrics = []
    renderLyrics()
    audio.src = await aiws.mediaUrl('playlist', track.name)
    if (track.lrc) {
      try {
        var text = await (await fetch(await aiws.mediaUrl('playlist', track.lrc))).text()
        if (loaded === track.name) { lyrics = parseLrc(text); renderLyrics() }
      } catch (e) { /* lyrics are a nicety */ }
    }
  }

  function parseLrc(text) {
    var out = []
    text.split(/\r?\n/).forEach(function (line) {
      var times = [], m, re = /\[(\d{1,3}):(\d{1,2}(?:\.\d{1,3})?)\]/g
      while ((m = re.exec(line))) times.push(Number(m[1]) * 60000 + Math.round(Number(m[2]) * 1000))
      var words = line.replace(/\[[^\]]*\]/g, '').trim()
      times.forEach(function (t) { out.push({ t: t, text: words }) })
    })
    return out.sort(function (a, b) { return a.t - b.t })
  }

  function renderLyrics() {
    lyricsEl.innerHTML = ''
    lyrics.forEach(function (l) { var li = document.createElement('li'); li.textContent = l.text || '·'; lyricsEl.appendChild(li) })
  }

  // ---- following: seek beyond 400 ms off, nudge the rate between 80 and 400 ms, pause with the broadcast

  function follow() {
    var track = current()
    $('track').textContent = track ? track.name : (tracks.length ? '' : 'The folder holds no audio')
    $('badge').textContent = own ? 'On my own' : (anchor() && anchor().playing ? 'Live' : 'Paused')
    $('badge').className = 'badge' + (own ? ' own' : '')
    $('live').textContent = own ? 'Back to live' : 'On my own'
    $('play').className = audio.paused ? '' : 'playing'
    if (!track || own) return
    ensureLoaded(track).then(function () {
      var a = anchor()
      var target = livePosition()
      var duration = isFinite(audio.duration) ? audio.duration * 1000 : null
      if (!a || !a.playing || (duration !== null && target >= duration - 50)) {
        if (!audio.paused) audio.pause()
        if (a && Math.abs(audio.currentTime * 1000 - Math.min(target, duration || target)) > 80) audio.currentTime = Math.min(target, duration || target) / 1000
        return
      }
      var off = target - audio.currentTime * 1000
      if (Math.abs(off) > 400) { audio.currentTime = target / 1000; audio.playbackRate = 1 }
      else if (Math.abs(off) > 80) audio.playbackRate = off > 0 ? 1.05 : 0.95
      else audio.playbackRate = 1
      if (audio.paused) play()
    })
  }

  function play() {
    audio.play().then(function () { $('tap').hidden = true }, function () { $('tap').hidden = false })
  }

  // ---- drawing: a waveform that runs with the playback, the lyric line of the moment

  function bars(name) {
    var h = 2166136261, out = []
    for (var i = 0; i < name.length; i++) h = Math.imul(h ^ name.charCodeAt(i), 16777619) >>> 0
    for (var j = 0; j < 72; j++) { h = Math.imul(h ^ j, 16777619) >>> 0; out.push(0.25 + ((h % 1000) / 1000) * 0.75) }
    return out
  }
  function fmt(ms) { var s = Math.max(0, Math.floor(ms / 1000)); var m = Math.floor(s / 60); return m + ':' + String(s % 60).padStart(2, '0') }

  function draw() {
    var track = current()
    var ratio = window.devicePixelRatio || 1
    var w = wave.clientWidth, h = wave.clientHeight
    if (wave.width !== Math.round(w * ratio)) { wave.width = Math.round(w * ratio); wave.height = Math.round(h * ratio) }
    var g = wave.getContext('2d')
    g.setTransform(ratio, 0, 0, ratio, 0, 0)
    g.clearRect(0, 0, w, h)
    var pos = audio.currentTime * 1000, dur = isFinite(audio.duration) ? audio.duration * 1000 : 0
    var done = dur ? pos / dur : 0
    var list = bars(track ? track.name : '')
    var bw = w / list.length
    var t = performance.now() / 180
    list.forEach(function (v, i) {
      var live = !audio.paused && Math.abs(i / list.length - done) < 0.06 ? 0.8 + 0.2 * Math.sin(t + i) : 1
      var bh = Math.max(2, v * live * (h - 4))
      g.fillStyle = i / list.length <= done ? '#6b5bd2' : '#c9c3e6'
      g.fillRect(i * bw + 1, (h - bh) / 2, Math.max(1, bw - 2), bh)
    })
    $('pos').textContent = fmt(pos)
    $('dur').textContent = fmt(dur)
    var at = -1
    for (var k = 0; k < lyrics.length; k++) if (lyrics[k].t <= pos) at = k
    Array.prototype.forEach.call(lyricsEl.children, function (li, i) { li.className = i === at ? 'now' : '' })
    var li = lyricsEl.children[at]
    if (li) lyricsEl.scrollTop = li.offsetTop - lyricsEl.clientHeight / 2 + li.clientHeight / 2
    requestAnimationFrame(draw)
  }

  // ---- on my own: position and volume kept in the person's own user state (H4)

  function saveOwn() {
    if (!own) return
    aiws.userState.set('own', { track: (current() || {}).name || null, pos_ms: Math.round(audio.currentTime * 1000) })
  }
  function goOwn() {
    if (own) return
    own = true
    ownTrack = (liveTrack() || {}).name || null
    audio.playbackRate = 1
    saveOwn()
    follow()
  }
  function goLive() {
    own = false
    aiws.userState.set('own', null)
    follow()
  }
  async function ownStep(delta) {
    goOwn()
    var i = tracks.indexOf(current())
    var next = tracks[(i + delta + tracks.length) % tracks.length]
    if (!next) return
    ownTrack = next.name
    await ensureLoaded(next)
    audio.currentTime = 0
    play()
    saveOwn()
  }

  $('live').addEventListener('click', function () { if (own) goLive(); else { goOwn(); play() } })
  $('prev').addEventListener('click', function () { ownStep(-1) })
  $('next').addEventListener('click', function () { ownStep(1) })
  $('play').addEventListener('click', function () { goOwn(); if (audio.paused) play(); else audio.pause(); saveOwn() })
  $('tap').addEventListener('click', function () { play() })
  $('volume').addEventListener('input', function () { audio.volume = Number($('volume').value); aiws.userState.set('volume', audio.volume) })
  // a drag on the waveform seeks: on my own (the broadcast is the owner's, from the toolbar)
  function seekAt(event) {
    var r = wave.getBoundingClientRect()
    if (!isFinite(audio.duration)) return
    goOwn()
    audio.currentTime = Math.max(0, Math.min(1, (event.clientX - r.left) / r.width)) * audio.duration
  }
  wave.addEventListener('pointerdown', function (e) { dragging = true; wave.setPointerCapture(e.pointerId); seekAt(e) })
  wave.addEventListener('pointermove', function (e) { if (dragging) seekAt(e) })
  wave.addEventListener('pointerup', function () { if (dragging) { dragging = false; if (audio.paused) play(); saveOwn() } })
  audio.addEventListener('ended', function () { if (own) ownStep(1) })
  window.addEventListener('pagehide', saveOwn)
  setInterval(saveOwn, 5000)

  // ---- the owner's actions forwarded from the toolbar (H2): one Commit each, of this instance's own config

  async function writeConfig(props, label) {
    var read = await aiws.read(aiws.context.cell_id)
    var revs = (read && read.content && read.content.key_revs) || {}
    var expect = {}
    Object.keys(props).forEach(function (k) { expect[k] = revs['p:config.' + k] || 0 })
    return aiws.submit([{ op: 'entity.set_props', entity_id: aiws.context.cell_id, key: 'config', props: props, expect: expect }], label)
  }
  aiws.on('action:pause', function () {
    return writeConfig({ anchor: { pos_ms: Math.round(livePosition()), at_ms: aiws.serverNow(), playing: false } }, 'Pause listening together')
  })
  aiws.on('action:resume', function () {
    var a = anchor()
    return writeConfig({ anchor: { pos_ms: a ? Math.round(a.playing ? livePosition() : a.pos_ms) : 0, at_ms: aiws.serverNow(), playing: true } }, 'Resume listening together')
  })
  aiws.on('action:next', function () {
    if (!tracks.length) return null
    var i = tracks.indexOf(liveTrack())
    var j = config.mode === 'shuffle' && tracks.length > 1 ? (i + 1 + Math.floor(Math.random() * (tracks.length - 1))) % tracks.length : (i + 1) % tracks.length
    // the track and a fresh anchor together: nobody hears the new song at the old position
    return writeConfig({ track: tracks[j].name, anchor: { pos_ms: 0, at_ms: aiws.serverNow(), playing: true } }, 'Next song')
  })

  // ---- the owner changed something: a running player follows at once (H1)
  aiws.onConfig(function (next) {
    config = Object.assign({}, DEFAULTS, next || {})
    $('title').textContent = config.title
    follow()
  })
  aiws.watch('playlist', function () { loadPlaylist().then(follow) })

  ;(async function start() {
    $('title').textContent = config.title
    try {
      var volume = await aiws.userState.get('volume')
      audio.volume = typeof volume === 'number' ? volume : 1
      $('volume').value = String(audio.volume)
      await loadPlaylist()
      var saved = await aiws.userState.get('own')
      if (saved && saved.track && find(saved.track)) {
        own = true
        ownTrack = saved.track
        await ensureLoaded(find(saved.track))
        audio.currentTime = (saved.pos_ms || 0) / 1000
      }
    } catch (e) {
      $('track').textContent = String(e && e.message || e)
    } finally {
      follow()
      setInterval(follow, 500)
      requestAnimationFrame(draw)
      aiws.ready()
    }
  })()
})()
