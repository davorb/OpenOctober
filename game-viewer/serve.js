#!/usr/bin/env node
// Darkfall game-viewer dev server (no dependencies).  usage: node serve.js [port]
//   /            -> this folder          /data/       -> ./data  (decrypted game data + built json)
//   /converted/  -> ../converted         /dds/        -> ../export/texture/dds
//   /tex/        -> ../export/texture (colour-grade LUTs)     /audio/      -> ../export/audio
const http = require('http'), fs = require('fs'), path = require('path');
const HERE = __dirname, ROOT = path.resolve(HERE, '..');
const MOUNTS = [
  ['/data/', path.join(HERE, 'data')], ['/converted/', path.join(ROOT, 'converted')],
  ['/dds/', path.join(ROOT, 'export/texture/dds')], ['/tex/', path.join(ROOT, 'export/texture')], ['/audio/', path.join(ROOT, 'export/audio')],
  ['/', HERE],
];
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.json': 'application/json', '.css': 'text/css', '.png': 'image/png',
  '.glb': 'model/gltf-binary', '.ogg': 'audio/ogg', '.wav': 'audio/wav', '.dds': 'application/octet-stream', '.xml': 'text/plain', '.teq': 'text/plain', '.frag': 'text/plain' };
http.createServer((req, res) => {
  let p; try { p = decodeURIComponent(req.url.split('?')[0]); } catch { res.writeHead(400).end(); return; }
  if (p.endsWith('/')) p += 'index.html';
  const m = MOUNTS.find(([pre]) => p.startsWith(pre));
  const file = path.normalize(path.join(m[1], p.slice(m[0].length)));
  if (!file.startsWith(m[1] + path.sep) && file !== m[1]) { res.writeHead(403).end(); return; }
  fs.stat(file, (err, st) => {
    if (err || !st.isFile()) { res.writeHead(404).end('not found'); return; }
    const h = { 'Content-Type': TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream', 'Accept-Ranges': 'bytes',
      'Cache-Control': (m[0] === '/' || m[0] === '/data/') ? 'no-cache' : 'max-age=3600' };
    const r = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
    if (r) {
      const s = r[1] ? +r[1] : 0, e = r[2] ? Math.min(+r[2], st.size - 1) : st.size - 1;
      res.writeHead(206, { ...h, 'Content-Range': `bytes ${s}-${e}/${st.size}`, 'Content-Length': e - s + 1 });
      fs.createReadStream(file, { start: s, end: e }).pipe(res);
    } else { res.writeHead(200, { ...h, 'Content-Length': st.size }); fs.createReadStream(file).pipe(res); }
  });
}).listen(+process.argv[2] || 8080, () => console.log(`Darkfall game-viewer: http://localhost:${+process.argv[2] || 8080}/`));
