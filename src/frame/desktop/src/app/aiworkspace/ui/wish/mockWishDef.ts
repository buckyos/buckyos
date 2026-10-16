/* The Mock wish executor (phase two §7.6, D8): a fake wish with the real wish's interaction. It is an
 * HTML extension definition — saved as a `buckyos.block-def` entity (D9) — whose script answers the
 * host's `analyze` and `execute` requests deterministically from the inputs it is given. Nothing here
 * calls a model; every output says it is simulated. The generators are ported from the ai-canvas
 * prototype (`canvas/agent/mock.ts`, `canvas/agent/aigc.ts`): quarterly sales analysis, and the
 * three-stage short-film workflow (character sheets → storyboard → frame preview). SVG images are
 * returned as markup; the host uploads them as assets (§7.6).
 *
 * The script's text travels with it in every language AI Workspace has (the `aiworkspace.wish.mock.*`
 * keys, JSON-injected when the payload is built); the script picks the host's language (`aiws.locale`)
 * when it starts, so its output follows the viewer, not the language the definition was created in.
 * The words it looks for in the user's data (column names, titles, prompts) are matched in both.
 *
 * Failure injection (deterministic): a prompt containing `#fail` makes execution throw; `#invalid`
 * returns a candidate the backend must refuse; `#slow` delays execution. */

import { aiwsT, aiwsText } from '../../i18n'

export const MOCK_WISH_DEF_ID = 'def-mock-wish'
export const MOCK_WISH_RENDERER = 'buckyos.mock-wish'

/** The script's text by the name it uses (`T(name, vars)`). */
const MOCK_TEXT: Record<string, string> = {
  listSep: 'aiworkspace.common.listSeparator',
  clauseSep: 'aiworkspace.common.clauseSeparator',
  commaSep: 'aiworkspace.wish.commaSeparator',
  none: 'aiworkspace.common.none',
  nameWithType: 'aiworkspace.wish.nameWithType',
  title: 'aiworkspace.wish.mock.title',
  ready: 'aiworkspace.wish.mock.ready',
  failInjected: 'aiworkspace.wish.mock.failInjected',
  invalidProp: 'aiworkspace.wish.mock.invalidProp',
  noInputsWarning: 'aiworkspace.wish.mock.noInputsWarning',
  inputName: 'aiworkspace.wish.mock.inputName',
  noInputs: 'aiworkspace.wish.mock.noInputs',
  context: 'aiworkspace.wish.mock.context',
  summaryName: 'aiworkspace.wish.mock.summaryName',
  noteName: 'aiworkspace.wish.mock.noteName',
  revenue: 'aiworkspace.wish.mock.revenue',
  target: 'aiworkspace.wish.mock.target',
  growth: 'aiworkspace.wish.mock.growth',
  completion: 'aiworkspace.wish.mock.completion',
  region: 'aiworkspace.wish.mock.region',
  assumeMargin: 'aiworkspace.wish.mock.assumeMargin',
  assumeCompletion: 'aiworkspace.wish.mock.assumeCompletion',
  noCost: 'aiworkspace.wish.mock.noCost',
  noTarget: 'aiworkspace.wish.mock.noTarget',
  metricsName: 'aiworkspace.wish.mock.metricsName',
  metricsTitle: 'aiworkspace.wish.mock.metricsTitle',
  revenueThisQuarter: 'aiworkspace.wish.mock.revenueThisQuarter',
  overallMargin: 'aiworkspace.wish.mock.overallMargin',
  regionsName: 'aiworkspace.wish.mock.regionsName',
  regionsTitle: 'aiworkspace.wish.mock.regionsTitle',
  anomaliesName: 'aiworkspace.wish.mock.anomaliesName',
  anomaliesTitle: 'aiworkspace.wish.mock.anomaliesTitle',
  item: 'aiworkspace.wish.mock.item',
  issue: 'aiworkspace.wish.mock.issue',
  issueNegativeMargin: 'aiworkspace.wish.mock.issueNegativeMargin',
  issueFarBelowTarget: 'aiworkspace.wish.mock.issueFarBelowTarget',
  issueDecline: 'aiworkspace.wish.mock.issueDecline',
  salesHeading: 'aiworkspace.wish.mock.salesHeading',
  salesTotal: 'aiworkspace.wish.mock.salesTotal',
  salesGrowthUp: 'aiworkspace.wish.mock.salesGrowthUp',
  salesGrowthDown: 'aiworkspace.wish.mock.salesGrowthDown',
  salesMargin: 'aiworkspace.wish.mock.salesMargin',
  salesCompletion: 'aiworkspace.wish.mock.salesCompletion',
  salesBestRegion: 'aiworkspace.wish.mock.salesBestRegion',
  salesTopItems: 'aiworkspace.wish.mock.salesTopItems',
  salesTopItem: 'aiworkspace.wish.mock.salesTopItem',
  salesAnomalies: 'aiworkspace.wish.mock.salesAnomalies',
  salesNoAnomalies: 'aiworkspace.wish.mock.salesNoAnomalies',
  salesAdvice: 'aiworkspace.wish.mock.salesAdvice',
  salesSummaryTitle: 'aiworkspace.wish.mock.salesSummaryTitle',
  salesSummary: 'aiworkspace.wish.mock.salesSummary',
  defaultStyle: 'aiworkspace.wish.mock.defaultStyle',
  mainColor: 'aiworkspace.wish.mock.mainColor',
  altColor: 'aiworkspace.wish.mock.altColor',
  cardHeader: 'aiworkspace.wish.mock.cardHeader',
  cardBadge: 'aiworkspace.wish.mock.cardBadge',
  roleDefault: 'aiworkspace.wish.mock.roleDefault',
  noCharacterNames: 'aiworkspace.wish.mock.noCharacterNames',
  charactersHeading: 'aiworkspace.wish.mock.charactersHeading',
  charactersIntro: 'aiworkspace.wish.mock.charactersIntro',
  characterLine: 'aiworkspace.wish.mock.characterLine',
  characterNotesName: 'aiworkspace.wish.mock.characterNotesName',
  characterNotesTitle: 'aiworkspace.wish.mock.characterNotesTitle',
  assumeOneSheet: 'aiworkspace.wish.mock.assumeOneSheet',
  assumeMainColor: 'aiworkspace.wish.mock.assumeMainColor',
  charactersSummary: 'aiworkspace.wish.mock.charactersSummary',
  shotLong: 'aiworkspace.wish.mock.shotLong',
  shotFull: 'aiworkspace.wish.mock.shotFull',
  shotMedium: 'aiworkspace.wish.mock.shotMedium',
  shotClose: 'aiworkspace.wish.mock.shotClose',
  shotCloseup: 'aiworkspace.wish.mock.shotCloseup',
  sceneN: 'aiworkspace.wish.mock.sceneN',
  frameBadge: 'aiworkspace.wish.mock.frameBadge',
  noScript: 'aiworkspace.wish.mock.noScript',
  noCharacters: 'aiworkspace.wish.mock.noCharacters',
  noScenes: 'aiworkspace.wish.mock.noScenes',
  scriptNoScenes: 'aiworkspace.wish.mock.scriptNoScenes',
  storyboardNoScenes: 'aiworkspace.wish.mock.storyboardNoScenes',
  shotListName: 'aiworkspace.wish.mock.shotListName',
  shotListTitle: 'aiworkspace.wish.mock.shotListTitle',
  colShot: 'aiworkspace.wish.mock.colShot',
  colSetting: 'aiworkspace.wish.mock.colSetting',
  colShotType: 'aiworkspace.wish.mock.colShotType',
  colChars: 'aiworkspace.wish.mock.colChars',
  colAction: 'aiworkspace.wish.mock.colAction',
  colSeconds: 'aiworkspace.wish.mock.colSeconds',
  assumeShotCycle: 'aiworkspace.wish.mock.assumeShotCycle',
  assumeDuration: 'aiworkspace.wish.mock.assumeDuration',
  storyboardSummary: 'aiworkspace.wish.mock.storyboardSummary',
  noFrames: 'aiworkspace.wish.mock.noFrames',
  noFrameSource: 'aiworkspace.wish.mock.noFrameSource',
  videoNoFrames: 'aiworkspace.wish.mock.videoNoFrames',
  noShotList: 'aiworkspace.wish.mock.noShotList',
  videoName: 'aiworkspace.wish.mock.videoName',
  videoTitle: 'aiworkspace.wish.mock.videoTitle',
  videoCaption: 'aiworkspace.wish.mock.videoCaption',
  videoMetricsName: 'aiworkspace.wish.mock.videoMetricsName',
  videoMetricsTitle: 'aiworkspace.wish.mock.videoMetricsTitle',
  totalDuration: 'aiworkspace.wish.mock.totalDuration',
  shotCount: 'aiworkspace.wish.mock.shotCount',
  exportNotesName: 'aiworkspace.wish.mock.exportNotesName',
  exportNotesTitle: 'aiworkspace.wish.mock.exportNotesTitle',
  exportNotes: 'aiworkspace.wish.mock.exportNotes',
  assumeShotOrder: 'aiworkspace.wish.mock.assumeShotOrder',
  assumeDurationColumn: 'aiworkspace.wish.mock.assumeDurationColumn',
  videoSummary: 'aiworkspace.wish.mock.videoSummary',
  genericHeading: 'aiworkspace.wish.mock.genericHeading',
  genericPrompt: 'aiworkspace.wish.mock.genericPrompt',
  genericInputs: 'aiworkspace.wish.mock.genericInputs',
  genericTable: 'aiworkspace.wish.mock.genericTable',
  genericText: 'aiworkspace.wish.mock.genericText',
  genericTitle: 'aiworkspace.wish.mock.genericTitle',
  genericWarning: 'aiworkspace.wish.mock.genericWarning',
  genericSummary: 'aiworkspace.wish.mock.genericSummary',
}

