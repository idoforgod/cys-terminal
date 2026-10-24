// diag/publish-results.mjs
// Uploads every file of the OUT folder to this repository as ONE parentless commit on a results branch,
// using only the GitHub REST Git Data API (blobs -> tree -> commit -> ref). No git, no checkout.
//
//   node diag/publish-results.mjs --job <e2e|sacrules> --os <runner label> --out <OUT folder>
//
// env: GITHUB_TOKEN (never printed), GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT
// branch: refs/heads/diag-results/<run_id>-<attempt>-<job>-<os>   (force-updated when it already exists)
// Files larger than 40 MB (and empty files) are skipped and listed in _publish-manifest.json.
// API economy (secondary rate limits for content creation): small UTF-8 text files go INLINE into the tree request
// (GitHub makes the blob itself); only binary / big files use the blobs endpoint. 403/429 answers honour Retry-After.
// On failure: prints the HTTP status and the first 300 characters of the response body, exit code 1.
import fs from 'node:fs';
import path from 'node:path';

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
const TOKEN = process.env.GITHUB_TOKEN || '';
const REPO = process.env.GITHUB_REPOSITORY || '';
const RUN_ID = process.env.GITHUB_RUN_ID || 'local';
const ATTEMPT = process.env.GITHUB_RUN_ATTEMPT || '1';
const JOB = String(args.job || 'job');
const OS = String(args.os || 'os');
const OUT = String(args.out || '');
const API = (process.env.GITHUB_API_URL || 'https://api.github.com').replace(/\/+$/, '');
const MAX_FILE_BYTES = 40 * 1024 * 1024;

const clean = (s) => String(s).replace(/[^A-Za-z0-9._-]+/g, '-');
const BRANCH = `diag-results/${clean(RUN_ID)}-${clean(ATTEMPT)}-${clean(JOB)}-${clean(OS)}`;

