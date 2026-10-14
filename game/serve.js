#!/usr/bin/env node
// OpenOctober game dev server (no dependencies).  usage: node serve.js [port]
//   env BASE_PATH=/df     serve under a path prefix (behind a reverse proxy that keeps it or strips it): root-absolute URLs in .html/.js are rewritten on the fly
//   env GAME_SERVER=host:port   where <prefix>/ws (the websocket) is proxied to (default 127.0.0.1:1338); X-Forwarded-For of the reverse proxy is passed on
//   /            -> this folder          /data/       -> ./data  (unpacked game data + built json)
//   /converted/  -> ../converted         /dds/        -> ../export/texture/dds
//   /tex/        -> ../export/texture (colour-grade LUTs)     /audio/      -> ../export/audio
const http = require('http'), net = require('net'), fs = require('fs'), path = require('path'), zlib = require('zlib'), crypto = require('crypto');
const HERE = __dirname, ROOT = path.resolve(HERE, '..');
const PREFIX = (process.env.BASE_PATH || '').replace(/\/+$/, '');
const [GS_HOST, GS_PORT] = (process.env.GAME_SERVER || '127.0.0.1:1338').split(/:(?=\d+$)/);
const PORT = +process.argv[2] || +process.env.PORT || 1337;
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
// Under a prefix the code's root-absolute URLs ('/data/..', `/converted/..`, '/js/..') get the prefix; the rewritten copy is kept in .cache/ per source version.
const ABS = /(['"`])\/(?=(?:data|converted|dds|tex|js|css|vendor)\/|audio['"`/])/g;
function rewritten(file, st) {
  const out = path.join(CACHE, 'rw-' + crypto.createHash('sha1').update(`${PREFIX}|${file}|${st.size}|${st.mtimeMs}`).digest('hex') + path.extname(file));
  if (!fs.existsSync(out)) {
    fs.mkdirSync(CACHE, { recursive: true });
    const tmp = out + '.' + process.pid;
    fs.writeFileSync(tmp, fs.readFileSync(file, 'utf8').replace(ABS, `$1${PREFIX}/`));
    fs.renameSync(tmp, out);
  }
  return out;
}
const server = http.createServer((req, res) => {
  let p; try { p = decodeURIComponent(req.url.split('?')[0]); } catch { res.writeHead(400).end(); return; }
  if (PREFIX) { if (p === PREFIX) { res.writeHead(301, { Location: PREFIX + '/' }).end(); return; } if (!p.startsWith(PREFIX + '/')) { res.writeHead(404).end('not found'); return; } p = p.slice(PREFIX.length); }
  if (p.endsWith('/')) p += 'index.html';
  const m = MOUNTS.find(([pre]) => p.startsWith(pre));
  let file = path.normalize(path.join(m[1], p.slice(m[0].length)));
  if (!file.startsWith(m[1] + path.sep) && file !== m[1]) { res.writeHead(403).end(); return; }
  fs.stat(file, (err, st) => {
    if (err || !st.isFile()) { res.writeHead(404).end('not found'); return; }
    const ext = path.extname(file).toLowerCase();
    if (PREFIX && (ext === '.html' || ext === '.js') && !file.startsWith(path.join(HERE, 'vendor') + path.sep)) { file = rewritten(file, st); st = fs.statSync(file); }
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
});
// <prefix>/ws -> the server, so a single port (and a single reverse-proxy upstream) serves both
server.on('upgrade', (req, sock) => {
  const p = req.url.split('?')[0];
  if (p !== PREFIX + '/ws') { sock.destroy(); return; }
  const up = net.connect(+GS_PORT, GS_HOST);
  const peer = (req.socket.remoteAddress || '').replace(/^::ffff:/, '');
  const hdr = []; let xff = peer;
  for (let i = 0; i < req.rawHeaders.length; i += 2) {
    if (/^x-forwarded-for$/i.test(req.rawHeaders[i])) xff = req.rawHeaders[i + 1]; else hdr.push(`${req.rawHeaders[i]}: ${req.rawHeaders[i + 1]}`);
  }
  up.on('connect', () => { up.write(`GET /ws HTTP/1.1\r\n${hdr.join('\r\n')}\r\nX-Forwarded-For: ${xff}\r\n\r\n`); sock.pipe(up); up.pipe(sock); });
  up.on('error', () => sock.destroy()); sock.on('error', () => up.destroy()); up.on('close', () => sock.destroy()); sock.on('close', () => up.destroy());
});
server.listen(PORT, process.env.BIND || undefined, () => console.log(`OpenOctober game: http://localhost:${PORT}${PREFIX}/`));