/** The languages AI Workspace has tables for; any other host language reads English (as the host does). */
const MOCK_LOCALES = ['en', 'zh-CN']

/** `{ <locale>: { <name>: template } }` for the script; a `<name>_one` entry is the English singular for `count: 1`. */
function mockText(): Record<string, Record<string, string>> {
  const out: Record<string, Record<string, string>> = {}
  for (const locale of MOCK_LOCALES) {
    const text = aiwsText(locale)
    const table: Record<string, string> = {}
    for (const [name, key] of Object.entries(MOCK_TEXT)) {
      table[name] = text.template(key)
      const one = text.template(key, 1)
      if (one !== table[name]) table[`${name}_one`] = one
    }
    out[locale] = table
  }
  return out
}

// i18n-exempt: words and patterns the Mock looks for in the user's data (column names, titles, prompts, script lines) — matched, never shown
const MOCK_MATCH = String.raw`
  var W = {
    charName: ['角色', '人物', '姓名', '名字', 'name', 'character'],
    charTrait: ['外貌', '外形', '描述', '定位', '性格', '主色', 'look', 'role', 'appearance', 'personality'],
    role: ['定位', '身份', '职业', 'role'], look: ['外貌', '外形', '描述', 'look', 'appearance'], personality: ['性格', 'personality'],
    color: ['主色', '颜色', '色调', 'color', 'colour'], outfit: ['服装', '道具', '穿着', '装扮', 'outfit', 'props', 'costume'],
    revenue: ['销售额', '营收', '收入', 'revenue', 'sales'], cost: ['成本', 'cost'], target: ['目标', 'target'],
    prev: ['上季度', '上期', '去年', 'last quarter', 'previous', 'prior', 'last year'],
    region: ['区域', '地区', '大区', 'region', 'area'], product: ['产品', '品类', '商品', 'product', 'category'],
    frameCols: ['镜号', '时长', 'shot'], duration: ['时长', 'duration'], action: ['动作', '描述', 'action'], shotNo: ['镜号', 'shot'],
    scriptTitle: /剧本|大纲|故事|script/i,
    wantsVideo: /视频|成片|合成|剪辑|video|render/i, wantsStoryboard: /故事板|分镜|storyboard/i, wantsCharacters: /角色|人物|立绘|形象|设定图|character/i,
    colors: [[/青|cyan|teal/i, '#22d3ee'], [/蓝|blue|navy/i, '#3b82f6'], [/紫|purple|violet/i, '#a855f7'], [/朱|红|red|crimson/i, '#ef4444'], [/橙|orange/i, '#f97316'], [/琥珀|amber|金|gold|黄|yellow/i, '#f59e0b'], [/绿|green/i, '#22c55e'], [/粉|pink|magenta/i, '#ec4899'], [/白|white|银|silver/i, '#e5e7eb'], [/黑|black|炭/i, '#334155'], [/灰|gray|grey/i, '#94a3b8'], [/棕|褐|brown/i, '#a16207']],
    robot: /机器人|机械|(?<![a-z])AI(?![a-z])|android|robot|droid/i,
    styleTitle: /风格|style/i, styleLine: /风格[:：]|style\s*[:：]/i, stylePrefix: /^风格[:：]\s*|^style\s*[:：]\s*/i, styleHeading: /^风格|^视觉风格$|^(visual\s+)?style$/i,
    apron: /围裙|apron/i, bag: /背包|包|bag|backpack/i, glasses: /眼镜|护目镜|glasses|goggles/i, headset: /耳机|耳麦|headphone|headset|earphone/i, umbrella: /伞|umbrella/i,
    listSplit: /[、,，\/\s]+/, castSplit: /[、,，\/]+/, numbered: /^\s*(\d+)[.、．)]\s*(.+)$/, paren: /[（(]([^()（）]+)[)）]\s*$/, colon: /[:：]/, logline: /^一句话|^logline/i,
    sentenceJoin: '。', sentenceSplit: /[。！？!?]/,
    dawn: /日出|黎明|清晨|早晨|\b(sunrise|dawn|daybreak|morning)/i, interior: /面馆|餐|店|室内|屋内|房间|厨房|\b(restaurant|noodle|diner|cafe|shop|indoor|kitchen|room)/i,
    street: /街|市|巷|\b(street|market|alley)/i, bridge: /桥|高架|公路|\b(bridge|overpass|highway)/i, tower: /塔|顶层|云端|\b(tower|summit)/i, rooftop: /天台|楼顶|屋顶|\broof/i,
    rain: /雨|\brain/i, night: /夜|晚|\b(night|evening)/i
  };`

