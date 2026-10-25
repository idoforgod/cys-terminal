// Observer-only CDP diagnostics. No product calls, servers, or verdict decisions.
export const CDP_DIAGNOSTIC_LIMITS = Object.freeze({ text: 1024, body: 2048, causes: 3 });
const clip = (value, max = CDP_DIAGNOSTIC_LIMITS.text) => String(value ?? '').slice(0, max);
export function cdpException(error, depth = 0) {
  if (error == null) return null;
  return { name: clip(error.name || typeof error), message: clip(error.message ?? error),
    code: error.code == null ? null : clip(error.code),
    cause: depth < CDP_DIAGNOSTIC_LIMITS.causes ? cdpException(error.cause, depth + 1) : null,
    cause_truncated: depth >= CDP_DIAGNOSTIC_LIMITS.causes && error.cause != null };
}
export async function discoverCdpTargets(url, { fetchImpl = fetch, now = () => new Date().toISOString(), signal } = {}) {
  const record = { started: now(), finished: null, url: clip(url), status: null, content_type: null,
    body: null, body_truncated: false, parse_error: null, exception: null, target_count: null,
    target_types: null, page_count: null, attachable_page_count: null, reason: null };
  let targets = null;
  try {
    const response = await fetchImpl(url, { signal });
    record.status = response.status;
    record.content_type = clip(response.headers.get('content-type'));
    const body = await response.text();
    record.body = clip(body, CDP_DIAGNOSTIC_LIMITS.body);
    record.body_truncated = body.length > CDP_DIAGNOSTIC_LIMITS.body;
    let parsed;
    try { parsed = JSON.parse(body); }
    catch (e) { record.parse_error = cdpException(e); }
    if (!response.ok) record.reason = 'http_error';
    else if (record.parse_error) record.reason = 'invalid_json';
    else if (!Array.isArray(parsed) || parsed.some(t => !t || typeof t !== 'object' || Array.isArray(t) || typeof t.type !== 'string')) record.reason = 'invalid_target_shape';
    else {
      targets = parsed;
      record.target_count = targets.length;
      // Aggregate types, rather than retaining an unbounded copy of target metadata.
      const types = new Map();
      for (const t of targets) { const key = clip(t.type); types.set(key, (types.get(key) || 0) + 1); }
      record.target_types = [...types].slice(0, 32).map(([type, count]) => ({ type, count }));
      record.target_types_truncated = types.size > 32;
      record.page_count = targets.filter(t => t.type === 'page').length;
      record.attachable_page_count = targets.filter(t => t.type === 'page' && t.webSocketDebuggerUrl).length;
      record.reason = !targets.length ? 'empty_target_list' : !record.page_count ? 'no_page_target' : !record.attachable_page_count ? 'page_without_debugger_url' : 'page_discovered';
    }
  } catch (e) {
    record.exception = cdpException(e);
    // A response header was received if status is non-null: distinguish a body read failure.
    record.reason = record.status == null ? 'endpoint_unavailable' : 'response_read_failed';
  }
  record.finished = now();
  return { targets, record };
}
export async function observeCdpStage(stage, operation, { now = () => new Date().toISOString() } = {}) {
  const record = { stage, started: now(), finished: null, outcome: null, exception: null };
  try { const value = await operation(); record.outcome = 'succeeded'; return { value, record }; }
  catch (error) { record.outcome = 'failed'; record.exception = cdpException(error); return { error, record }; }
  finally { record.finished = now(); }
}
