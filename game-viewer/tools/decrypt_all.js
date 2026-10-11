// decrypt xml-files (xml keystream) and xml-files/shaders (shader keystream) into ../data
const fs = require('fs'), path = require('path');
const src = path.resolve(__dirname, '../../export/xml-files'), out = path.resolve(__dirname, '../data');
const kx = fs.readFileSync(path.join(__dirname, 'keystream.bin')), ksh = fs.readFileSync(path.join(__dirname, 'ks_shader.bin'));
function dec(d, ks) { const o = Buffer.alloc(d.length); for (let i = 0; i < d.length; i++) o[i] = d[i] ^ (i < ks.length ? ks[i] : 0); return o; }
const want = process.argv.slice(2);
(function w(d, rel) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name), r = path.join(rel, e.name);
    if (e.isDirectory()) { w(p, r); continue; }
    if (want.length && !want.some(x => r.startsWith(x))) continue;
    const o = path.join(out, r); fs.mkdirSync(path.dirname(o), { recursive: true });
    fs.writeFileSync(o, dec(fs.readFileSync(p), r.startsWith('shaders') ? ksh : kx));
  }
})(src, '');