const MOCK_JS = String.raw`
(function () {
  'use strict';
  // ---- text in the host's language: every language travels with the script, aiws.locale picks one
  var TEXT = __TEXT__;
  var LOCALE = TEXT[aiws.locale] ? aiws.locale : 'en';
  var S = TEXT[LOCALE];
  function T(name, vars) { var s = (vars && vars.count === 1 && S[name + '_one']) || S[name]; return vars ? s.replace(/\{\{(\w+)\}\}/g, function (w, k) { return k in vars ? String(vars[k]) : w; }) : s; }
  var SHOT_LABEL = { long: 'shotLong', full: 'shotFull', medium: 'shotMedium', close: 'shotClose', closeup: 'shotCloseup' };
  function shotLabel(id) { return T(SHOT_LABEL[id]); }
__MATCH__
  // ---- deterministic helpers (FNV-1a + mulberry32)
  function hash(text) { var h = 0x811c9dc5; for (var i = 0; i < text.length; i++) { h ^= text.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; } return h >>> 0; }
  function rng(seed) { var a = seed >>> 0; return function () { a = (a + 0x6d2b79f5) >>> 0; var t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
  function esc(s) { return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'); }
  function num(v) { if (typeof v === 'number') return v; if (typeof v === 'string') { var n = Number(v.replace(/[,¥$%\s]/g, '')); return isFinite(n) ? n : 0; } return 0; }
  function money(n) { return '¥' + Math.round(n).toLocaleString(LOCALE); }
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
  function findCol(cols, names) { for (var i = 0; i < names.length; i++) { var n = names[i]; var hit = cols.filter(function (c) { return c === n; })[0] || cols.filter(function (c) { return String(c).toLowerCase().indexOf(String(n).toLowerCase()) >= 0; })[0]; if (hit) return hit; } return undefined; }
  function isCharacterTable(t) { return !!findCol(t.columns, W.charName) && !!findCol(t.columns, W.charTrait); }
  function isSalesTable(t) { return !!findCol(t.columns, W.revenue); }
  function classify(prompt, inputs) {
    var all = flatten(inputs);
    var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed);
    var images = all.filter(function (i) { return i.type_id === 'buckyos.asset-ref'; });
    var texts = all.filter(function (i) { return i.type_id === 'buckyos.richtext'; });
    var charTable = tables.filter(isCharacterTable)[0];
    var sales = tables.filter(isSalesTable)[0];
    var hasFrames = images.some(function (im) { return /^S\d+/.test(im.title || im.name || ''); }) || tables.some(function (t) { return !!findCol(t.columns, W.frameCols); });
    var hasCharImages = images.some(function (im) { return !/^S\d+/.test(im.title || im.name || ''); });
    var hasScript = texts.some(function (t) { return W.scriptTitle.test(t.title || t.name || ''); });
    var wantsVideo = W.wantsVideo.test(prompt);
    var wantsStoryboard = W.wantsStoryboard.test(prompt);
    var wantsCharacters = W.wantsCharacters.test(prompt);
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
      if (picked.length === 0) warnings.push(T('noInputsWarning'));
    }
    var names = picked.map(function (p) { return T('inputName', { label: p.label || p.entity_id, id: p.entity_id }); }).join(T('listSep'));
    var context = T('context', { inputs: names || T('noInputs'), prompt: prompt.replace(/#\w+/g, '').trim() });
    return { context_prompt: context, inputs: picked, warnings: warnings };
  });

  // ---- sales analysis (PRD §13.6 port)
  function groupBy(rows, key) { var m = {}; rows.forEach(function (r) { var k = String(r[key] == null ? '' : r[key]); (m[k] = m[k] || []).push(r); }); return m; }
  function sum(rows, key) { return rows.reduce(function (a, r) { return a + num(r[key]); }, 0); }
  function sales(table, prompt) {
    var cols = table.columns;
    var rev = findCol(cols, W.revenue); var cost = findCol(cols, W.cost); var target = findCol(cols, W.target); var prev = findCol(cols, W.prev);
    var region = findCol(cols, W.region); var product = findCol(cols, W.product);
    var rows = table.rows;
    var totalRev = sum(rows, rev), totalCost = cost ? sum(rows, cost) : 0, totalTarget = target ? sum(rows, target) : 0, totalPrev = prev ? sum(rows, prev) : 0;
    var margin = totalRev ? (totalRev - totalCost) / totalRev : 0, completion = totalTarget ? totalRev / totalTarget : 0, growth = totalPrev ? (totalRev - totalPrev) / totalPrev : 0;
    var warnings = [], assumptions = [T('assumeMargin'), T('assumeCompletion')];
    if (!cost) warnings.push(T('noCost')); if (!target) warnings.push(T('noTarget'));
    var results = [];
    results.push({ name: T('metricsName'), type: 'record', renderer: 'record', title: T('metricsTitle'), size: { w: 320, h: 170 },
      content: { schema: { properties: [{ key: 'revenue', name: T('revenueThisQuarter'), type: 'decimal', scale: 2 }, { key: 'margin', name: T('overallMargin'), type: 'text' }, { key: 'completion', name: T('completion'), type: 'text' }, { key: 'growth', name: T('growth'), type: 'text' }] },
        props: { revenue: totalRev.toFixed(2), margin: pct(margin), completion: target ? pct(completion) : '—', growth: prev ? pct(growth) : '—' } } });
    var regionRows = [];
    if (region) { var byRegion = groupBy(rows, region); Object.keys(byRegion).forEach(function (name) { var rs = byRegion[name]; regionRows.push({ region: name, revenue: sum(rs, rev), target: target ? sum(rs, target) : 0, growth: prev ? (sum(rs, prev) ? (sum(rs, rev) - sum(rs, prev)) / sum(rs, prev) : 0) : 0 }); }); }
    if (region) results.push({ name: T('regionsName'), type: 'table', renderer: 'sample.bar-chart', title: T('regionsTitle'), size: { w: 420, h: 260 }, config: { value: 'revenue', by: 'region' },
      content: { fields: [{ field_id: 'region', name: T('region'), type: 'text' }, { field_id: 'revenue', name: T('revenue'), type: 'number' }, { field_id: 'target', name: T('target'), type: 'number' }, { field_id: 'growth', name: T('growth'), type: 'text' }],
        rows: regionRows.map(function (r) { return { region: r.region, revenue: Math.round(r.revenue), target: Math.round(r.target), growth: pct(r.growth) }; }) } });
    var ranked = rows.map(function (r) { var p = prev ? num(r[prev]) : 0; var g = p ? (num(r[rev]) - p) / p : 0; return { label: [region ? r[region] : null, product ? r[product] : null].filter(Boolean).join(' · '), growth: g, rev: num(r[rev]), cost: cost ? num(r[cost]) : 0, target: target ? num(r[target]) : 0 }; }).sort(function (a, b) { return b.growth - a.growth; });
    var anomalies = ranked.filter(function (x) { return (cost && x.cost > x.rev) || (target && x.target && x.rev / x.target < 0.7) || x.growth < -0.1; });
    if (anomalies.length) results.push({ name: T('anomaliesName'), type: 'table', renderer: 'table', title: T('anomaliesTitle'), size: { w: 520, h: 220 },
      content: { fields: [{ field_id: 'item', name: T('item'), type: 'text' }, { field_id: 'revenue', name: T('revenue'), type: 'number' }, { field_id: 'completion', name: T('completion'), type: 'text' }, { field_id: 'growth', name: T('growth'), type: 'text' }, { field_id: 'issue', name: T('issue'), type: 'text' }],
        rows: anomalies.map(function (a) { var issues = []; if (cost && a.cost > a.rev) issues.push(T('issueNegativeMargin')); if (target && a.target && a.rev / a.target < 0.7) issues.push(T('issueFarBelowTarget')); if (a.growth < -0.1) issues.push(T('issueDecline')); return { item: a.label, revenue: Math.round(a.rev), completion: a.target ? pct(a.rev / a.target) : '—', growth: pct(a.growth), issue: issues.join(T('listSep')) }; }) } });
    var best = regionRows.slice().sort(function (a, b) { return b.growth - a.growth; });
    var lines = ['# ' + T('salesHeading'),
      T('salesTotal', { total: money(totalRev), growth: prev ? T(growth >= 0 ? 'salesGrowthUp' : 'salesGrowthDown', { pct: pct(Math.abs(growth)) }) : '', margin: cost ? T('salesMargin', { pct: pct(margin) }) : '', completion: target ? T('salesCompletion', { pct: pct(completion) }) : '' })];
    if (best.length) lines.push('- ' + T('salesBestRegion', { best: best[0].region, bestGrowth: pct(best[0].growth), worst: best[best.length - 1].region, worstGrowth: pct(best[best.length - 1].growth) }));
    if (ranked.length) lines.push('- ' + T('salesTopItems', { items: ranked.slice(0, 3).map(function (t) { return T('salesTopItem', { label: t.label, pct: pct(t.growth) }); }).join(T('listSep')) }));
    lines.push('- ' + (anomalies.length ? T('salesAnomalies', { count: anomalies.length }) : T('salesNoAnomalies')));
    lines.push(T('salesAdvice'));
    if (/#invalid/.test(prompt)) results[0].content.props.revenue = 'not-a-number';
    results.push({ name: T('summaryName'), type: 'richtext', renderer: 'richtext', title: T('salesSummaryTitle'), size: { w: 520, h: 260 }, content: { markdown: lines.join('\n') } });
    return { results: results, warnings: warnings, assumptions: assumptions, summary: T('salesSummary', { revenue: money(totalRev), margin: pct(margin) }) };
  }

  // ---- AIGC: colours, figures, cards, storyboard frames (ported, deterministic)
  var COLOR_WORDS = W.colors;
  function pickColor(text, seed) { for (var i = 0; i < COLOR_WORDS.length; i++) if (COLOR_WORDS[i][0].test(text)) return COLOR_WORDS[i][1]; return 'hsl(' + (hash(seed) % 360) + ' 70% 60%)'; }
  function accentFor(color, seed) { var h = (hash(seed) % 360) + 180; return color.indexOf('hsl') === 0 ? 'hsl(' + (h % 360) + ' 80% 70%)' : ['#fde68a', '#a5f3fc', '#f9a8d4', '#bbf7d0'][hash(seed) % 4]; }
  function isRobot(text) { return W.robot.test(text); }
  function styleLabel(texts, prompt) {
    var styleText = texts.filter(function (t) { return W.styleTitle.test(t.title || t.name || '') || W.styleLine.test(t.content.text || ''); })[0];
    var raw = styleText ? styleText.content.text : prompt;
    var line = raw.split('\n').map(function (l) { return l.replace(/^#+\s*/, '').replace(W.stylePrefix, '').trim(); }).filter(function (l) { return l.length > 0 && !W.styleHeading.test(l); })[0];
    return (line || T('defaultStyle')).slice(0, 22);
  }
  function figure(x, baseY, s, c, seed) {
    var r = rng(seed); var skin = ['#f1d3b3', '#e8c39e', '#d9a679', '#c68a5b'][seed % 4]; var hair = ['#111827', '#3f2a1d', '#7c2d12', '#1e293b', '#4c1d95'][seed % 5]; var p = [];
    p.push('<ellipse cx="' + x + '" cy="' + baseY + '" rx="' + 44 * s + '" ry="' + 8 * s + '" fill="#000" opacity="0.35"/>');
    p.push('<path d="M' + (x - 40 * s) + ',' + baseY + ' L' + (x - 34 * s) + ',' + (baseY - 130 * s) + ' Q' + x + ',' + (baseY - 150 * s) + ' ' + (x + 34 * s) + ',' + (baseY - 130 * s) + ' L' + (x + 40 * s) + ',' + baseY + ' Z" fill="' + c.color + '"/>');
    p.push('<rect x="' + (x - 6 * s) + '" y="' + (baseY - 132 * s) + '" width="' + 12 * s + '" height="' + 62 * s + '" rx="' + 4 * s + '" fill="' + c.accent + '" opacity="0.85"/>');
    if (W.apron.test(c.outfit)) p.push('<rect x="' + (x - 26 * s) + '" y="' + (baseY - 100 * s) + '" width="' + 52 * s + '" height="' + 90 * s + '" rx="' + 6 * s + '" fill="#f8fafc" opacity="0.85"/>');
    if (W.bag.test(c.outfit)) p.push('<rect x="' + (x - 58 * s) + '" y="' + (baseY - 125 * s) + '" width="' + 24 * s + '" height="' + 60 * s + '" rx="' + 6 * s + '" fill="' + c.accent + '"/>');
    if (c.robot) { var hy = baseY - 190 * s; p.push('<rect x="' + (x - 26 * s) + '" y="' + hy + '" width="' + 52 * s + '" height="' + 46 * s + '" rx="' + 10 * s + '" fill="#cbd5e1"/><circle cx="' + x + '" cy="' + (hy - 20 * s) + '" r="' + 5 * s + '" fill="' + c.accent + '"/><rect x="' + (x - 17 * s) + '" y="' + (hy + 16 * s) + '" width="' + 12 * s + '" height="' + 8 * s + '" fill="' + c.accent + '"/><rect x="' + (x + 5 * s) + '" y="' + (hy + 16 * s) + '" width="' + 12 * s + '" height="' + 8 * s + '" fill="' + c.accent + '"/>'); }
    else { var cy = baseY - 165 * s; p.push('<circle cx="' + x + '" cy="' + cy + '" r="' + 28 * s + '" fill="' + skin + '"/><path d="M' + (x - 29 * s) + ',' + (cy - 4 * s) + ' Q' + x + ',' + (cy - 44 * s) + ' ' + (x + 29 * s) + ',' + (cy - 4 * s) + ' Q' + (x + 20 * s) + ',' + (cy - 22 * s) + ' ' + x + ',' + (cy - 26 * s) + ' Q' + (x - 20 * s) + ',' + (cy - 22 * s) + ' ' + (x - 29 * s) + ',' + (cy - 4 * s) + ' Z" fill="' + hair + '"/><circle cx="' + (x - 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 2.5 * s + '" fill="#111827"/><circle cx="' + (x + 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 2.5 * s + '" fill="#111827"/>');
      if (W.glasses.test(c.outfit)) p.push('<circle cx="' + (x - 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 8 * s + '" fill="none" stroke="' + c.accent + '" stroke-width="' + 2 * s + '"/><circle cx="' + (x + 10 * s) + '" cy="' + (cy + 2 * s) + '" r="' + 8 * s + '" fill="none" stroke="' + c.accent + '" stroke-width="' + 2 * s + '"/>');
      if (W.headset.test(c.outfit)) p.push('<path d="M' + (x - 30 * s) + ',' + cy + ' A' + 30 * s + ',' + 30 * s + ' 0 0 1 ' + (x + 30 * s) + ',' + cy + '" fill="none" stroke="' + c.accent + '" stroke-width="' + 4 * s + '"/>'); }
    if (W.umbrella.test(c.outfit)) { var ux = x + 50 * s, uy = baseY - 200 * s; p.push('<path d="M' + (ux - 60 * s) + ',' + (uy + 30 * s) + ' A' + 60 * s + ',' + 60 * s + ' 0 0 1 ' + (ux + 60 * s) + ',' + (uy + 30 * s) + ' Z" fill="' + c.accent + '" opacity="0.9"/><line x1="' + ux + '" y1="' + (uy + 30 * s) + '" x2="' + ux + '" y2="' + (baseY - 60 * s) + '" stroke="#e2e8f0" stroke-width="' + 3 * s + '"/>'); }
    if (r() > 0.5) p.push('<circle cx="' + (x + 30 * s) + '" cy="' + (baseY - 110 * s) + '" r="' + 5 * s + '" fill="' + c.accent + '" opacity="0.7"/>');
    return p.join('');
  }
  function characterCard(c, style) {
    var seed = hash(c.name + '|' + c.look + '|' + style); var r = rng(seed); var deco = [];
    for (var i = 0; i < 7; i++) { var cx = Math.round(r() * 480), cy = Math.round(r() * 420), rad = Math.round(10 + r() * 60); deco.push('<circle cx="' + cx + '" cy="' + cy + '" r="' + rad + '" fill="' + (i % 2 ? c.accent : c.color) + '" opacity="' + (0.05 + r() * 0.12).toFixed(2) + '"/>'); }
    var traits = c.personality.split(W.listSplit).filter(Boolean).slice(0, 3);
    var chips = traits.map(function (t, i) { var w = t.length * 13 + 18, x = 24 + i * (w + 8); return '<rect x="' + x + '" y="562" width="' + w + '" height="24" rx="12" fill="#fff" opacity="0.14"/><text x="' + (x + w / 2) + '" y="579" text-anchor="middle" font-size="12" fill="#f8fafc" ' + FONT + '>' + esc(t) + '</text>'; }).join('');
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 600" width="480" height="600"><defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0b1020"/><stop offset="1" stop-color="' + c.color + '" stop-opacity="0.55"/></linearGradient><radialGradient id="glow"><stop offset="0" stop-color="' + c.accent + '" stop-opacity="0.5"/><stop offset="1" stop-color="' + c.accent + '" stop-opacity="0"/></radialGradient></defs><rect width="480" height="600" fill="#0b1020"/><rect width="480" height="600" fill="url(#bg)"/><circle cx="240" cy="260" r="220" fill="url(#glow)"/>' + deco.join('') + figure(240, 470, 1.55, c, seed) + '<rect x="0" y="484" width="480" height="116" fill="#020617" opacity="0.55"/><text x="24" y="522" font-size="32" font-weight="700" fill="#f8fafc" ' + FONT + '>' + esc(c.name) + '</text><text x="24" y="548" font-size="15" fill="' + c.accent + '" ' + FONT + '>' + esc(c.role) + (c.colorText ? ' · ' + esc(T('mainColor', { color: c.colorText })) : '') + '</text>' + chips + '<text x="20" y="30" font-size="12" fill="#e2e8f0" opacity="0.8" ' + FONT + '>' + esc(T('cardHeader', { style: style })) + '</text><text x="460" y="30" text-anchor="end" font-size="11" fill="#e2e8f0" opacity="0.6" ' + FONT + '>' + esc(T('cardBadge')) + '</text></svg>';
  }
  function parseCharacters(table) {
    var cols = table.columns; var name = findCol(cols, W.charName), role = findCol(cols, W.role), look = findCol(cols, W.look), personality = findCol(cols, W.personality), color = findCol(cols, W.color), outfit = findCol(cols, W.outfit);
    function str(row, col) { return col && row[col] != null ? String(row[col]) : ''; }
    return table.rows.map(function (row) { var n = str(row, name).trim(); if (!n) return null; var all = [str(row, role), str(row, look), str(row, outfit)].join(' '); var colorText = str(row, color); var c = pickColor(colorText || all, n);
      return { name: n, role: str(row, role) || T('roleDefault'), look: str(row, look), personality: str(row, personality), colorText: colorText, outfit: str(row, outfit), color: c, accent: accentFor(c, n), robot: isRobot(all) }; }).filter(Boolean).slice(0, 6);
  }
  function charactersFromImages(images) { return images.filter(function (im) { return !/^S\d+/.test(im.title || im.name || ''); }).map(function (im) { var title = im.title || im.name || im.entity_id; var meta = (im.content && im.content.alt) || ''; var color = pickColor(meta, title); return { name: title, role: '', look: meta, personality: '', colorText: '', outfit: meta, color: color, accent: accentFor(color, title), robot: isRobot(meta) }; }); }
  function generateCharacters(prompt, inputs) {
    var all = flatten(inputs); var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed); var texts = all.filter(function (i) { return i.type_id === 'buckyos.richtext'; });
    var table = tables.filter(isCharacterTable)[0]; var chars = table ? parseCharacters(table) : []; var style = styleLabel(texts, prompt); var results = [], warnings = [];
    if (chars.length === 0) warnings.push(T('noCharacterNames'));
    chars.forEach(function (c, i) { results.push({ name: c.name, type: 'image', renderer: 'asset', title: c.name, size: { w: 220, h: 303 }, position: { x: i * 236, y: 0 }, content: { svg: characterCard(c, style), width: 480, height: 600, alt: [c.role, c.look].concat(c.colorText ? [T('altColor', { color: c.colorText })] : [], c.outfit ? [c.outfit] : []).join(T('commaSep')), caption: c.role } }); });
    var lines = ['## ' + T('charactersHeading'), T('charactersIntro', { style: style, count: chars.length })];
    chars.forEach(function (c) { lines.push('- ' + T('characterLine', { name: c.name, role: c.role, details: [c.look || '—'].concat(c.colorText ? [T('mainColor', { color: c.colorText })] : [], c.outfit ? [c.outfit] : []).join(T('clauseSep')) })); });
    results.push({ name: T('characterNotesName'), type: 'richtext', renderer: 'richtext', title: T('characterNotesTitle'), size: { w: Math.max(460, chars.length * 236 - 16), h: 150 }, position: { x: 0, y: 320 }, content: { markdown: lines.join('\n') } });
    return { results: results, warnings: warnings, assumptions: [T('assumeOneSheet'), T('assumeMainColor')], summary: T('charactersSummary', { count: chars.length, style: style }) };
  }
  var SHOT_CYCLE = ['long', 'medium', 'closeup', 'full', 'medium', 'close', 'long', 'medium'];
  // the characters named in a scene: a known name may contain spaces ("Old Zhou"); anything else splits at them too
  function castOf(text, names) { return text.split(W.castSplit).map(function (s) { return s.trim(); }).filter(Boolean).reduce(function (out, s) { return out.concat(names.indexOf(s) >= 0 ? [s] : s.split(/\s+/)); }, []).filter(Boolean); }
  function parseScript(text, names) {
    var shots = []; var numbered = text.split('\n').map(function (l) { return W.numbered.exec(l); }).filter(Boolean);
    function pushShot(setting, action, explicit) { var no = shots.length + 1; var mentioned = names.filter(function (n) { return action.indexOf(n) >= 0 || setting.indexOf(n) >= 0; }); var chars = explicit.length ? explicit : mentioned; var shotType = SHOT_CYCLE[(no - 1) % SHOT_CYCLE.length]; if (chars.length === 0 && (shotType === 'closeup' || shotType === 'close')) shotType = 'full'; if (chars.length > 2 && shotType === 'closeup') shotType = 'medium'; shots.push({ no: no, setting: setting.trim() || T('sceneN', { no: no }), action: action.trim(), chars: chars, shotType: shotType, seconds: 4 + (hash(action) % 3) }); }
    if (numbered.length) numbered.forEach(function (m) { var body = m[2].trim(); var explicit = []; var paren = W.paren.exec(body); if (paren) { explicit = castOf(paren[1], names); body = body.slice(0, paren.index).trim(); } var sep = body.search(W.colon); pushShot(sep >= 0 ? body.slice(0, sep) : '', sep >= 0 ? body.slice(sep + 1) : body, explicit); });
    else text.split('\n').map(function (l) { return l.replace(/^#+\s*/, '').trim(); }).filter(function (l) { return l && !W.logline.test(l); }).join(W.sentenceJoin).split(W.sentenceSplit).map(function (s) { return s.trim(); }).filter(function (s) { return s.length > 4; }).slice(0, 6).forEach(function (s) { pushShot('', s, []); });
    return shots.slice(0, 8);
  }
  function sceneKind(setting) { if (W.dawn.test(setting)) return 'dawn'; if (W.interior.test(setting)) return 'interior'; if (W.street.test(setting)) return 'street'; if (W.bridge.test(setting)) return 'bridge'; if (W.tower.test(setting)) return 'tower'; if (W.rooftop.test(setting)) return 'rooftop'; return 'generic'; }
  function storyboardFrame(shot, chars, style) {
    var seed = hash(shot.no + '|' + shot.setting + '|' + shot.action + '|' + style); var r = rng(seed); var kind = sceneKind(shot.setting); var rain = W.rain.test(shot.setting + shot.action); var night = W.night.test(shot.setting);
    var palette = { dawn: ['#1e1b4b', '#f97316', '#fde68a'], interior: ['#3b2a1a', '#b45309', '#fbbf24'], street: ['#0f172a', '#312e81', '#22d3ee'], bridge: ['#111827', '#1f2937', '#a5b4fc'], tower: ['#0c1a3a', '#1d4ed8', '#e0f2fe'], rooftop: ['#0b1020', '#1e3a8a', '#93c5fd'], generic: ['#111827', '#334155', '#cbd5e1'] }[kind];
    var c0 = palette[0], c1 = palette[1], acc = palette[2]; var p = [];
    p.push('<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="' + c0 + '"/><stop offset="1" stop-color="' + c1 + '"/></linearGradient></defs><rect width="640" height="360" fill="url(#sky)"/>');
    if (kind === 'dawn') p.push('<circle cx="' + (320 + Math.round(r() * 120 - 60)) + '" cy="230" r="46" fill="#fbbf24" opacity="0.9"/><rect x="0" y="250" width="640" height="110" fill="#1e1b4b" opacity="0.6"/>');
    if (kind === 'interior') { for (var i = 0; i < 4; i++) { var x = 90 + i * 150; p.push('<circle cx="' + x + '" cy="82" r="14" fill="#fbbf24" opacity="0.9"/><circle cx="' + x + '" cy="82" r="40" fill="#fbbf24" opacity="0.12"/>'); } p.push('<rect x="0" y="270" width="640" height="90" fill="#451a03" opacity="0.9"/>'); }
    if (kind === 'street') { for (var j = 0; j < 9; j++) { var sx = Math.round(r() * 640), h = Math.round(80 + r() * 180), col = j % 2 ? '#22d3ee' : '#e879f9'; p.push('<rect x="' + sx + '" y="' + (300 - h) + '" width="' + (6 + Math.round(r() * 10)) + '" height="' + h + '" fill="' + col + '" opacity="0.7"/>'); } p.push('<rect x="0" y="300" width="640" height="60" fill="#020617" opacity="0.8"/>'); }
    if (kind === 'bridge') { p.push('<polygon points="0,360 640,360 420,250 220,250" fill="#0f172a" opacity="0.9"/>'); for (var k = 0; k < 8; k++) p.push('<line x1="' + (k * 90) + '" y1="0" x2="320" y2="250" stroke="#94a3b8" stroke-width="1.5" opacity="0.4"/>'); }
    if (kind === 'tower') { p.push('<polygon points="320,20 350,300 290,300" fill="#0f172a" opacity="0.85"/><circle cx="320" cy="18" r="5" fill="#fca5a5"/>'); }
    if (kind === 'rooftop' || kind === 'generic' || kind === 'dawn') { for (var m = 0; m < 14; m++) { var w = 20 + Math.round(r() * 50), hh = 40 + Math.round(r() * 150), bx = Math.round(r() * 640); p.push('<rect x="' + bx + '" y="' + (300 - hh) + '" width="' + w + '" height="' + hh + '" fill="#020617" opacity="' + (0.5 + r() * 0.4).toFixed(2) + '"/>'); } if (kind === 'rooftop') p.push('<rect x="0" y="296" width="640" height="64" fill="#0f172a"/><line x1="0" y1="296" x2="640" y2="296" stroke="' + acc + '" stroke-width="2" opacity="0.6"/>'); if (night && kind !== 'dawn') p.push('<circle cx="' + (560 + Math.round(r() * 40)) + '" cy="' + (50 + Math.round(r() * 30)) + '" r="22" fill="#fef3c7" opacity="0.85"/>'); }
    var scale = shot.shotType === 'long' ? 0.55 : shot.shotType === 'full' ? 0.8 : shot.shotType === 'medium' ? 1.15 : shot.shotType === 'close' ? 1.6 : 2.4; var baseY = shot.shotType === 'closeup' ? 640 : shot.shotType === 'close' ? 470 : shot.shotType === 'medium' ? 400 : 330;
    var cast = shot.chars.map(function (n) { return chars.filter(function (c) { return c.name === n; })[0] || { name: n, role: '', look: '', personality: '', colorText: '', outfit: '', color: '#94a3b8', accent: '#e2e8f0', robot: isRobot(n) }; }); var shown = shot.shotType === 'closeup' ? cast.slice(0, 1) : cast;
    shown.forEach(function (c, i) { p.push(figure(Math.round((640 * (i + 1)) / (shown.length + 1)), baseY, scale, c, hash(c.name))); });
    if (rain) for (var q = 0; q < 70; q++) { var rx = Math.round(r() * 640), ry = Math.round(r() * 360); p.push('<line x1="' + rx + '" y1="' + ry + '" x2="' + (rx - 6) + '" y2="' + (ry + 18) + '" stroke="#bae6fd" stroke-width="1" opacity="0.45"/>'); }
    p.push('<rect x="0" y="0" width="640" height="24" fill="#000" opacity="0.75"/><rect x="0" y="336" width="640" height="24" fill="#000" opacity="0.75"/><rect x="10" y="30" width="118" height="22" rx="11" fill="#000" opacity="0.55"/><text x="20" y="45" font-size="12" font-weight="700" fill="#fff" ' + FONT + '>S' + String(shot.no).padStart(2, '0') + ' · ' + esc(shotLabel(shot.shotType)) + ' · ' + shot.seconds + 's</text><text x="630" y="45" text-anchor="end" font-size="11" fill="#fff" opacity="0.7" ' + FONT + '>' + esc(T('frameBadge')) + '</text>');
    var subtitle = shot.action.length > 38 ? shot.action.slice(0, 37) + '…' : shot.action; p.push('<rect x="0" y="300" width="640" height="36" fill="#000" opacity="0.45"/><text x="320" y="324" text-anchor="middle" font-size="14" fill="#fff" ' + FONT + '>' + esc(subtitle) + '</text>');
    return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360">' + p.join('') + '</svg>';
  }
  function generateStoryboard(prompt, inputs) {
    var all = flatten(inputs); var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed); var texts = all.filter(function (i) { return i.type_id === 'buckyos.richtext'; }); var images = all.filter(function (i) { return i.type_id === 'buckyos.asset-ref'; });
    var style = styleLabel(texts, prompt); var charTable = tables.filter(isCharacterTable)[0]; var chars = charTable ? parseCharacters(charTable) : charactersFromImages(images);
    var script = texts.filter(function (t) { return W.scriptTitle.test(t.title || t.name || ''); })[0] || texts.filter(function (t) { return !W.styleTitle.test(t.title || t.name || ''); })[0];
    var warnings = []; if (!script) warnings.push(T('noScript')); if (chars.length === 0) warnings.push(T('noCharacters'));
    var shots = parseScript(script ? script.content.text : prompt, chars.map(function (c) { return c.name; }));
    if (shots.length === 0) return { results: [{ name: T('noteName'), type: 'richtext', renderer: 'richtext', title: T('noteName'), content: { markdown: T('noScenes') } }], warnings: warnings.concat([T('scriptNoScenes')]), assumptions: [], summary: T('storyboardNoScenes') };
    var results = []; var cols = 3, fw = 288, fh = 190;
    results.push({ name: T('shotListName'), type: 'table', renderer: 'table', title: T('shotListTitle'), size: { w: cols * fw + 32, h: 28 * (shots.length + 1) + 60 }, position: { x: 0, y: 0 },
      content: { fields: [{ field_id: 'shot', name: T('colShot'), type: 'text' }, { field_id: 'setting', name: T('colSetting'), type: 'text' }, { field_id: 'shot_type', name: T('colShotType'), type: 'text' }, { field_id: 'chars', name: T('colChars'), type: 'text' }, { field_id: 'action', name: T('colAction'), type: 'text' }, { field_id: 'seconds', name: T('colSeconds'), type: 'number' }],
        rows: shots.map(function (s) { return { shot: 'S' + String(s.no).padStart(2, '0'), setting: s.setting, shot_type: shotLabel(s.shotType), chars: s.chars.join(T('listSep')) || '—', action: s.action, seconds: s.seconds }; }) } });
    var tableHeight = 28 * (shots.length + 1) + 60;
    shots.forEach(function (s, i) { results.push({ name: 'S' + String(s.no).padStart(2, '0'), type: 'image', renderer: 'asset', title: 'S' + String(s.no).padStart(2, '0') + ' · ' + shotLabel(s.shotType), size: { w: fw, h: fh }, position: { x: (i % cols) * (fw + 16), y: tableHeight + 20 + Math.floor(i / cols) * (fh + 16) }, content: { svg: storyboardFrame(s, chars, style), width: 640, height: 360, alt: s.action, caption: s.setting + ' · ' + shotLabel(s.shotType) } }); });
    var total = shots.reduce(function (n, s) { return n + s.seconds; }, 0);
    return { results: results, warnings: warnings, assumptions: [T('assumeShotCycle'), T('assumeDuration')], summary: T('storyboardSummary', { count: shots.length, seconds: total, style: style }) };
  }
  function generateVideo(prompt, inputs) {
    var all = flatten(inputs); var images = all.filter(function (i) { return i.type_id === 'buckyos.asset-ref' && i.content && i.content.svg; }).sort(function (a, b) { return String(a.title || a.name).localeCompare(String(b.title || b.name), LOCALE, { numeric: true }); });
    var tables = all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed); var table = tables.filter(function (t) { return !!findCol(t.columns, W.duration); })[0];
    var durationCol = table ? findCol(table.columns, W.duration) : undefined, actionCol = table ? findCol(table.columns, W.action) : undefined, noCol = table ? findCol(table.columns, W.shotNo) : undefined; var warnings = [];
    if (images.length === 0) return { results: [{ name: T('noteName'), type: 'richtext', renderer: 'richtext', title: T('noteName'), content: { markdown: T('noFrames') } }], warnings: [T('noFrameSource')], assumptions: [], summary: T('videoNoFrames') };
    var frames = images.map(function (im, i) { var title = im.title || im.name || ''; var row = (table && noCol) ? table.rows.filter(function (r) { return String(r[noCol] || '') && title.indexOf(String(r[noCol])) === 0; })[0] : (table ? table.rows[i] : undefined); var secs = row && durationCol ? Number(row[durationCol]) : NaN; var caption = row && actionCol ? String(row[actionCol] || '') : (im.content.alt || title); return { svg: im.content.svg, durationMs: (isFinite(secs) && secs > 0 ? secs : 4) * 1000, caption: title.split(' · ')[0] + ' · ' + caption }; });
    if (!table) warnings.push(T('noShotList')); var total = frames.reduce(function (n, f) { return n + f.durationMs; }, 0);
    var results = [
      { name: T('videoName'), type: 'video', renderer: 'sample.video', title: T('videoTitle'), size: { w: 640, h: 452 }, position: { x: 0, y: 0 }, content: { frames: frames, width: 640, height: 360, caption: T('videoCaption', { count: frames.length, seconds: (total / 1000).toFixed(0) }) } },
      { name: T('videoMetricsName'), type: 'record', renderer: 'record', title: T('videoMetricsTitle'), size: { w: 230, h: 150 }, position: { x: 656, y: 0 }, content: { schema: { properties: [{ key: 'duration', name: T('totalDuration'), type: 'number' }, { key: 'shots', name: T('shotCount'), type: 'number' }] }, props: { duration: total / 1000, shots: frames.length } } },
      { name: T('exportNotesName'), type: 'richtext', renderer: 'richtext', title: T('exportNotesTitle'), size: { w: 230, h: 200 }, position: { x: 656, y: 170 }, content: { markdown: T('exportNotes') } }
    ];
    return { results: results, warnings: warnings, assumptions: [T('assumeShotOrder'), T('assumeDurationColumn')], summary: T('videoSummary', { count: frames.length, seconds: (total / 1000).toFixed(0) }) };
  }
  function generic(prompt, inputs) {
    var all = flatten(inputs); var lines = ['# ' + T('genericHeading'), T('genericPrompt', { prompt: prompt.replace(/#\w+/g, '').trim() }), T('genericInputs', { inputs: all.map(function (i) { return T('nameWithType', { name: i.title || i.name || i.entity_id, type: i.type_id }); }).join(T('listSep')) || T('none') })];
    all.filter(function (i) { return i.type_id === 'buckyos.table-source'; }).forEach(function (t) { var rows = (t.content.rows || []).length; lines.push('- ' + T('genericTable', { name: t.title || t.name, rows: rows, columns: (t.content.fields || []).length, count: rows })); });
    all.filter(function (i) { return i.type_id === 'buckyos.richtext'; }).forEach(function (t) { lines.push('- ' + T('genericText', { name: t.title || t.name, chars: (t.content.text || '').length })); });
    return { results: [{ name: T('summaryName'), type: 'richtext', renderer: 'richtext', title: T('genericTitle'), size: { w: 480, h: 220 }, content: { markdown: lines.join('\n') } }], warnings: [T('genericWarning')], assumptions: [], summary: T('genericSummary') };
  }

  aiws.on('execute', function (req) {
    var prompt = String(req.prompt || ''); var inputs = req.inputs || [];
    var slow = /#slow/.test(prompt) ? 1500 : 0;
    return new Promise(function (resolve, reject) {
      setTimeout(function () {
        try {
          if (/#fail/.test(prompt)) throw new Error(T('failInjected'));
          var kind = classify(prompt, inputs); var out;
          if (kind === 'sales') out = sales(flatten(inputs).filter(function (i) { return i.type_id === 'buckyos.table-source'; }).map(tableNamed).filter(isSalesTable)[0], prompt);
          else if (kind === 'characters') out = generateCharacters(prompt, inputs);
          else if (kind === 'storyboard') out = generateStoryboard(prompt, inputs);
          else if (kind === 'video') out = generateVideo(prompt, inputs);
          else out = generic(prompt, inputs);
          if (/#invalid/.test(prompt) && out.results.length) { out.results[0].type = 'record'; out.results[0].renderer = 'record'; out.results[0].content = { schema: { properties: [{ key: 'n', name: T('invalidProp'), type: 'number' }] }, props: { n: 'not-a-number' } }; }
          out.kind = kind; out.simulated = true;
          resolve(out);
        } catch (e) { reject(e); }
      }, slow);
    });
  });

  // a visible instance (the wish Block activated) only shows what it is
  var head = document.getElementById('mock-title'); if (head) head.textContent = T('title');
  document.getElementById('status').textContent = T('ready', { title: aiws.context.title || '' });
  aiws.ready();
})();`

