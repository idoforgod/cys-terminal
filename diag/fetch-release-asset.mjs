// diag/fetch-release-asset.mjs
// Downloads ONE asset of a release of this repository, found by its tag name - also when the release is still a DRAFT.
// A draft has no tag endpoint (GET /releases/tags/<tag> answers 404 for it), so the release LIST is searched for tag_name == <tag>
// (drafts are listed only for a token with push access: the job gives GITHUB_TOKEN contents: write).
//
//   node diag/fetch-release-asset.mjs --tag vX.Y.Z --suffix _x64-setup.exe --out <folder> --report <json file>
//
// env: GITHUB_TOKEN (never printed), GITHUB_REPOSITORY, GITHUB_API_URL (optional, default https://api.github.com)
// steps: release list (per_page 50, at most 3 pages) -> the release with that tag_name (two or more: a draft is preferred, and the
//        report says so) -> its asset list -> EXACTLY ONE asset whose name ends with --suffix (0 or 2+ = failure, the names are printed)
//        -> GET /releases/assets/<id> with Accept: application/octet-stream, streamed into <folder>/<asset name> -> sha256 ->
//        the byte count must equal the asset's size, and the sha256 must equal the asset's digest ("sha256:<hex>") when it has one.
// report (always written, also on failure - then ok=false and error says why):
//   { ok, kind, tag, release_id, draft, prerelease, release_name, asset_name, asset_id, size, sha256, digest, digest_match,
//     asset_updated_at, saved_to, duplicates_note, ... }
// On failure: prints the HTTP status and the first 300 characters of the response body, exit code 1. Same retry rules as
// diag/publish-results.mjs (5 tries, 5xx / 429 / rate-limited 403 are retried, Retry-After is honoured). Node 22, no packages.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { Readable, Transform } from 'node:stream';
import { pipeline } from 'node:stream/promises';

function parseArgs(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('--')) {
      const k = a.slice(2);
      const nxt = argv[i + 1];
      if (nxt !== undefined && !nxt.startsWith('--')) {
        o[k] = nxt;
        i++;
      } else {
        o[k] = true;
      }
    }
  }
  return o;
}

const args = parseArgs(process.argv.slice(2));
const str = (v) => (v === undefined || v === true ? '' : String(v));
const TOKEN = process.env.GITHUB_TOKEN || '';
const REPO = process.env.GITHUB_REPOSITORY || '';
const API = (process.env.GITHUB_API_URL || 'https://api.github.com').replace(/\/+$/, '');
const TAG = str(args.tag);
const SUFFIX = str(args.suffix);
const OUT = str(args.out);
const REPORT = str(args.report);
const PER_PAGE = 50;
const MAX_PAGES = 3;
const num = (v, d) => {
  const n = Number(v);
  return Number.isFinite(n) && n >= 0 ? n : d;
};
// test knobs (the workflow does not pass them)
const RETRY_BASE_MS = num(args['retry-base-ms'], 2000);
// one download try may take 240 s (headers + all bytes); 3 tries stay inside the 15 minutes of the workflow step, so the report is written
const DOWNLOAD_TIMEOUT_MS = Math.max(5, num(args['download-timeout-sec'], 240)) * 1000;

const REP = {
  ok: false,
  kind: 'release-asset',
  script: 'fetch-release-asset.mjs',
  node: process.version,
  started: new Date().toISOString(),
  finished: null,
  repository: REPO,
  tag: TAG,
  suffix: SUFFIX,
  releases_scanned: 0,
  pages_read: 0,
  release_id: null,
  draft: null,
  prerelease: null,
  release_name: null,
  release_created_at: null,
  release_published_at: null,
  asset_names: null,
  asset_name: null,
  asset_id: null,
  asset_state: null,
  size: null,
  bytes_written: null,
  sha256: null,
  digest: null,
  digest_match: null,
  asset_updated_at: null,
  saved_to: null,
  duplicates_note: null,
  download: null,
  error: null,
  http_status: null,
  body_head: null,
};

