import test from 'node:test';
import assert from 'node:assert/strict';
import { buildSuite, digest, repository } from './cases.mjs';
import { makeRequest, validateResponse, compare, callOnce, providers, limits, parseArgs, outputDirectory } from './evaluator.mjs';

const suite = buildSuite();
const request = makeRequest(suite, 'openrouter');
function responseFixture() {
  return {
    model: providers.openrouter.responseModel,
    answers: Object.fromEntries(suite.cases.map(item => [item.id, { type: 'choice', choice: item.expected,
      probabilities: Object.fromEntries(['supported', 'contradicted', 'insufficient'].map(label => [label, label === item.expected ? 0.9 : 0.05])), confidence: 0.8 }])),
    usage: { input_tokens: 100, output_tokens: 80, cost: 0.001 },
  };
}

test('archived evidence has verified hashes; samples distinguish observed and constructed', () => {
  assert.equal(suite.provenance.length, 3);
  assert.equal(suite.cases.length, 12);
  assert.deepEqual(suite.cases.filter(item => item.origin === 'observed').map(item => item.id), ['c01', 'c03', 'c05', 'c07']);
  assert.deepEqual(suite.cases.reduce((counts, item) => ({ ...counts, [item.expected]: counts[item.expected] + 1 }), { supported: 0, contradicted: 0, insufficient: 0 }),
    { supported: 6, contradicted: 5, insufficient: 1 });
  assert.equal(suite.cases.find(item => item.id === 'c01').observation.independentChecks.chartCount, 0);
  assert.equal(suite.cases.find(item => item.id === 'c07').observation.actualRequestCapture, null);
});

test('request excludes reference labels, rationale, sample names and provenance', () => {
  for (const item of suite.cases) {
    assert.deepEqual(Object.keys(request.state[item.id]), ['observation']);
    assert.ok(!JSON.stringify(request).includes(item.rationale));
    assert.ok(!JSON.stringify(request).includes(item.name));
  }
  const changedLabels = structuredClone(suite);
  for (const item of changedLabels.cases) { item.expected = 'insufficient'; item.rationale = 'REFERENCE_LABEL_LEAK'; }
  assert.equal(digest(JSON.stringify(makeRequest(changedLabels, 'openrouter'))), digest(JSON.stringify(request)));
});

test('fixture agreement is a comparison test, never a product verdict', () => {
  const result = compare(suite, validateResponse(responseFixture(), request, 'openrouter'));
  assert.equal(result.matched, 12);
  assert.equal(result.productVerdict, 'UNCHANGED');
  const raw = responseFixture();
  for (const id of ['c01', 'c07']) raw.answers[id] = { type: 'choice', choice: 'supported', probabilities: { supported: 1, contradicted: 0, insufficient: 0 }, confidence: 1 };
  const disagreement = compare(suite, validateResponse(raw, request, 'openrouter'));
  assert.deepEqual(disagreement.falseSupport, ['c01', 'c07']);
  assert.equal(disagreement.productVerdict, 'UNCHANGED');
});

const invalid = [
  ['missing answer', raw => { delete raw.answers.c01; }],
  ['extra answer', raw => { raw.answers.extra = raw.answers.c01; }],
  ['missing probability', raw => { delete raw.answers.c01.probabilities.insufficient; }],
  ['extra option', raw => { raw.answers.c01.probabilities.other = 0; }],
  ['negative probability', raw => { raw.answers.c01.probabilities.supported = -0.1; }],
  ['non-finite probability', raw => { raw.answers.c01.probabilities.supported = NaN; }],
  ['non-normalized probability', raw => { raw.answers.c01.probabilities.supported = 0.5; }],
  ['non-winning choice', raw => { raw.answers.c01.choice = 'supported'; }],
  ['model drift', raw => { raw.model = 'typesafe/jev-latest'; }],
  ['wrong answer type', raw => { raw.answers.c01.type = 'score'; }],
  ['invalid confidence', raw => { raw.answers.c01.confidence = 2; }],
  ['negative cost', raw => { raw.usage.cost = -1; }],
  ['invalid usage', raw => { raw.usage.input_tokens = '100'; }],
];
for (const [name, change] of invalid) test(`reject ${name} without repair`, () => {
  const raw = responseFixture(); change(raw);
  assert.throws(() => validateResponse(raw, request, 'openrouter'), /^Error: E_/);
});

test('one bounded request uses fixed endpoint and does not follow redirects', async () => {
  let count = 0;
  const answer = await callOnce(request, 'openrouter', 'fixture-secret', async (url, init) => {
    count += 1;
    assert.equal(url, providers.openrouter.url);
    assert.equal(init.redirect, 'error');
    assert.ok(init.signal instanceof AbortSignal);
    assert.equal(init.headers.authorization, 'Bearer fixture-secret');
    assert.deepEqual(JSON.parse(init.body), request);
    const raw = responseFixture(); raw.debug = 'fixture-secret';
    return new Response(JSON.stringify(raw));
  });
  assert.equal(count, 1);
  assert.ok(!JSON.stringify(answer).includes('fixture-secret'));
});