/** The script with its text and match words in place. */
function mockScript(): string {
  const text = JSON.stringify(mockText())
  return MOCK_JS.replace('__TEXT__', () => text).replace('__MATCH__', () => MOCK_MATCH)
}

/** The definition's HTML source; its static text is in the current language (the script sets the viewer's). */
export const MOCK_WISH_SOURCE = {
  get html() { return `<div class="mock"><b id="mock-title">${aiwsT('aiworkspace.wish.mock.title')}</b><div id="status">${aiwsT('aiworkspace.wish.mock.initializing')}</div></div>` },
  css: 'body{margin:0;font:12px system-ui,sans-serif;color:#334155;background:#f8fafc}.mock{padding:8px}',
  get js() { return mockScript() },
  api_version: 2,
}

/** The payload of the `buckyos.block-def` entity that carries the Mock executor. */
export function mockWishDefPayload(): Record<string, unknown> {
  return {
    def_id: MOCK_WISH_RENDERER, version: 1, kind: 'html', title: aiwsT('aiworkspace.wish.mock.title'),
    description: aiwsT('aiworkspace.wish.mock.description'),
    accepts: ['buckyos.wish'], allow_no_source: false, default_size: { w: 420, h: 260 },
    html: { ...MOCK_WISH_SOURCE },
  }
}

/** The commit operation creating the definition entity under `data` (idempotent by id). */
export function mockWishDefOp(orderKey: string) {
  return { op: 'entity.create', entity_id: MOCK_WISH_DEF_ID, type_id: 'buckyos.block-def', parent_id: 'data', order_key: orderKey, name: 'mock-wish', payload: mockWishDefPayload() }
}
