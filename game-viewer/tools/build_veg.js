// Vegetation tufts + per-tile grass type tables from export/veg  ->  data/veg.json
//   meshes: { name: { tex, pos[], nrm[], uv[], idx[] } }       (export/veg/<name>.sfvm, game space: x, y(up), z; we flip z at load)
//   tiles : { tile: [[meshName, f0, f1, f2, f3], ...] }       (export/veg/terrain_<tile>.sfv type table; the run-length cell lists in that file are NOT decoded)
// .sfvm = 'SFVM', u32 ver, ... , NUL-terminated texture name at 0x3c, 36 byte vertices {pos3, wind f, normal3, u, v}, u16 triangle list
// (the vertex block is addressed from the end of the file: header is always 188 bytes). .sfv = 'SFVG' header, N (start,count) u32 runs, then 148 byte
// records {4 floats, 128 byte mesh name}.
const fs = require('fs'), path = require('path');
const DIR = path.resolve(__dirname, '../../export/veg');
const SKIP = /^(corn|scatter|grass_stones|grass_seaweed|grass_crop|flower_mirdain)/;
const meshes = {}, tiles = {};
const r3 = a => Array.from(a, x => +x.toFixed(3));
for (const f of fs.readdirSync(DIR)) {
  if (!f.endsWith('.sfvm') || SKIP.test(f)) continue;
  const b = fs.readFileSync(path.join(DIR, f)), tri = b.readUInt32LE(0x38);
  const idx = []; for (let i = 0; i < tri * 3; i++) idx.push(b.readUInt16LE(b.length - tri * 6 + i * 2));
  const nv = Math.max(...idx) + 1, s = b.length - tri * 6 - nv * 36;
  if (s !== 188) { console.warn('skip', f, 'layout'); continue; }
  const tex = b.slice(0x3c, 0x3c + 64).toString('latin1').split('\0')[0];
  const pos = [], nrm = [], uv = [];
  for (let v = 0; v < nv; v++) {
    const o = s + v * 36, F = i => b.readFloatLE(o + i * 4);
    pos.push(F(0), F(1), -F(2)); nrm.push(F(4), F(5), -F(6)); uv.push(F(7), F(8));
  }
  meshes[f.replace('.sfvm', '')] = { tex: tex.replace(/\.dds$/, '').replace(/^veg_/, 'veg_'), pos: r3(pos), nrm: r3(nrm), uv: r3(uv), idx };
}
const files = fs.readdirSync(DIR).filter(f => /^terrain_.*\.sfv$/.test(f));
for (const f of files) {
  const tile = f.replace(/^terrain_/, '').replace(/(_edit)?\.sfv$/, '');
  if (f.includes('_edit') && files.includes(`terrain_${tile}.sfv`)) continue;     // prefer the plain file
  const b = fs.readFileSync(path.join(DIR, f)); if (b.length < 24 || b.toString('latin1', 0, 4) !== 'SFVG') continue;
  const n = b.readUInt32LE(20), off = 24 + n * 8, rec = [];
  for (let o = off; o + 148 <= b.length; o += 148) {
    const name = b.toString('latin1', o + 16, o + 16 + 128).split('\0')[0].replace(/\.sfvm$/, '');
    if (meshes[name]) rec.push([name, ...[0, 1, 2, 3].map(i => +b.readFloatLE(o + i * 4).toFixed(4))]);
  }
  if (rec.length) tiles[tile] = rec;
}
fs.writeFileSync(path.join(__dirname, '../data/veg.json'), JSON.stringify({ meshes, tiles }));
console.log('meshes', Object.keys(meshes).length, 'tiles', Object.keys(tiles).length, fs.statSync(path.join(__dirname, '../data/veg.json')).size);
