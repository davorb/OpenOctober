#!/usr/bin/env node
// Darkfall game-viewer dev server (no dependencies).  usage: node serve.js [port]
//   /            -> this folder          /data/       -> ./data  (decrypted game data + built json)
//   /converted/  -> ../converted         /dds/        -> ../export/texture/dds
//   /tex/        -> ../export/texture (colour-grade LUTs)     /audio/      -> ../export/audio
const http = require('http'), fs = require('fs'), path = require('path'), zlib = require('zlib'), crypto = require('crypto');
const HERE = __dirname, ROOT = path.resolve(HERE, '..');
const MOUNTS = [
  ['/data/', path.join(HERE, 'data')], ['/converted/', path.join(ROOT, 'converted')],
  ['/dds/', path.join(ROOT, 'export/texture/dds')], ['/tex/', path.join(ROOT, 'export/texture')], ['/audio/', path.join(ROOT, 'export/audio')],
  ['/', HERE],
];
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.css': 'text/css', '.png': 'image/png',
  '.glb': 'model/gltf-binary', '.ogg': 'audio/ogg', '.wav': 'audio/wav', '.dds': 'application/octet-stream', '.xml': 'text/plain', '.teq': 'text/plain', '.frag': 'text/plain' };
// Behind a CDN: the game-content mounts never change once converted, so they are cached for a month (the CDN honours this and so does
// the browser); code and /data/ (rebuilt by tools/) revalidate with an ETag, so an unchanged file is a 304 and not a re-download.
// Compressible types are brotli'd once and kept in .cache/ (the CDNs don't compress model/gltf-binary or octet-stream themselves).
const IMMUTABLE = new Set(['/converted/', '/dds/', '/tex/', '/audio/']);
const COMPRESS = new Set(['.html', '.js', '.json', '.css', '.xml', '.teq', '.frag', '.glb', '.dds', '.wav']);
const CACHE = path.join(HERE, '.cache');
const inflight = new Map();
function brotliOf(file, st) {
  const out = path.join(CACHE, crypto.createHash('sha1').update(`${file}|${st.size}|${st.mtimeMs}`).digest('hex') + '.br');
  if (fs.existsSync(out)) return Promise.resolve(out);
  if (!inflight.has(out)) {
    fs.mkdirSync(CACHE, { recursive: true });
    const tmp = out + '.' + process.pid;
    const p = new Promise((ok, fail) => fs.createReadStream(file)
      .pipe(zlib.createBrotliCompress({ params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 5, [zlib.constants.BROTLI_PARAM_SIZE_HINT]: st.size } }))
      .pipe(fs.createWriteStream(tmp)).on('finish', () => { fs.renameSync(tmp, out); ok(out); }).on('error', fail));
    inflight.set(out, p.finally(() => inflight.delete(out)));
  }
  return inflight.get(out);
}
http.createServer((req, res) => {
  let p; try { p = decodeURIComponent(req.url.split('?')[0]); } catch { res.writeHead(400).end(); return; }
  if (p.endsWith('/')) p += 'index.html';
  const m = MOUNTS.find(([pre]) => p.startsWith(pre));
  const file = path.normalize(path.join(m[1], p.slice(m[0].length)));
  if (!file.startsWith(m[1] + path.sep) && file !== m[1]) { res.writeHead(403).end(); return; }
  fs.stat(file, (err, st) => {
    if (err || !st.isFile()) { res.writeHead(404).end('not found'); return; }
    const ext = path.extname(file).toLowerCase();
    const etag = `"${st.size.toString(36)}-${Math.floor(st.mtimeMs).toString(36)}"`;
    const h = { 'Content-Type': TYPES[ext] || 'application/octet-stream', 'Accept-Ranges': 'bytes', 'ETag': etag, 'Vary': 'Accept-Encoding',
      'Cache-Control': IMMUTABLE.has(m[0]) ? 'public, max-age=2592000' : 'no-cache' };
    if (req.headers['if-none-match'] === etag) { res.writeHead(304, h).end(); return; }
    const r = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
    if (r) {
      const s = r[1] ? +r[1] : 0, e = r[2] ? Math.min(+r[2], st.size - 1) : st.size - 1;
      res.writeHead(206, { ...h, 'Content-Range': `bytes ${s}-${e}/${st.size}`, 'Content-Length': e - s + 1 });
      fs.createReadStream(file, { start: s, end: e }).pipe(res);
    } else if (COMPRESS.has(ext) && st.size > 1024 && /\bbr\b/.test(req.headers['accept-encoding'] || '')) {
      brotliOf(file, st).then(br => {
        res.writeHead(200, { ...h, 'Content-Encoding': 'br', 'Content-Length': fs.statSync(br).size });
        fs.createReadStream(br).pipe(res);
      }, () => { res.writeHead(500).end(); });
    } else { res.writeHead(200, { ...h, 'Content-Length': st.size }); fs.createReadStream(file).pipe(res); }
  });
}).listen(+process.argv[2] || 1337, () => console.log(`Darkfall game-viewer: http://localhost:${+process.argv[2] || 1337}/`));
