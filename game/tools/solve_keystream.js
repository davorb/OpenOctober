// Many-time-pad solver. Archive entries of one cipher class share a position-indexed keystream (xor).
// usage: node solve_keystream.js <encDir> <out.bin> [maxLen] [--header "crib"] [--self N]
//   --self N : after the first pass, retrain the bigram on the decoded text and re-solve N times
const fs = require('fs'), path = require('path');
const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf(n); return i >= 0 ? args[i + 1] : d; };
const encDir = path.resolve(args[0]), outFile = path.resolve(args[1]);
const maxLen = +args[2] || 1100000;
const HEADER = opt('--header') ? Buffer.from(JSON.parse('"' + opt('--header') + '"'), 'latin1') : Buffer.alloc(0);
const SELF = +opt('--self', 0);
const ROOT = path.resolve(__dirname, '../../export');
const walk = (d, out = []) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); e.isDirectory() ? walk(p, out) : out.push(p); } return out; };

function bigram(bufs) {
  const c2 = new Float64Array(65536).fill(0.01), c3 = new Uint32Array(1 << 24), t3 = new Uint32Array(65536);
  for (const b of bufs) for (let i = 2; i < b.length; i++) { c2[b[i - 1] * 256 + b[i]]++; c3[(b[i - 2] * 65536) + b[i - 1] * 256 + b[i]]++; t3[b[i - 2] * 256 + b[i - 1]]++; }
  const p2 = new Float32Array(65536);
  for (let p = 0; p < 256; p++) { let t = 0; for (let c = 0; c < 256; c++) t += c2[p * 256 + c]; for (let c = 0; c < 256; c++) p2[p * 256 + c] = c2[p * 256 + c] / t; }
  const LP = new Float32Array(1 << 24), L = 2.0;
  for (let q = 0; q < 65536; q++) { const tt = t3[q], p1 = q & 255; for (let c = 0; c < 256; c++) LP[q * 256 + c] = Math.log((c3[q * 256 + c] + L * p2[p1 * 256 + c]) / (tt + L)); }
  return LP;
}
let corpus = []; { let n = 0; for (const f of walk(path.join(ROOT, opt('--train', 'xml')))) { if (n > 30e6) break; const b = fs.readFileSync(f); n += b.length; corpus.push(b); } }
let LP = bigram(corpus);

const EXCL = opt('--exclude');
const files = walk(encDir).filter(p => !EXCL || !p.includes('/' + EXCL + '/')).map(p => ({ p, d: fs.readFileSync(p) })).sort((a, b) => b.d.length - a.d.length);
const MAXF_DEEP = 48, N = Math.min(maxLen, files[0].d.length);
function solve() {
  const ks = new Uint8Array(N), prev = new Uint8Array(files.length), prev2 = new Uint8Array(files.length), score = new Float64Array(256);
  for (let i = 0; i < HEADER.length; i++) ks[i] = files[0].d[i] ^ HEADER[i];
  prev.fill(HEADER.length ? HEADER[HEADER.length - 1] : 10); prev2.fill(HEADER.length > 1 ? HEADER[HEADER.length - 2] : 10);
  for (let i = HEADER.length; i < N; i++) {
    score.fill(0);
    const lim = 1e9; let n = 0;
    for (let f = 0; f < files.length && n < lim; f++) {
      const d = files[f].d; if (i >= d.length) break;
      const c = d[i], base = (prev2[f] * 256 + prev[f]) * 256; n++;
      for (let k = 0; k < 256; k++) score[k] += LP[base + (c ^ k)];
    }
    let best = 0; for (let k = 1; k < 256; k++) if (score[k] > score[best]) best = k;
    ks[i] = best;
    for (let f = 0; f < files.length; f++) { const d = files[f].d; if (i >= d.length) break; prev2[f] = prev[f]; prev[f] = d[i] ^ best; }
  }
  return ks;
}
let ks = solve();
for (let r = 0; r < SELF; r++) {
  const dec = files.map(f => Buffer.from(f.d.map((c, i) => c ^ (i < ks.length ? ks[i] : 0))));
  LP = bigram(dec); ks = solve();
  console.log('self pass', r + 1);
}
fs.writeFileSync(outFile, ks);
const f0 = files.find(f => f.p.includes(opt('--show', 'zzzz'))) || files[files.length - 1];
console.log(Buffer.from(f0.d.subarray(0, 600).map((c, i) => c ^ ks[i])).toString('latin1'));