for (const status of [401, 429, 529]) test(`HTTP ${status} is evaluator error with no retry or response leak`, async () => {
  let count = 0;
  await assert.rejects(callOnce(request, 'openrouter', 'fixture-secret', async () => {
    count += 1;
    return new Response('fixture-secret', { status });
  }), new RegExp(`^Error: E_HTTP_${status}$`));
  assert.equal(count, 1);
});

test('transport exception redacts arbitrary diagnostics', async () => {
  await assert.rejects(callOnce(request, 'openrouter', 'fixture-secret', async () => { throw new Error('fixture-secret'); }), /^Error: E_TRANSPORT$/);
});

test('oversized request is rejected before any call', async () => {
  let calls = 0;
  const large = { ...request, state: 'x'.repeat(limits.requestBytes) };
  await assert.rejects(callOnce(large, 'openrouter', 'fixture-secret', async () => { calls += 1; }), /^Error: E_REQUEST_SIZE$/);
  assert.equal(calls, 0);
});

test('oversized or invalid JSON responses are evaluator errors', async () => {
  await assert.rejects(callOnce(request, 'openrouter', 'fixture-secret', async () => new Response('x'.repeat(limits.responseBytes + 1))), /^Error: E_RESPONSE_SIZE$/);
  await assert.rejects(callOnce(request, 'openrouter', 'fixture-secret', async () => new Response('fixture-secret')), /^Error: E_JSON$/);
});

test('network use is an explicit mode and never implied by supplying a key', () => {
  assert.throws(() => parseArgs(['prepare', '--provider', 'openrouter', '--out', 'x', '--config', 'x']), /^Error: E_ARGUMENT$/);
  assert.throws(() => parseArgs(['live', '--provider', 'openrouter', '--out', 'x']), /^Error: E_ARGUMENT$/);
  assert.equal(parseArgs(['prepare', '--provider', 'typesafe', '--out', 'x']).mode, 'prepare');
  assert.throws(() => makeRequest({ ...suite, cases: [...suite.cases, suite.cases[0]] }, 'openrouter'), /^Error: E_QUESTION_COUNT$/);
});

test('outputs outside the project scratch directory are rejected before writing', async () => {
  await assert.rejects(outputDirectory('/root/canvas-test-forbidden'), /^Error: E_OUTPUT_OUTSIDE_WORK$/);
});

test('direct TypeSafe request and response retain their own model identity', () => {
  const native = makeRequest(suite, 'typesafe');
  const raw = responseFixture(); raw.model = providers.typesafe.responseModel; delete raw.usage.cost;
  assert.equal(native.model, 'jev-1.13.0');
  assert.equal(validateResponse(raw, native, 'typesafe').usage.cost, null);
});

test('CLI prepares offline, rejects overwrite, replays bound evidence and rejects wrong request hash', async () => {
  const { mkdtemp, writeFile, readFile } = await import('node:fs/promises');
  const { resolve, join } = await import('node:path');
  const { spawnSync } = await import('node:child_process');
  const work = await mkdtemp(resolve(repository, '../.work/canvas-evaluator-selftest-'));
  const invoke = args => spawnSync(process.execPath, [resolve(repository, 'test/canvas_e2e/evaluator.mjs'), ...args], { cwd: work, encoding: 'utf8', timeout: 10000 });
  const prepared = join(work, 'prepared');
  const prepareArgs = ['prepare', '--provider', 'openrouter', '--out', prepared];
  assert.equal(invoke(prepareArgs).status, 0);
  assert.notEqual(invoke(prepareArgs).status, 0);
  const preparedReport = JSON.parse(await readFile(join(prepared, 'report.json'), 'utf8'));
  assert.equal(preparedReport.networkCalls, 0);
  assert.equal(preparedReport.liveEvaluation, 'NOT_RUN');
  const fixture = join(work, 'synthetic-response.json');
  await writeFile(fixture, JSON.stringify({ requestHash: digest(JSON.stringify(request)), response: responseFixture() }));
  const replayed = join(work, 'replayed');
  assert.equal(invoke(['replay', '--provider', 'openrouter', '--out', replayed, '--response', fixture]).status, 0);
  const replayReport = JSON.parse(await readFile(join(replayed, 'report.json'), 'utf8'));
  assert.equal(replayReport.networkCalls, 0);
  assert.equal(replayReport.status, 'REPLAYED');
  assert.equal(replayReport.liveEvaluation, 'NOT_RUN');
  assert.equal(replayReport.comparison.matched, 12);
  await writeFile(fixture, JSON.stringify({ requestHash: 'wrong-request', response: responseFixture() }));
  const rejected = join(work, 'rejected');
  assert.equal(invoke(['replay', '--provider', 'openrouter', '--out', rejected, '--response', fixture]).status, 1);
  const rejectedReport = JSON.parse(await readFile(join(rejected, 'report.json'), 'utf8'));
  assert.equal(rejectedReport.error, 'E_REPLAY_REQUEST');
  assert.equal(rejectedReport.productVerdict, 'UNCHANGED');
});