class ApiError extends Error {
  constructor(status, body, what) {
    super(`${what}: HTTP ${status}`);
    this.status = status;
    this.body = body;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function gh(method, urlPath, body, what) {
  const url = `${API}/repos/${REPO}${urlPath}`;
  let lastErr = null;
  for (let attempt = 1; attempt <= 5; attempt++) {
    let waitMs = 2000 * attempt;
    try {
      const res = await fetch(url, {
        method,
        headers: {
          Authorization: `Bearer ${TOKEN}`,
          Accept: 'application/vnd.github+json',
          'X-GitHub-Api-Version': '2022-11-28',
          'User-Agent': 'diag-win11-publish',
          'Content-Type': 'application/json',
        },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(60000),
      });
      const text = await res.text();
      if (res.ok) {
        return text ? JSON.parse(text) : {};
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
      lastErr = e; // network error: retry
      if (attempt === 5) throw e;
    }
    await sleep(waitMs);
  }
  throw lastErr;
}

function walk(dir, base, acc) {
  let entries = [];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch (e) {
    return acc;
  }
  for (const ent of entries) {
    const full = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      walk(full, base, acc);
    } else if (ent.isFile()) {
      let bytes = 0;
      try {
        bytes = fs.statSync(full).size;
      } catch (e) {
        bytes = -1;
      }
      acc.push({ full, rel: path.relative(base, full).split(path.sep).join('/'), bytes });
    }
  }
  return acc;
}

async function mapPool(items, limit, fn) {
  const results = new Array(items.length);
  let next = 0;
  async function worker() {
    for (;;) {
      const i = next++;
      if (i >= items.length) return;
      results[i] = await fn(items[i], i);
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
  return results;
}

function fail(msg, e) {
  const status = e && e.status ? ` status=${e.status}` : '';
  const body = e && e.body ? ` body=${String(e.body).slice(0, 300).replace(/\s+/g, ' ')}` : '';
  console.error(`publish-results FAILED: ${msg}${status}${body}`);
  process.exit(1);
}

async function main() {
  if (!TOKEN) fail('GITHUB_TOKEN is empty');
  if (!REPO || !REPO.includes('/')) fail('GITHUB_REPOSITORY is not set');
  if (!OUT) fail('--out is required');

  const files = [];
  const skipped = [];
  const all = fs.existsSync(OUT) ? walk(OUT, OUT, []) : [];
  for (const f of all) {
    if (f.bytes < 0) skipped.push({ path: f.rel, reason: 'stat failed' });
    else if (f.bytes === 0) skipped.push({ path: f.rel, reason: 'empty file' });
    else if (f.bytes > MAX_FILE_BYTES) skipped.push({ path: f.rel, bytes: f.bytes, reason: 'larger than 40 MB' });
    else files.push(f);
  }
  console.log(`publish-results: ${files.length} files to upload, ${skipped.length} skipped, out=${OUT}`);

  const serverUrl = process.env.GITHUB_SERVER_URL || 'https://github.com';
  const manifest = {
    generated: new Date().toISOString(),
    repository: REPO,
    run_id: RUN_ID,
    run_attempt: ATTEMPT,
    run_url: `${serverUrl}/${REPO}/actions/runs/${RUN_ID}`,
    job: JOB,
    os: OS,
    branch: BRANCH,
    node: process.version,
    files: files.map((f) => ({ path: f.rel, bytes: f.bytes })),
    skipped,
  };

  // text <= 200 KB each and <= 2.5 MB in total (valid UTF-8, no NUL, exact round trip) goes inline into the tree
  // request (GitHub caps tree requests at about 7 MB); everything else uses base64 blobs
  const INLINE_MAX = 200 * 1024;
  const INLINE_TOTAL_MAX = 2.5 * 1024 * 1024;
  let inlineTotal = 0;
  const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
  const inline = [];
  const forBlobs = [];
  for (const f of files) {
    const buf = fs.readFileSync(f.full);
    let text = null;
    if (buf.length <= INLINE_MAX && inlineTotal + buf.length <= INLINE_TOTAL_MAX && !buf.includes(0)) {
      try {
        const t = decoder.decode(buf);
        if (Buffer.from(t, 'utf8').equals(buf)) text = t;
      } catch (e) {
        text = null;
      }
    }
    if (text !== null) {
      inline.push({ path: f.rel, mode: '100644', type: 'blob', content: text });
      inlineTotal += buf.length;
    } else {
      forBlobs.push({ f, buf });
    }
  }
  const blobEntries = await mapPool(forBlobs, 2, async ({ f, buf }) => {
    const blob = await gh('POST', '/git/blobs', { content: buf.toString('base64'), encoding: 'base64' }, `blob ${f.rel}`);
    return { path: f.rel, mode: '100644', type: 'blob', sha: blob.sha };
  });
  const entries = inline.concat(blobEntries);
  manifest.inline_files = inline.length;
  manifest.blob_files = blobEntries.length;
  entries.push({ path: '_publish-manifest.json', mode: '100644', type: 'blob', content: JSON.stringify(manifest, null, 2) });

  // tree, commit (no parent), ref
  const tree = await gh('POST', '/git/trees', { tree: entries }, 'tree');
  const now = new Date().toISOString();
  const who = { name: 'diag-bot', email: 'diag-bot@users.noreply.github.com', date: now };
  const commit = await gh(
    'POST',
    '/git/commits',
    { message: `diag results run ${RUN_ID} ${JOB} ${OS}`, tree: tree.sha, parents: [], author: who, committer: who },
    'commit',
  );
  const ref = `refs/heads/${BRANCH}`;
  try {
    await gh('POST', '/git/refs', { ref, sha: commit.sha }, 'create ref');
  } catch (e) {
    if (e instanceof ApiError && (e.status === 422 || e.status === 409)) {
      await gh('PATCH', `/git/${ref}`, { sha: commit.sha, force: true }, 'update ref');
    } else {
      throw e;
    }
  }
  console.log(`publish-results: ok -> ${ref} (commit ${commit.sha}), ${files.length} files + manifest`);

  // step summary (plain markdown, no secrets)
  try {
    if (process.env.GITHUB_STEP_SUMMARY) {
      const lines = [`### diag results: ${JOB} / ${OS}`, '', `branch: \`${BRANCH}\` (commit \`${commit.sha}\`)`, `files: ${files.length}, skipped: ${skipped.length}`, ''];
      for (const name of ['e2e-verdict.json']) {
        const p = path.join(OUT, name);
        if (fs.existsSync(p)) {
          try {
            const j = JSON.parse(fs.readFileSync(p, 'utf8'));
            lines.push(`e2e verdict: **${j.verdict}** (source: ${j.verdict_source}) - ${String(j.why || '').slice(0, 400)}`, '');
          } catch (e) {
            // ignore
          }
        }
      }
      fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, lines.join('\n') + '\n');
    }
  } catch (e) {
    // ignore
  }
}

try {
  await main();
} catch (e) {
  fail(e && e.message ? e.message : String(e), e);
}
