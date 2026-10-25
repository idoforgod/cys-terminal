import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { discoverCdpTargets, observeCdpStage, cdpException } from './cdp-discovery.mjs';
const url = 'http://127.0.0.1:9333/json/list';
const run = (body, status = 200) => discoverCdpTargets(url, { fetchImpl: async () => new Response(body, { status, headers: { 'content-type': 'application/json' } }), now: () => '2026-10-25T00:00:00Z' });
for (const [label, body, reason] of [
  ['non-JSON', '<html>oops</html>', 'invalid_json'],
  ['object shape', '{}', 'invalid_target_shape'],
  ['null entry', '[null]', 'invalid_target_shape'],
  ['missing type', '[{}]', 'invalid_target_shape'],
  ['empty list', '[]', 'empty_target_list'],
  ['non-page', '[{"type":"worker"}]', 'no_page_target'],
  ['no debugger URL', '[{"type":"page"}]', 'page_without_debugger_url'],
  ['page', '[{"type":"page","webSocketDebuggerUrl":"ws://localhost/test"}]', 'page_discovered'],
]) test(label + ' has distinct persisted discovery reason', async () => {
  const { record } = await run(body);
  assert.equal(record.reason, reason);
  assert.equal(record.url, url); assert.equal(record.status, 200);
  assert.equal(record.content_type, 'application/json');
  assert.equal(record.started, record.finished);
  assert.deepEqual(JSON.parse(JSON.stringify(record)), record);
});
test('network exception preserves bounded cause and no HTTP status', async () => {
  const error = new TypeError('fetch failed', { cause: Object.assign(new Error('refused'), { code: 'ECONNREFUSED' }) });
  const { record, targets } = await discoverCdpTargets(url, { fetchImpl: async () => { throw error; } });
  assert.equal(record.reason, 'endpoint_unavailable'); assert.equal(record.status, null); assert.equal(targets, null);
  assert.equal(record.exception.name, 'TypeError'); assert.equal(record.exception.cause.code, 'ECONNREFUSED');
});
test('HTTP rejection is distinct even with valid list', async () => {
  const { record, targets } = await run('[]', 404);
  assert.equal(record.reason, 'http_error'); assert.equal(record.status, 404); assert.equal(targets, null);
});
test('body-read exception retains response status', async () => {
  const { record } = await discoverCdpTargets(url, { fetchImpl: async () => ({ status: 200, headers: new Headers(), text: async () => { throw new Error('stream ended'); } }) });
  assert.equal(record.reason, 'response_read_failed'); assert.equal(record.status, 200);
});
test('parse errors and body are bounded', async () => {
  const { record } = await run('x'.repeat(9000));
  assert.equal(record.body.length, 2048); assert.equal(record.body_truncated, true);
  assert.equal(record.parse_error.name, 'SyntaxError'); assert(record.parse_error.message.length <= 1024);
});
test('target counts distinguish non-pages and attachable pages', async () => {
  const { record } = await run(JSON.stringify([{ type:'worker' }, { type:'page' }, { type:'page', webSocketDebuggerUrl:'ws://local/test' }]));
  assert.equal(record.target_count, 3); assert.equal(record.page_count, 2); assert.equal(record.attachable_page_count, 1);
  assert.deepEqual(record.target_types, [{ type:'worker', count:1 }, { type:'page', count:2 }]);
});
test('cyclic exception causes terminate and text is bounded', () => {
  const e = new Error('x'.repeat(9000)); e.cause = e;
  const rec = cdpException(e); assert.equal(rec.message.length, 1024);
  assert.equal(rec.cause.cause.cause.cause_truncated, true);
});
test('production attach-stage wrapper records success and failure distinctly', async () => {
  const pass = await observeCdpStage('websocket_connect', async () => 42);
  assert.equal(pass.record.outcome, 'succeeded'); assert.equal(pass.value, 42); assert(pass.record.finished);
  const fail = await observeCdpStage('websocket_connect', async () => { throw new Error('handshake rejected'); });
  assert.equal(fail.record.outcome, 'failed'); assert.equal(fail.record.exception.message, 'handshake rejected'); assert(fail.record.finished);
});
test('vendor uses observed production helpers and preserves uipre stop', () => {
  const src = fs.readFileSync(new URL('./vendor/cdp-update.mjs', import.meta.url), 'utf8');
  assert(src.includes('await discoverCdpTargets(')); assert(src.includes("await observeCdpStage('websocket_connect'"));
  assert(src.includes("attachment.outcome = 'attached_tauri'"));
  assert(src.includes("MODE === 'uipre'"));
});
