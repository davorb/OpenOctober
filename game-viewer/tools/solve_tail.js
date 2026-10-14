// Extend keystream.bin past the 1.1 MB where solve_keystream.js ran out of ciphertexts: only the ~12 files longer than that
// still overlap, so positions >= START are solved from those alone (greedy byte-trigram ML, self-training passes).
// The language model is trained on the already-unpacked first 1.1 MB of the same files (same schemas) plus export/xml.
// usage: node solve_tail.js <out.bin> [passes=3] [maxLen]
const fs = require('fs'), path = require('path');
const out = path.resolve(process.argv[2]), PASSES = +process.argv[3] || 3;
const ROOT = path.resolve(__dirname, '../../export'), ENC = path.join(ROOT, 'xml-files');
const walk = (d, o = []) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); e.isDirectory() ? walk(p, o) : o.push(p); } return o; };
const base = require('./keystreams').need('keystream.bin'), START = base.length;
let files = walk(ENC).filter(p => !p.includes('/shaders/') && !p.endsWith('.DS_Store')).map(p => ({ p, d: fs.readFileSync(p) })).filter(f => f.d.length > START).sort((a, b) => b.d.length - a.d.length);
const N = Math.min(+process.argv[4] || 1e9, files[0].d.length);
console.log(files.map(f => path.relative(ENC, f.p) + ' ' + f.d.length).join('\n'));

function bigram(bufs) {
  const c2 = new Float64Array(65536).fill(0.01), c3 = new Uint32Array(1 << 24), t3 = new Uint32Array(65536);
  for (const b of bufs) for (let i = 2; i < b.length; i++) { c2[b[i - 1] * 256 + b[i]]++; c3[(b[i - 2] * 65536) + b[i - 1] * 256 + b[i]]++; t3[b[i - 2] * 256 + b[i - 1]]++; }
  const p2 = new Float32Array(65536);
  for (let p = 0; p < 256; p++) { let t = 0; for (let c = 0; c < 256; c++) t += c2[p * 256 + c]; for (let c = 0; c < 256; c++) p2[p * 256 + c] = c2[p * 256 + c] / t; }
  const LP = new Float32Array(1 << 24), L = 2.0;
  for (let q = 0; q < 65536; q++) { const tt = t3[q], p1 = q & 255; for (let c = 0; c < 256; c++) LP[q * 256 + c] = Math.log((c3[q * 256 + c] + L * p2[p1 * 256 + c]) / (tt + L)); }
  return LP;
}
const head = f => Buffer.from(f.d.subarray(0, START).map((c, i) => c ^ base[i]));
const corpus = files.map(head); { let n = 0; for (const f of walk(path.join(ROOT, 'xml'))) { if (n > 20e6) break; const b = fs.readFileSync(f); n += b.length; corpus.push(b); } }
let LP = bigram(corpus);

function solve(prevKs) {
  const ks = new Uint8Array(N); ks.set(base);
  const prev = new Uint8Array(files.length), prev2 = new Uint8Array(files.length), score = new Float64Array(256);
  files.forEach((f, j) => { prev[j] = f.d[START - 1] ^ base[START - 1]; prev2[j] = f.d[START - 2] ^ base[START - 2]; });
  for (let i = START; i < N; i++) {
    score.fill(0);
    let n = 0;
    for (let f = 0; f < files.length; f++) {
      const d = files[f].d; if (i >= d.length) break;
      const c = d[i], b = (prev2[f] * 256 + prev[f]) * 256; n++;
      for (let k = 0; k < 256; k++) score[k] += LP[b + (c ^ k)];
    }
    let best = 0; for (let k = 1; k < 256; k++) if (score[k] > score[best]) best = k;
    ks[i] = best;
    for (let f = 0; f < files.length; f++) { const d = files[f].d; if (i >= d.length) break; prev2[f] = prev[f]; prev[f] = d[i] ^ best; }
    if ((i & 0x3ffff) === 0) process.stdout.write(`\r${(i / 1e6).toFixed(1)}M (${n} files)`);
  }
  process.stdout.write('\n');
  return ks;
}
const bad = b => !(b === 9 || b === 10 || b === 13 || (b >= 32 && b < 127));
const report = ks => { for (const f of files.slice(0, 5)) { let nb = 0, t = 0; for (let i = START; i < Math.min(f.d.length, N); i++) { t++; if (bad(f.d[i] ^ ks[i])) nb++; } console.log(path.relative(ENC, f.p), 'bad bytes', (100 * nb / t).toFixed(3) + '%'); } };
let ks = solve(); report(ks); fs.writeFileSync(out, ks);
for (let r = 0; r < PASSES; r++) {
  const dec = files.map(f => Buffer.from(f.d.map((c, i) => c ^ (i < N ? ks[i] : 0))).subarray(0));
  LP = bigram(dec.concat(corpus.slice(files.length)));
  ks = solve(); console.log('self pass', r + 1); report(ks); fs.writeFileSync(out, ks);
}
