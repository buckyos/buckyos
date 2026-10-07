/* The Mock wish executor (phase two §7.6, D8): a fake wish with the real wish's interaction. It is an
 * HTML extension definition — saved as a `buckyos.block-def` entity (D9) — whose script answers the
 * host's `analyze` and `execute` requests deterministically from the inputs it is given. Nothing here
 * calls a model; every output says it is simulated. The generators are ported from the ai-canvas
 * prototype (`canvas/agent/mock.ts`, `canvas/agent/aigc.ts`): quarterly sales analysis, and the
 * three-stage short-film workflow (character sheets → storyboard → frame preview). SVG images are
 * returned as markup; the host uploads them as assets (§7.6).
 *
 * Failure injection (deterministic): a prompt containing `#fail` makes execution throw; `#invalid`
 * returns a candidate the backend must refuse; `#slow` delays execution. */

export const MOCK_WISH_DEF_ID = 'def-mock-wish'
export const MOCK_WISH_RENDERER = 'buckyos.mock-wish'

const MOCK_JS = String.raw`
(function () {
  'use strict';
  // ---- deterministic helpers (FNV-1a + mulberry32)
  function hash(text) { var h = 0x811c9dc5; for (var i = 0; i < text.length; i++) { h ^= text.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; } return h >>> 0; }
  function rng(seed) { var a = seed >>> 0; return function () { a = (a + 0x6d2b79f5) >>> 0; var t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
  function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
  function num(v) { if (typeof v === 'number') return v; if (typeof v === 'string') { var n = Number(v.replace(/[,¥$%\s]/g, '')); return isFinite(n) ? n : 0; } return 0; }
  function money(n) { return '¥' + Math.round(n).toLocaleString('zh-CN'); }
  function pct(n) { return (n * 100).toFixed(1) + '%'; }
  var FONT = 'font-family="system-ui,-apple-system,Segoe UI,PingFang SC,Noto Sans CJK SC,sans-serif"';

  // ---- inputs
  function flatten(inputs) {
    var out = [];
    inputs.forEach(function (it) { out.push(it); if (it.content && it.content.children) out = out.concat(flatten(it.content.children)); });
    return out;
  }
  function tableNamed(t) { // rows keyed by field name
    var names = {}; (t.content.fields || []).forEach(function (f) { names[f.field_id] = f.name; });
    return { title: t.title || t.name || t.entity_id, columns: (t.content.fields || []).map(function (f) { return f.name; }),
      rows: (t.content.rows || []).map(function (r) { var o = {}; Object.keys(r).forEach(function (k) { o[names[k] || k] = r[k]; }); return o; }), entity_id: t.entity_id };
  }
  function findCol(cols) { for (var i = 1; i < arguments.length; i++) { var n = arguments[i]; var hit = cols.filter(function (c) { return c === n; })[0] || cols.filter(function (c) { return String(c).toLowerCase().indexOf(String(n).toLowerCase()) >= 0; })[0]; if (hit) return hit; } return undefined; }
  function isCharacterTable(t) { return !!findCol(t.columns, '角色', '人物', '姓名', '名字', 'name', 'character') && !!findCol(t.columns, '外貌', '外形', '描述', '定位', '性格', '主色', 'look', 'role'); }
  function isSalesTable(t) { return !!findCol(t.columns, '销售额', '营收', '收入', 'revenue'); }
  function classify(prompt, inputs) {
    var all = flatten(inputs);
    var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed);
    var images = all.filter(function (i) { return i.type_id === 'buckyos.asset-ref'; });
    var texts = all.filter(function (i) { return i.type_id === 'buckyos.richtext'; });
    var charTable = tables.filter(isCharacterTable)[0];
    var sales = tables.filter(isSalesTable)[0];
    var hasFrames = images.some(function (im) { return /^S\d+/.test(im.title || im.name || ''); }) || tables.some(function (t) { return !!findCol(t.columns, '镜号', '时长', 'shot'); });
    var hasCharImages = images.some(function (im) { return !/^S\d+/.test(im.title || im.name || ''); });
    var hasScript = texts.some(function (t) { return /剧本|大纲|故事|script/i.test(t.title || t.name || ''); });
    var wantsVideo = /视频|成片|合成|剪辑|video|render/i.test(prompt);
    var wantsStoryboard = /故事板|分镜|storyboard/i.test(prompt);
    var wantsCharacters = /角色|人物|立绘|形象|设定图|character/i.test(prompt);
    if (sales && !wantsCharacters && !wantsStoryboard && !wantsVideo) return 'sales';
    if (wantsVideo && hasFrames) return 'video';
    if (wantsStoryboard && (hasScript || hasCharImages || !charTable)) return 'storyboard';
    if (charTable && wantsCharacters) return 'characters';
    if (wantsStoryboard) return 'storyboard';
    if (wantsVideo && images.length) return 'video';
    if (sales) return 'sales';
    return 'generic';
  }

  // ---- analyze: translate the prompt into a context prompt that names its inputs
  aiws.on('analyze', function (req) {
    var prompt = String(req.prompt || '');
    var visible = req.visible || [];
    var declared = req.declared || [];
    var picked = declared.slice();
    var warnings = [];
    function mention(e) { var n = e.title || e.name || ''; return n && prompt.indexOf(n) >= 0; }
    visible.forEach(function (e) {
      if (picked.some(function (p) { return p.entity_id === e.entity_id; })) return;
      if (mention(e)) picked.push({ entity_id: e.entity_id, label: e.title || e.name || e.entity_id, version: { mode: 'follow' } });
    });
    if (picked.length === 0) {
      // nothing named: take the data entities of the wish's own folder as the context the user sees
      visible.filter(function (e) { return e.same_folder && e.type_id !== 'buckyos.wish' && e.type_id !== 'buckyos.block-def'; }).slice(0, 4)
        .forEach(function (e) { picked.push({ entity_id: e.entity_id, label: e.title || e.name || e.entity_id, version: { mode: 'follow' } }); });
      if (picked.length === 0) warnings.push('没有找到可作为输入的数据：请在提示词中写明数据名称，或手动添加输入');
    }
    var names = picked.map(function (p) { return '「' + (p.label || p.entity_id) + '」(' + p.entity_id + ')'; }).join('、');
    var context = '【模拟执行器】读取 ' + (names || '（无输入）') + ' 的当前内容；按用户意图「' + prompt.replace(/#\w+/g, '').trim() + '」生成结果，结果标明为模拟，不调用任何模型。';
    return { context_prompt: context, inputs: picked, warnings: warnings };
  });

  // ---- sales analysis (PRD §13.6 port)
  function groupBy(rows, key) { var m = {}; rows.forEach(function (r) { var k = String(r[key] == null ? '' : r[key]); (m[k] = m[k] || []).push(r); }); return m; }
  function sum(rows, key) { return rows.reduce(function (a, r) { return a + num(r[key]); }, 0); }
  function sales(table, prompt) {
    var cols = table.columns;
    var rev = findCol(cols, '销售额', '营收', '收入'); var cost = findCol(cols, '成本'); var target = findCol(cols, '目标'); var prev = findCol(cols, '上季度', '上期', '去年');
    var region = findCol(cols, '区域', '地区', '大区'); var product = findCol(cols, '产品', '品类', '商品');
    var rows = table.rows;
    var totalRev = sum(rows, rev), totalCost = cost ? sum(rows, cost) : 0, totalTarget = target ? sum(rows, target) : 0, totalPrev = prev ? sum(rows, prev) : 0;
    var margin = totalRev ? (totalRev - totalCost) / totalRev : 0, completion = totalTarget ? totalRev / totalTarget : 0, growth = totalPrev ? (totalRev - totalPrev) / totalPrev : 0;
    var warnings = [], assumptions = ['毛利率 = (销售额 − 成本) / 销售额', '目标完成率 = 销售额 / 目标'];
    if (!cost) warnings.push('未找到"成本"列，毛利率按 0 计算'); if (!target) warnings.push('未找到"目标"列，无法计算目标完成率');
    var results = [];
    results.push({ name: '指标', type: 'record', renderer: 'record', title: '核心指标（模拟）', size: { w: 320, h: 170 },
      content: { schema: { properties: [{ key: 'revenue', name: '本季度销售额', type: 'decimal', scale: 2 }, { key: 'margin', name: '整体毛利率', type: 'text' }, { key: 'completion', name: '目标完成率', type: 'text' }, { key: 'growth', name: '环比', type: 'text' }] },
        props: { revenue: totalRev.toFixed(2), margin: pct(margin), completion: target ? pct(completion) : '—', growth: prev ? pct(growth) : '—' } } });
    var regionRows = [];
    if (region) { var byRegion = groupBy(rows, region); Object.keys(byRegion).forEach(function (name) { var rs = byRegion[name]; regionRows.push({ region: name, revenue: sum(rs, rev), target: target ? sum(rs, target) : 0, growth: prev ? (sum(rs, prev) ? (sum(rs, rev) - sum(rs, prev)) / sum(rs, prev) : 0) : 0 }); }); }
    if (region) results.push({ name: '区域对比', type: 'table', renderer: 'sample.bar-chart', title: '各区域销售额对比（模拟）', size: { w: 420, h: 260 }, config: { value: 'revenue', by: 'region' },
      content: { fields: [{ field_id: 'region', name: '区域', type: 'text' }, { field_id: 'revenue', name: '销售额', type: 'number' }, { field_id: 'target', name: '目标', type: 'number' }, { field_id: 'growth', name: '环比', type: 'text' }],
        rows: regionRows.map(function (r) { return { region: r.region, revenue: Math.round(r.revenue), target: Math.round(r.target), growth: pct(r.growth) }; }) } });
    var ranked = rows.map(function (r) { var p = prev ? num(r[prev]) : 0; var g = p ? (num(r[rev]) - p) / p : 0; return { label: [region ? r[region] : null, product ? r[product] : null].filter(Boolean).join(' · '), growth: g, rev: num(r[rev]), cost: cost ? num(r[cost]) : 0, target: target ? num(r[target]) : 0 }; }).sort(function (a, b) { return b.growth - a.growth; });
    var anomalies = ranked.filter(function (x) { return (cost && x.cost > x.rev) || (target && x.target && x.rev / x.target < 0.7) || x.growth < -0.1; });
    if (anomalies.length) results.push({ name: '异常明细', type: 'table', renderer: 'table', title: '异常明细（模拟）', size: { w: 520, h: 220 },
      content: { fields: [{ field_id: 'item', name: '项目', type: 'text' }, { field_id: 'revenue', name: '销售额', type: 'number' }, { field_id: 'completion', name: '目标完成率', type: 'text' }, { field_id: 'growth', name: '环比', type: 'text' }, { field_id: 'issue', name: '问题', type: 'text' }],
        rows: anomalies.map(function (a) { var issues = []; if (cost && a.cost > a.rev) issues.push('负毛利'); if (target && a.target && a.rev / a.target < 0.7) issues.push('远低于目标'); if (a.growth < -0.1) issues.push('明显下滑'); return { item: a.label, revenue: Math.round(a.rev), completion: a.target ? pct(a.rev / a.target) : '—', growth: pct(a.growth), issue: issues.join('、') }; }) } });
    var best = regionRows.slice().sort(function (a, b) { return b.growth - a.growth; });
    var lines = ['# 本季度经营总结（模拟生成）',
      '本季度总销售额 ' + money(totalRev) + (prev ? '，环比' + (growth >= 0 ? '增长 ' : '下降 ') + pct(Math.abs(growth)) : '') + (cost ? '，整体毛利率 ' + pct(margin) : '') + (target ? '，目标完成率 ' + pct(completion) : '') + '。'];
    if (best.length) lines.push('- 增长最快的区域是 ' + best[0].region + '（环比 ' + pct(best[0].growth) + '），最弱的是 ' + best[best.length - 1].region + '（环比 ' + pct(best[best.length - 1].growth) + '）。');
    if (ranked.length) lines.push('- 增长最快的三项：' + ranked.slice(0, 3).map(function (t) { return t.label + '（' + pct(t.growth) + '）'; }).join('、') + '。');
    lines.push(anomalies.length ? '- 发现 ' + anomalies.length + ' 条需要关注的异常记录（负毛利、远低于目标或明显下滑），详见异常明细表。' : '- 未发现明显异常记录。');
    lines.push('建议：优先复盘下滑项的成本结构与目标设定，对增长项追加库存与渠道投入。');
    if (/#invalid/.test(prompt)) results[0].content.props.revenue = 'not-a-number';
    results.push({ name: '摘要', type: 'richtext', renderer: 'richtext', title: '管理层总结（模拟）', size: { w: 520, h: 260 }, content: { markdown: lines.join('\n') } });
    return { results: results, warnings: warnings, assumptions: assumptions, summary: '季度经营分析：销售额 ' + money(totalRev) + '，毛利率 ' + pct(margin) };
  }

  // ---- AIGC: colours, figures, cards, storyboard frames (ported, deterministic)
  var COLOR_WORDS = [[/青|cyan|teal/i, '#22d3ee'], [/蓝|blue|navy/i, '#3b82f6'], [/紫|purple|violet/i, '#a855f7'], [/朱|红|red|crimson/i, '#ef4444'], [/橙|orange/i, '#f97316'], [/琥珀|amber|金|gold|黄|yellow/i, '#f59e0b'], [/绿|green/i, '#22c55e'], [/粉|pink|magenta/i, '#ec4899'], [/白|white|银|silver/i, '#e5e7eb'], [/黑|black|炭/i, '#334155'], [/灰|gray|grey/i, '#94a3b8'], [/棕|褐|brown/i, '#a16207']];
  function pickColor(text, seed) { for (var i = 0; i < COLOR_WORDS.length; i++) if (COLOR_WORDS[i][0].test(text)) return COLOR_WORDS[i][1]; return 'hsl(' + (hash(seed) % 360) + ' 70% 60%)'; }
  function accentFor(color, seed) { var h = (hash(seed) % 360) + 180; return color.indexOf('hsl') === 0 ? 'hsl(' + (h % 360) + ' 80% 70%)' : ['#fde68a', '#a5f3fc', '#f9a8d4', '#bbf7d0'][hash(seed) % 4]; }
  function isRobot(text) { return /机器人|机械|AI|android|robot|droid/i.test(text); }
  function styleLabel(texts, prompt) {
    var styleText = texts.filter(function (t) { return /风格|style/i.test(t.title || t.name || '') || /风格[:：]/.test(t.content.text || ''); })[0];
    var raw = styleText ? styleText.content.text : prompt;
    var line = raw.split('\n').map(function (l) { return l.replace(/^#+\s*/, '').replace(/^风格[:：]\s*/, '').trim(); }).filter(function (l) { return l.length > 0 && !/^风格|^视觉风格$/.test(l); })[0];
    return (line || '默认风格').slice(0, 22);
  }
  function figure(x, baseY, s, c, seed) {
    var r = rng(seed); var skin = ['#f1d3b3', '#e8c39e', '#d9a679', '#c68a5b'][seed % 4]; var hair = ['#111827', '#3f2a1d', '#7c2d12', '#1e293b', '#4c1d95'][seed % 5]; var p = [];
    p.push('<ellipse cx="' + x + '" cy="' + baseY + '" rx="' + 44 * s + '" ry="' + 8 * s + '" fill="#000" opacity="0.35"/>');
    p.push('<path d="M' + (x - 40 * s) + ',' + baseY + ' L' + (x - 34 * s) + ',' + (baseY - 130 * s) + ' Q' + x + ',' + (baseY - 150 * s) + ' ' + (x + 34 * s) + ',' + (baseY - 130 * s) + ' L' + (x + 40 * s) + ',' + baseY + ' Z" fill="' + c.color + '"/>');
    p.push('<rect x="' + (x - 6 * s) + '" y="' + (baseY - 132 * s) + '" width="' + 12 * s + '" height="' + 62 * s + '" rx="' + 4 * s + '" fill="' + c.accent + '" opacity="0.85"/>');
    if (/围裙/.test(c.outfit)) p.push('<rect x="' + (x - 26 * s) + '" y="' + (baseY - 100 * s) + '" width="' + 52 * s + '" height="' + 90 * s + '" rx="' + 6 * s + '" fill="#f8fafc" opacity="0.85"/>');
    if (/背包|包/.test(c.outfit)) p.push('<rect x="' + (x - 58 * s) + '" y="' + (baseY - 125 * s) + '" width="' + 24 * s + '" height="' + 60 * s + '" rx="' + 6 * s + '" fill="' + c.accent + '"/>');
    if (c.robot) { var hy = baseY - 190 * s; p.push('<rect x="' + (x - 26 * s) + '" y="' + hy + '" width="' + 52 * s + '" height="' + 46 * s + '" rx="' + 10 * s + '" fill="#cbd5e1"/><circle cx="' + x + '" cy="' + (hy - 20 * s) + '" r="' + 5 * s + '" fill="' + c.accent + '"/><rect x="' + (x - 17 * s) + '" y="' + (hy + 16 * s) + '" width="' + 12 * s + '" height="' + 8 * s + '" fill="' + c.accent + '"/><rect x="' + (x + 5 * s) + '" y="' + (hy + 16 * s) + '" width="' + 12 * s + '" height="' + 8 * s + '" fill="' + c.accent + '"/>'); }
    else { var cy = baseY - 165 * s; p.push('<circle cx="' + x + '" cy="' + cy + '" r="' + 28 * s + '" fill="' + skin + '"/><path d="M' + (x - 29 * s) + ',' + (cy - 4 * s) + ' Q' + x + ',' + (cy - 44 * s) + ' ' + (x + 29 * s) + ',' + (cy - 4 * s) + ' Q' + (x + 20 * s) + ',' + (cy - 22 * s) + ' ' + x + ',' + (cy - 26 * s) + ' Q' + (x - 20 * s) + ',' + (cy - 22 * s) + ' ' + (x - 29 * s) + ',' + (cy - 4 * s) + ' Z" fill="' + hair + '"/><circle cx="' + (x - 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 2.5 * s + '" fill="#111827"/><circle cx="' + (x + 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 2.5 * s + '" fill="#111827"/>');
      if (/眼镜|护目镜/.test(c.outfit)) p.push('<circle cx="' + (x - 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 8 * s + '" fill="none" stroke="' + c.accent + '" stroke-width="' + 2 * s + '"/><circle cx="' + (x + 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 8 * s + '" fill="none" stroke="' + c.accent + '" stroke-width="' + 2 * s + '"/>');
      if (/耳机|耳麦/.test(c.outfit)) p.push('<path d="M' + (x - 30 * s) + ',' + cy + ' A' + 30 * s + ',' + 30 * s + ' 0 0 1 ' + (x + 30 * s) + ',' + cy + '" fill="none" stroke="' + c.accent + '" stroke-width="' + 4 * s + '"/>'); }
    if (/伞/.test(c.outfit)) { var ux = x + 50 * s, uy = baseY - 200 * s; p.push('<path d="M' + (ux - 60 * s) + ',' + (uy + 30 * s) + ' A' + 60 * s + ',' + 60 * s + ' 0 0 1 ' + (ux + 60 * s) + ',' + (uy + 30 * s) + ' Z" fill="' + c.accent + '" opacity="0.9"/><line x1="' + ux + '" y1="' + (uy + 30 * s) + '" x2="' + ux + '" y2="' + (baseY - 60 * s) + '" stroke="#e2e8f0" stroke-width="' + 3 * s + '"/>'); }
    if (r() > 0.5) p.push('<circle cx="' + (x + 30 * s) + '" cy="' + (baseY - 110 * s) + '" r="' + 5 * s + '" fill="' + c.accent + '" opacity="0.7"/>');
    return p.join('');
  }
  function characterCard(c, style) {
    var seed = hash(c.name + '|' + c.look + '|' + style); var r = rng(seed); var deco = [];
    for (var i = 0; i < 7; i++) { var cx = Math.round(r() * 480), cy = Math.round(r() * 420), rad = Math.round(10 + r() * 60); deco.push('<circle cx="' + cx + '" cy="' + cy + '" r="' + rad + '" fill="' + (i % 2 ? c.accent : c.color) + '" opacity="' + (0.05 + r() * 0.12).toFixed(2) + '"/>'); }
    var traits = c.personality.split(/[、,，\/\s]+/).filter(Boolean).slice(0, 3);
    var chips = traits.map(function (t, i) { var w = t.length * 13 + 18, x = 24 + i * (w + 8); return '<rect x="' + x + '" y="562" width="' + w + '" height="24" rx="12" fill="#fff" opacity="0.14"/><text x="' + (x + w / 2) + '" y="579" text-anchor="middle" font-size="12" fill="#f8fafc" ' + FONT + '>' + esc(t) + '</text>'; }).join('');
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 600" width="480" height="600"><defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0b1020"/><stop offset="1" stop-color="' + c.color + '" stop-opacity="0.55"/></linearGradient><radialGradient id="glow"><stop offset="0" stop-color="' + c.accent + '" stop-opacity="0.5"/><stop offset="1" stop-color="' + c.accent + '" stop-opacity="0"/></radialGradient></defs><rect width="480" height="600" fill="#0b1020"/><rect width="480" height="600" fill="url(#bg)"/><circle cx="240" cy="260" r="220" fill="url(#glow)"/>' + deco.join('') + figure(240, 470, 1.55, c, seed) + '<rect x="0" y="484" width="480" height="116" fill="#020617" opacity="0.55"/><text x="24" y="522" font-size="32" font-weight="700" fill="#f8fafc" ' + FONT + '>' + esc(c.name) + '</text><text x="24" y="548" font-size="15" fill="' + c.accent + '" ' + FONT + '>' + esc(c.role) + (c.colorText ? ' · 主色 ' + esc(c.colorText) : '') + '</text>' + chips + '<text x="20" y="30" font-size="12" fill="#e2e8f0" opacity="0.8" ' + FONT + '>角色设定图 · ' + esc(style) + '</text><text x="460" y="30" text-anchor="end" font-size="11" fill="#e2e8f0" opacity="0.6" ' + FONT + '>Mock · 模拟生成</text></svg>';
  }
  function parseCharacters(table) {
    var cols = table.columns; var name = findCol(cols, '角色', '人物', '姓名', '名字', 'name', 'character'), role = findCol(cols, '定位', '身份', '职业', 'role'), look = findCol(cols, '外貌', '外形', '描述', 'look', 'appearance'), personality = findCol(cols, '性格', 'personality'), color = findCol(cols, '主色', '颜色', '色调', 'color'), outfit = findCol(cols, '服装', '道具', '穿着', '装扮', 'outfit');
    function str(row, col) { return col && row[col] != null ? String(row[col]) : ''; }
    return table.rows.map(function (row) { var n = str(row, name).trim(); if (!n) return null; var all = [str(row, role), str(row, look), str(row, outfit)].join(' '); var colorText = str(row, color); var c = pickColor(colorText || all, n);
      return { name: n, role: str(row, role) || '角色', look: str(row, look), personality: str(row, personality), colorText: colorText, outfit: str(row, outfit), color: c, accent: accentFor(c, n), robot: isRobot(all) }; }).filter(Boolean).slice(0, 6);
  }
  function charactersFromImages(images) { return images.filter(function (im) { return !/^S\d+/.test(im.title || im.name || ''); }).map(function (im) { var title = im.title || im.name || im.entity_id; var meta = (im.content && im.content.alt) || ''; var color = pickColor(meta, title); return { name: title, role: '', look: meta, personality: '', colorText: '', outfit: meta, color: color, accent: accentFor(color, title), robot: isRobot(meta) }; }); }
  function generateCharacters(prompt, inputs) {
    var all = flatten(inputs); var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed); var texts = all.filter(function (i) { return i.type_id === 'buckyos.richtext'; });
    var table = tables.filter(isCharacterTable)[0]; var chars = table ? parseCharacters(table) : []; var style = styleLabel(texts, prompt); var results = [], warnings = [];
    if (chars.length === 0) warnings.push('角色表中没有可用的角色名');
    chars.forEach(function (c, i) { results.push({ name: c.name, type: 'image', renderer: 'asset', title: c.name, size: { w: 220, h: 303 }, position: { x: i * 236, y: 0 }, content: { svg: characterCard(c, style), width: 480, height: 600, alt: c.role + '，' + c.look + (c.colorText ? '，主色' + c.colorText : '') + (c.outfit ? '，' + c.outfit : ''), caption: c.role } }); });
    var lines = ['## 角色设定说明（模拟）', '风格：' + style + '。以下 ' + chars.length + ' 张设定图由角色表逐行生成，修改表格中的外貌 / 主色 / 服装后重新运行即可刷新。'];
    chars.forEach(function (c) { lines.push('- ' + c.name + '（' + c.role + '）：' + (c.look || '—') + (c.colorText ? '；主色 ' + c.colorText : '') + (c.outfit ? '；' + c.outfit : '')); });
    results.push({ name: '设定说明', type: 'richtext', renderer: 'richtext', title: '设定说明（模拟）', size: { w: Math.max(460, chars.length * 236 - 16), h: 150 }, position: { x: 0, y: 320 }, content: { markdown: lines.join('\n') } });
    return { results: results, warnings: warnings, assumptions: ['每个角色一张竖版设定图（480×600）', '主色取自"主色"列，缺失时按名字推断'], summary: '角色设定图 ×' + chars.length + ' · ' + style };
  }
  var SHOT_CYCLE = ['远景', '中景', '特写', '全景', '中景', '近景', '远景', '中景'];
  function parseScript(text, names) {
    var shots = []; var numbered = text.split('\n').map(function (l) { return /^\s*(\d+)[.、．)]\s*(.+)$/.exec(l); }).filter(Boolean);
    function pushShot(setting, action, explicit) { var no = shots.length + 1; var mentioned = names.filter(function (n) { return action.indexOf(n) >= 0 || setting.indexOf(n) >= 0; }); var chars = explicit.length ? explicit : mentioned; var shotType = SHOT_CYCLE[(no - 1) % SHOT_CYCLE.length]; if (chars.length === 0 && (shotType === '特写' || shotType === '近景')) shotType = '全景'; if (chars.length > 2 && shotType === '特写') shotType = '中景'; shots.push({ no: no, setting: setting.trim() || '场景 ' + no, action: action.trim(), chars: chars, shotType: shotType, seconds: 4 + (hash(action) % 3) }); }
    if (numbered.length) numbered.forEach(function (m) { var body = m[2].trim(); var explicit = []; var paren = /[（(]([^()（）]+)[)）]\s*$/.exec(body); if (paren) { explicit = paren[1].split(/[、,，\/\s]+/).filter(Boolean); body = body.slice(0, paren.index).trim(); } var sep = body.search(/[:：]/); pushShot(sep >= 0 ? body.slice(0, sep) : '', sep >= 0 ? body.slice(sep + 1) : body, explicit); });
    else text.split('\n').map(function (l) { return l.replace(/^#+\s*/, '').trim(); }).filter(function (l) { return l && !/^一句话|^logline/i.test(l); }).join('。').split(/[。！？!?]/).map(function (s) { return s.trim(); }).filter(function (s) { return s.length > 4; }).slice(0, 6).forEach(function (s) { pushShot('', s, []); });
    return shots.slice(0, 8);
  }
  function sceneKind(setting) { if (/日出|黎明|清晨|早晨/.test(setting)) return 'dawn'; if (/面馆|餐|店|室内|屋内|房间|厨房/.test(setting)) return 'interior'; if (/街|市|巷/.test(setting)) return 'street'; if (/桥|高架|公路/.test(setting)) return 'bridge'; if (/塔|顶层|云端/.test(setting)) return 'tower'; if (/天台|楼顶|屋顶/.test(setting)) return 'rooftop'; return 'generic'; }
  function storyboardFrame(shot, chars, style) {
    var seed = hash(shot.no + '|' + shot.setting + '|' + shot.action + '|' + style); var r = rng(seed); var kind = sceneKind(shot.setting); var rain = /雨/.test(shot.setting + shot.action); var night = /夜|晚/.test(shot.setting);
    var palette = { dawn: ['#1e1b4b', '#f97316', '#fde68a'], interior: ['#3b2a1a', '#b45309', '#fbbf24'], street: ['#0f172a', '#312e81', '#22d3ee'], bridge: ['#111827', '#1f2937', '#a5b4fc'], tower: ['#0c1a3a', '#1d4ed8', '#e0f2fe'], rooftop: ['#0b1020', '#1e3a8a', '#93c5fd'], generic: ['#111827', '#334155', '#cbd5e1'] }[kind];
    var c0 = palette[0], c1 = palette[1], acc = palette[2]; var p = [];
    p.push('<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="' + c0 + '"/><stop offset="1" stop-color="' + c1 + '"/></linearGradient></defs><rect width="640" height="360" fill="url(#sky)"/>');
    if (kind === 'dawn') p.push('<circle cx="' + (320 + Math.round(r() * 120 - 60)) + '" cy="230" r="46" fill="#fbbf24" opacity="0.9"/><rect x="0" y="250" width="640" height="110" fill="#1e1b4b" opacity="0.6"/>');
    if (kind === 'interior') { for (var i = 0; i < 4; i++) { var x = 90 + i * 150; p.push('<circle cx="' + x + '" cy="82" r="14" fill="#fbbf24" opacity="0.9"/><circle cx="' + x + '" cy="82" r="40" fill="#fbbf24" opacity="0.12"/>'); } p.push('<rect x="0" y="270" width="640" height="90" fill="#451a03" opacity="0.9"/>'); }
    if (kind === 'street') { for (var j = 0; j < 9; j++) { var sx = Math.round(r() * 640), h = Math.round(80 + r() * 180), col = j % 2 ? '#22d3ee' : '#e879f9'; p.push('<rect x="' + sx + '" y="' + (300 - h) + '" width="' + (6 + Math.round(r() * 10)) + '" height="' + h + '" fill="' + col + '" opacity="0.7"/>'); } p.push('<rect x="0" y="300" width="640" height="60" fill="#020617" opacity="0.8"/>'); }
    if (kind === 'bridge') { p.push('<polygon points="0,360 640,360 420,250 220,250" fill="#0f172a" opacity="0.9"/>'); for (var k = 0; k < 8; k++) p.push('<line x1="' + (k * 90) + '" y1="0" x2="320" y2="250" stroke="#94a3b8" stroke-width="1.5" opacity="0.4"/>'); }
    if (kind === 'tower') { p.push('<polygon points="320,20 350,300 290,300" fill="#0f172a" opacity="0.85"/><circle cx="320" cy="18" r="5" fill="#fca5a5"/>'); }
    if (kind === 'rooftop' || kind === 'generic' || kind === 'dawn') { for (var m = 0; m < 14; m++) { var w = 20 + Math.round(r() * 50), hh = 40 + Math.round(r() * 150), bx = Math.round(r() * 640); p.push('<rect x="' + bx + '" y="' + (300 - hh) + '" width="' + w + '" height="' + hh + '" fill="#020617" opacity="' + (0.5 + r() * 0.4).toFixed(2) + '"/>'); } if (kind === 'rooftop') p.push('<rect x="0" y="296" width="640" height="64" fill="#0f172a"/><line x1="0" y1="296" x2="640" y2="296" stroke="' + acc + '" stroke-width="2" opacity="0.6"/>'); if (night && kind !== 'dawn') p.push('<circle cx="' + (560 + Math.round(r() * 40)) + '" cy="' + (50 + Math.round(r() * 30)) + '" r="22" fill="#fef3c7" opacity="0.85"/>'); }
    var scale = shot.shotType === '远景' ? 0.55 : shot.shotType === '全景' ? 0.8 : shot.shotType === '中景' ? 1.15 : shot.shotType === '近景' ? 1.6 : 2.4; var baseY = shot.shotType === '特写' ? 640 : shot.shotType === '近景' ? 470 : shot.shotType === '中景' ? 400 : 330;
    var cast = shot.chars.map(function (n) { return chars.filter(function (c) { return c.name === n; })[0] || { name: n, role: '', look: '', personality: '', colorText: '', outfit: '', color: '#94a3b8', accent: '#e2e8f0', robot: isRobot(n) }; }); var shown = shot.shotType === '特写' ? cast.slice(0, 1) : cast;
    shown.forEach(function (c, i) { p.push(figure(Math.round((640 * (i + 1)) / (shown.length + 1)), baseY, scale, c, hash(c.name))); });
    if (rain) for (var q = 0; q < 70; q++) { var rx = Math.round(r() * 640), ry = Math.round(r() * 360); p.push('<line x1="' + rx + '" y1="' + ry + '" x2="' + (rx - 6) + '" y2="' + (ry + 18) + '" stroke="#bae6fd" stroke-width="1" opacity="0.45"/>'); }
    p.push('<rect x="0" y="0" width="640" height="24" fill="#000" opacity="0.75"/><rect x="0" y="336" width="640" height="24" fill="#000" opacity="0.75"/><rect x="10" y="30" width="118" height="22" rx="11" fill="#000" opacity="0.55"/><text x="20" y="45" font-size="12" font-weight="700" fill="#fff" ' + FONT + '>S' + String(shot.no).padStart(2, '0') + ' · ' + esc(shot.shotType) + ' · ' + shot.seconds + 's</text><text x="630" y="45" text-anchor="end" font-size="11" fill="#fff" opacity="0.7" ' + FONT + '>故事板 · Mock</text>');
    var subtitle = shot.action.length > 38 ? shot.action.slice(0, 37) + '…' : shot.action; p.push('<rect x="0" y="300" width="640" height="36" fill="#000" opacity="0.45"/><text x="320" y="324" text-anchor="middle" font-size="14" fill="#fff" ' + FONT + '>' + esc(subtitle) + '</text>');
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360">' + p.join('') + '</svg>';
  }
  function generateStoryboard(prompt, inputs) {
    var all = flatten(inputs); var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed); var texts = all.filter(function (i) { return i.type_id === 'buckyos.richtext'; }); var images = all.filter(function (i) { return i.type_id === 'buckyos.asset-ref'; });
    var style = styleLabel(texts, prompt); var charTable = tables.filter(isCharacterTable)[0]; var chars = charTable ? parseCharacters(charTable) : charactersFromImages(images);
    var script = texts.filter(function (t) { return /剧本|大纲|故事|script/i.test(t.title || t.name || ''); })[0] || texts.filter(function (t) { return !/风格|style/i.test(t.title || t.name || ''); })[0];
    var warnings = []; if (!script) warnings.push('未找到剧本文本，改用许愿格中的目标生成镜头'); if (chars.length === 0) warnings.push('未找到角色设定，人物以占位剪影表示');
    var shots = parseScript(script ? script.content.text : prompt, chars.map(function (c) { return c.name; }));
    if (shots.length === 0) return { results: [{ name: '说明', type: 'richtext', renderer: 'richtext', title: '说明', content: { markdown: '剧本里没有可识别的分场。请用 "1. 场景：动作（角色）" 的格式写分场后重新运行。' } }], warnings: warnings.concat(['剧本没有分场']), assumptions: [], summary: '故事板：缺少分场' };
    var results = []; var cols = 3, fw = 288, fh = 190;
    results.push({ name: '分镜表', type: 'table', renderer: 'table', title: '分镜表（模拟）', size: { w: cols * fw + 32, h: 28 * (shots.length + 1) + 60 }, position: { x: 0, y: 0 },
      content: { fields: [{ field_id: 'shot', name: '镜号', type: 'text' }, { field_id: 'setting', name: '场景', type: 'text' }, { field_id: 'shot_type', name: '景别', type: 'text' }, { field_id: 'chars', name: '出场角色', type: 'text' }, { field_id: 'action', name: '动作描述', type: 'text' }, { field_id: 'seconds', name: '时长(秒)', type: 'number' }],
        rows: shots.map(function (s) { return { shot: 'S' + String(s.no).padStart(2, '0'), setting: s.setting, shot_type: s.shotType, chars: s.chars.join('、') || '—', action: s.action, seconds: s.seconds }; }) } });
    var tableHeight = 28 * (shots.length + 1) + 60;
    shots.forEach(function (s, i) { results.push({ name: 'S' + String(s.no).padStart(2, '0'), type: 'image', renderer: 'asset', title: 'S' + String(s.no).padStart(2, '0') + ' · ' + s.shotType, size: { w: fw, h: fh }, position: { x: (i % cols) * (fw + 16), y: tableHeight + 20 + Math.floor(i / cols) * (fh + 16) }, content: { svg: storyboardFrame(s, chars, style), width: 640, height: 360, alt: s.action, caption: s.setting + ' · ' + s.shotType } }); });
    var total = shots.reduce(function (n, s) { return n + s.seconds; }, 0);
    return { results: results, warnings: warnings, assumptions: ['每个分场对应一个镜头，景别按 远景→中景→特写 循环', '时长 4–6 秒由动作描述推断'], summary: '故事板 ' + shots.length + ' 个镜头 · 约 ' + total + ' 秒 · ' + style };
  }
  function generateVideo(prompt, inputs) {
    var all = flatten(inputs); var images = all.filter(function (i) { return i.type_id === 'buckyos.asset-ref' && i.content && i.content.svg; }).sort(function (a, b) { return String(a.title || a.name).localeCompare(String(b.title || b.name), 'zh-CN', { numeric: true }); });
    var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed); var table = tables.filter(function (t) { return !!findCol(t.columns, '时长', 'duration'); })[0];
    var durationCol = table ? findCol(table.columns, '时长', 'duration') : undefined, actionCol = table ? findCol(table.columns, '动作', '描述', 'action') : undefined, noCol = table ? findCol(table.columns, '镜号', 'shot') : undefined; var warnings = [];
    if (images.length === 0) return { results: [{ name: '说明', type: 'richtext', renderer: 'richtext', title: '说明', content: { markdown: '数据来源里没有可用的画面。请先生成故事板，再把故事板结果作为来源。' } }], warnings: ['没有画面来源'], assumptions: [], summary: '视频：缺少画面' };
    var frames = images.map(function (im, i) { var title = im.title || im.name || ''; var row = (table && noCol) ? table.rows.filter(function (r) { return String(r[noCol] || '') && title.indexOf(String(r[noCol])) === 0; })[0] : (table ? table.rows[i] : undefined); var secs = row && durationCol ? Number(row[durationCol]) : NaN; var caption = row && actionCol ? String(row[actionCol] || '') : (im.content.alt || title); return { svg: im.content.svg, durationMs: (isFinite(secs) && secs > 0 ? secs : 4) * 1000, caption: title.split(' · ')[0] + ' · ' + caption }; });
    if (!table) warnings.push('未找到分镜表，每个镜头按 4 秒计'); var total = frames.reduce(function (n, f) { return n + f.durationMs; }, 0);
    var results = [
      { name: '成片预览', type: 'video', renderer: 'sample.video', title: '最终视频（逐帧预览，模拟）', size: { w: 640, h: 452 }, position: { x: 0, y: 0 }, content: { frames: frames, width: 640, height: 360, caption: '逐帧预览 · ' + frames.length + ' 个镜头 · ' + (total / 1000).toFixed(0) + ' 秒' } },
      { name: '成片指标', type: 'record', renderer: 'record', title: '成片指标（模拟）', size: { w: 230, h: 150 }, position: { x: 656, y: 0 }, content: { schema: { properties: [{ key: 'duration', name: '总时长(秒)', type: 'number' }, { key: 'shots', name: '镜头数', type: 'number' }] }, props: { duration: total / 1000, shots: frames.length } } },
      { name: '导出说明', type: 'richtext', renderer: 'richtext', title: '导出说明（模拟）', size: { w: 230, h: 200 }, position: { x: 656, y: 170 }, content: { markdown: '## 成片说明\n- 分辨率 1280×720 · 16:9\n- 镜头顺序与时长来自分镜表\n- Mock 只输出逐帧预览；接入真实视频模型后，这里会是可下载的 MP4，其余流程不变。' } }
    ];
    return { results: results, warnings: warnings, assumptions: ['镜头顺序按故事板编号', '时长取自分镜表"时长(秒)"列'], summary: '最终视频预览：' + frames.length + ' 个镜头，' + (total / 1000).toFixed(0) + ' 秒' };
  }
  function generic(prompt, inputs) {
    var all = flatten(inputs); var lines = ['# 模拟结果', '提示词：' + prompt.replace(/#\w+/g, '').trim(), '读取的输入：' + (all.map(function (i) { return (i.title || i.name || i.entity_id) + '（' + i.type_id + '）'; }).join('、') || '无')];
    all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).forEach(function (t) { lines.push('- 表「' + (t.title || t.name) + '」共 ' + (t.content.rows || []).length + ' 行，' + (t.content.fields || []).length + ' 列'); });
    all.filter(function (i) { return i.type_id === 'buckyos.richtext'; }).forEach(function (t) { lines.push('- 文本「' + (t.title || t.name) + '」约 ' + (t.content.text || '').length + ' 字'); });
    return { results: [{ name: '摘要', type: 'richtext', renderer: 'richtext', title: '模拟摘要', size: { w: 480, h: 220 }, content: { markdown: lines.join('\n') } }], warnings: ['模拟执行器没有识别出专用任务：只生成了输入摘要'], assumptions: [], summary: '输入摘要' };
  }

  aiws.on('execute', function (req) {
    var prompt = String(req.prompt || ''); var inputs = req.inputs || [];
    var slow = /#slow/.test(prompt) ? 1500 : 0;
    return new Promise(function (resolve, reject) {
      setTimeout(function () {
        try {
          if (/#fail/.test(prompt)) throw new Error('模拟失败：提示词包含 #fail（按注入规则失败，旧结果应保持不变）');
          var kind = classify(prompt, inputs); var out;
          if (kind === 'sales') out = sales(flatten(inputs).filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed).filter(isSalesTable)[0], prompt);
          else if (kind === 'characters') out = generateCharacters(prompt, inputs);
          else if (kind === 'storyboard') out = generateStoryboard(prompt, inputs);
          else if (kind === 'video') out = generateVideo(prompt, inputs);
          else out = generic(prompt, inputs);
          if (/#invalid/.test(prompt) && out.results.length) { out.results[0].type = 'record'; out.results[0].renderer = 'record'; out.results[0].content = { schema: { properties: [{ key: 'n', name: '数', type: 'number' }] }, props: { n: 'not-a-number' } }; }
          out.kind = kind; out.simulated = true;
          resolve(out);
        } catch (e) { reject(e); }
      }, slow);
    });
  });

  // a visible instance (the wish Block activated) only shows what it is
  document.getElementById('status').textContent = '模拟执行器已就绪：' + (aiws.context.title || '') + '（按宿主请求执行分析与生成，不调用任何模型）';
  aiws.ready();
})();`

export const MOCK_WISH_SOURCE = {
  html: '<div class="mock"><b>Mock 许愿格执行器</b><div id="status">初始化…</div></div>',
  css: 'body{margin:0;font:12px system-ui,sans-serif;color:#334155;background:#f8fafc}.mock{padding:8px}',
  js: MOCK_JS,
  api_version: 2,
}

/** The payload of the `buckyos.block-def` entity that carries the Mock executor. */
export function mockWishDefPayload(): Record<string, unknown> {
  return {
    def_id: MOCK_WISH_RENDERER, version: 1, kind: 'html', title: 'Mock 许愿格执行器',
    description: '假的许愿格：交互与真许愿格一致，分析与执行的结果按固定规则从输入构造。所有输出标明"模拟"。',
    accepts: ['buckyos.wish'], allow_no_source: false, default_size: { w: 420, h: 260 },
    html: MOCK_WISH_SOURCE,
  }
}

/** The commit operation creating the definition entity under `data` (idempotent by id). */
export function mockWishDefOp(orderKey: string) {
  return { op: 'entity.create', entity_id: MOCK_WISH_DEF_ID, type_id: 'buckyos.block-def', parent_id: 'data', order_key: orderKey, name: 'mock-wish', payload: mockWishDefPayload() }
}