class ApiError extends Error {
  constructor(status, body, what) {
    super(`${what}: HTTP ${status}`);
    this.status = status;
    this.body = body;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// nothing that is printed or written may carry the token; free texts (error messages, response bodies) also lose the query string of
// any URL they quote (a signed download URL carries its signature there)
function noToken(s) {
  const t = String(s === undefined || s === null ? '' : s);
  return TOKEN ? t.split(TOKEN).join('<token>') : t;
}
function scrub(s) {
  return noToken(s).replace(/(https?:\/\/[^\s"'<>?]+)\?[^\s"'<>]*/g, '$1?<query removed>');
}

function apiHeaders(accept) {
  return {
    Authorization: `Bearer ${TOKEN}`,
    Accept: accept,
    'X-GitHub-Api-Version': '2022-11-28',
    'User-Agent': 'diag-win11-fetch-release-asset',
  };
}

async function ghJson(urlPath, what) {
  const url = `${API}/repos/${REPO}${urlPath}`;
  let lastErr = null;
  for (let attempt = 1; attempt <= 5; attempt++) {
    let waitMs = RETRY_BASE_MS * attempt;
    try {
      const res = await fetch(url, { method: 'GET', headers: apiHeaders('application/vnd.github+json'), signal: AbortSignal.timeout(60000) });
      const text = await res.text();
      if (res.ok) {
        return text ? JSON.parse(text) : null;
      }
      const limited = res.status === 429 || (res.status === 403 && /rate limit|abuse|secondary/i.test(text));
      const retryable = res.status >= 500 || limited;
      lastErr = new ApiError(res.status, text, what);
      if (!retryable || attempt === 5) throw lastErr;
      const ra = Number(res.headers.get('retry-after'));
      if (Number.isFinite(ra) && ra > 0) waitMs = Math.min(ra, 120) * 1000 + 500;
      else if (limited) waitMs = Math.max(waitMs, 60000); // docs: without Retry-After wait at least one minute
    } catch (e) {
      if (e instanceof ApiError) throw e; // the retry decision was already taken above
      lastErr = e; // network error or a body that is not JSON: retry
      if (attempt === 5) throw e;
    }
    await sleep(waitMs);
  }
  throw lastErr;
}

function originOf(u) {
  try {
    return new URL(u).origin;
  } catch (e) {
    return null;
  }
}

// One asset, streamed to <dest>.part (never a half-written *.exe in the folder). 3 tries for network errors / 5xx / 429.
async function downloadAsset(assetId, dest) {
  // GET /releases/assets/<id> with Accept: application/octet-stream answers 302 to a signed storage URL on ANOTHER origin (or 200 with the
  // bytes). fetch follows the redirect itself. Authorization is not carried to the other origin: Node's fetch (undici, Node 18.x and later,
  // so Node 22 too) implements the Fetch standard's "HTTP-redirect fetch" step that deletes the Authorization header when the redirect
  // leaves the request's origin - measured with a local two-origin server on Node 22 and Node 24 (W11/UPG/WORKLOG.md).
  const url = `${API}/repos/${REPO}/releases/assets/${assetId}`;
  const part = dest + '.part';
  let lastErr = null;
  for (let attempt = 1; attempt <= 3; attempt++) {
    const t0 = Date.now();
    try {
      const res = await fetch(url, { method: 'GET', headers: apiHeaders('application/octet-stream'), redirect: 'follow', signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) });
      if (!res.ok) {
        const text = await res.text().catch(() => '');
        const err = new ApiError(res.status, text, `download of asset ${assetId}`);
        err.retryable = res.status >= 500 || res.status === 429;
        throw err;
      }
      if (!res.body) throw new Error('the download answer has no body');
      const hash = crypto.createHash('sha256');
      let bytes = 0;
      const meter = new Transform({
        transform(chunk, enc, cb) {
          hash.update(chunk);
          bytes += chunk.length;
          cb(null, chunk);
        },
      });
      await pipeline(Readable.fromWeb(res.body), meter, fs.createWriteStream(part));
      return {
        attempts: attempt,
        ms: Date.now() - t0,
        status: res.status,
        redirected: res.redirected === true,
        final_origin: originOf(res.url),
        api_origin: originOf(API),
        content_type: res.headers.get('content-type'),
        bytes,
        sha256: hash.digest('hex'),
        part,
      };
    } catch (e) {
      lastErr = e;
      try {
        fs.rmSync(part, { force: true });
      } catch (e2) {
        // ignore
      }
      const retryable = !(e instanceof ApiError) || e.retryable === true;
      if (!retryable || attempt === 3) throw e;
    }
    await sleep(RETRY_BASE_MS * attempt);
  }
  throw lastErr;
}

function writeReport() {
  REP.finished = new Date().toISOString();
  if (!REPORT) return;
  try {
    fs.mkdirSync(path.dirname(path.resolve(REPORT)), { recursive: true });
    fs.writeFileSync(REPORT, noToken(JSON.stringify(REP, null, 2)));
  } catch (e) {
    console.error('fetch-release-asset: the report could not be written: ' + scrub(e && e.message ? e.message : e));
  }
}

function fail(msg, e) {
  const status = e && e.status ? ` status=${e.status}` : '';
  const body = e && e.body ? ` body=${String(e.body).slice(0, 300).replace(/\s+/g, ' ')}` : '';
  const cause = e && e.cause && e.cause.message ? ` cause=${e.cause.message}` : '';
  REP.ok = false;
  REP.error = scrub(msg + cause);
  REP.http_status = e && e.status ? e.status : null;
  REP.body_head = e && e.body ? scrub(String(e.body).slice(0, 300)) : null;
  writeReport();
  console.error(scrub(`fetch-release-asset FAILED: ${msg}${status}${body}${cause}`));
  process.exit(1);
}

async function main() {
  if (!TOKEN) fail('GITHUB_TOKEN is empty');
  if (!REPO || !REPO.includes('/')) fail('GITHUB_REPOSITORY is not set');
  if (!TAG) fail('--tag is required');
  if (!SUFFIX) fail('--suffix is required');
  if (!OUT) fail('--out is required');
  if (!REPORT) fail('--report is required');

  // 1. the release with this tag_name, from the list (drafts included)
  const matches = [];
  for (let page = 1; page <= MAX_PAGES; page++) {
    const list = await ghJson(`/releases?per_page=${PER_PAGE}&page=${page}`, `release list page ${page}`);
    if (!Array.isArray(list)) fail(`release list page ${page} is not an array`);
    REP.pages_read = page;
    REP.releases_scanned += list.length;
    for (const r of list) {
      if (r && r.tag_name === TAG) matches.push(r);
    }
    if (list.length < PER_PAGE) break;
  }
  if (!matches.length) {
    fail(`no release with tag_name ${TAG} among the ${REP.releases_scanned} release(s) listed (${REP.pages_read} page(s) of ${PER_PAGE}; a draft is listed only for a token with push access)`);
  }
  const drafts = matches.filter((r) => r.draft === true);
  const rel = drafts.length ? drafts[0] : matches[0];
  if (matches.length > 1) {
    REP.duplicates_note =
      `${matches.length} releases have tag_name ${TAG}: ` +
      matches.map((r) => `id ${r.id} draft=${r.draft === true} created ${r.created_at}`).join('; ') +
      ` - took id ${rel.id} (${drafts.length ? 'a draft is preferred over a published release' : 'none is a draft: the first of the list'}${drafts.length > 1 ? '; ' + drafts.length + ' drafts: the first of the list' : ''})`;
  }
  REP.release_id = rel.id;
  REP.draft = rel.draft === true;
  REP.prerelease = rel.prerelease === true;
  REP.release_name = rel.name === undefined ? null : rel.name;
  REP.release_created_at = rel.created_at || null;
  REP.release_published_at = rel.published_at || null;

  // 2. its assets (own endpoint: complete and paged, 100 per page)
  const assets = [];
  for (let page = 1; page <= MAX_PAGES; page++) {
    const list = await ghJson(`/releases/${rel.id}/assets?per_page=100&page=${page}`, `asset list page ${page}`);
    if (!Array.isArray(list)) fail(`asset list page ${page} is not an array`);
    for (const a of list) assets.push(a);
    if (list.length < 100) break;
  }
  REP.asset_names = assets.map((a) => String(a && a.name));
  const hits = assets.filter((a) => a && typeof a.name === 'string' && a.name.endsWith(SUFFIX));
  if (hits.length !== 1) {
    fail(`${hits.length} assets of release ${rel.id} (${TAG}) end with ${SUFFIX}, exactly 1 is needed; ending with it: [${hits.map((a) => a.name).join(', ')}]; all assets: [${REP.asset_names.join(', ')}]`);
  }
  const asset = hits[0];
  REP.asset_name = asset.name;
  REP.asset_id = asset.id;
  REP.asset_state = asset.state === undefined ? null : asset.state;
  REP.size = typeof asset.size === 'number' ? asset.size : null;
  REP.digest = typeof asset.digest === 'string' && asset.digest ? asset.digest : null;
  REP.asset_updated_at = asset.updated_at || null;
  // the name becomes a file name: no path parts
  if (asset.name !== path.basename(asset.name) || /[\\/:*?"<>|]/.test(asset.name)) fail(`the asset name is not a plain file name: ${asset.name}`);
  if (REP.asset_state !== null && REP.asset_state !== 'uploaded') fail(`the asset ${asset.name} is not fully uploaded yet (state ${REP.asset_state})`);

  // 3. download, size and digest
  fs.mkdirSync(OUT, { recursive: true });
  const dest = path.resolve(OUT, asset.name);
  const d = await downloadAsset(asset.id, dest);
  REP.download = { attempts: d.attempts, ms: d.ms, status: d.status, redirected: d.redirected, final_origin: d.final_origin, api_origin: d.api_origin, content_type: d.content_type };
  REP.bytes_written = d.bytes;
  REP.sha256 = d.sha256;
  const drop = () => {
    try {
      fs.rmSync(d.part, { force: true });
    } catch (e) {
      // ignore
    }
  };
  if (REP.size === null || d.bytes !== REP.size) {
    drop();
    fail(`size mismatch for ${asset.name}: the asset says ${REP.size} bytes, ${d.bytes} bytes were received (nothing is kept)`);
  }
  if (REP.digest !== null) {
    const want = REP.digest.toLowerCase();
    REP.digest_match = want === 'sha256:' + d.sha256;
    if (!REP.digest_match) {
      drop();
      fail(`digest mismatch for ${asset.name}: the asset says ${REP.digest}, the received bytes have sha256:${d.sha256} (nothing is kept)`);
    }
  }
  fs.rmSync(dest, { force: true });
  fs.renameSync(d.part, dest);
  REP.saved_to = dest;
  REP.ok = true;
  writeReport();
  console.log(
    noToken(
      `fetch-release-asset: ok ${TAG} release ${rel.id} (draft=${REP.draft}) asset ${asset.name} id ${asset.id}: ${d.bytes} bytes, sha256 ${d.sha256}, digest ${REP.digest === null ? 'none' : 'match'}, redirected=${d.redirected} -> ${dest}`,
    ),
  );
  if (REP.duplicates_note) console.log(noToken('fetch-release-asset: NOTE ' + REP.duplicates_note));
}

try {
  await main();
} catch (e) {
  fail(e && e.message ? e.message : String(e), e);
}
