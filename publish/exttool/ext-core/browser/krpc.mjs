// kRPC over HTTP POST, as buckyos-websdk's krpc_client.ts: { method, params, sys: [seq, token] } →
// { result, sys } or { error, sys }.

import http from 'node:http';
import https from 'node:https';

// Not `fetch`: it refuses the ports of its "bad port" list, and an Agent Service can be given one (10080).
function post(url, text, timeoutMs) {
  return new Promise((resolve, reject) => {
    const target = new URL(url);
    const request = (target.protocol === 'https:' ? https : http).request(
      target,
      { method: 'POST', headers: { 'content-type': 'application/json', 'content-length': Buffer.byteLength(text) } },
      (response) => {
        const chunks = [];
        response.on('data', (chunk) => chunks.push(chunk));
        response.on('end', () => resolve({ status: response.statusCode, text: Buffer.concat(chunks).toString('utf8') }));
        response.on('error', reject);
      },
    );
    request.setTimeout(timeoutMs, () => request.destroy(new Error(`no answer in ${timeoutMs / 1000}s`)));
    request.on('error', reject);
    request.end(text);
  });
}

export class RpcError extends Error {
  constructor(message, code) {
    super(message);
    this.code = code;
  }
}

// The Agent Service puts its error as JSON (`error_to_wire`) inside the `error` text; the browser module's errors
// carry a stable `code` there.
function wireError(body) {
  const text = String(body.error);
  let detail;
  const start = text.indexOf('{');
  if (start >= 0) {
    try {
      detail = JSON.parse(text.slice(start));
    } catch {}
  }
  return new RpcError(detail?.message ?? text, detail?.code ?? body.code ?? detail?.kind);
}

export class KrpcClient {
  constructor(url, token) {
    this.url = url;
    this.token = token;
    this.seq = Date.now();
  }

  async call(method, params = {}) {
    const seq = this.seq++;
    let response;
    try {
      response = await post(this.url, JSON.stringify({ method, params, sys: this.token ? [seq, this.token] : [seq] }), 60_000);
    } catch (e) {
      throw new RpcError(`cannot reach the Agent Service at ${this.url}: ${e.cause?.message ?? e.message}`, 'transport');
    }
    if (response.status < 200 || response.status >= 300) throw new RpcError(`the Agent Service answered HTTP ${response.status}`, 'http_error');
    let body;
    try {
      body = JSON.parse(response.text);
    } catch {
      throw new RpcError('the Agent Service sent an unreadable answer', 'transport');
    }
    if (body.error !== undefined && body.error !== null) throw wireError(body);
    return body.result;
  }
}
