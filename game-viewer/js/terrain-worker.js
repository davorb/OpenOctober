// Parses a terrain-only tile GLB (<tile>_empty.glb) off the main thread and returns typed arrays.
// Layout (see FORMATS.md / sfexport.py): one node, one mesh; all primitives share POSITION/NORMAL/COLOR_0/_WEIGHTS0/_WEIGHTS1
// and differ by index buffer; mesh.extras.groups[i] = { materials, slots, textures } per primitive.
self.onmessage = async ({ data: { id, url } }) => {
  try {
    const buf = await (await fetch(url)).arrayBuffer();
    const dv = new DataView(buf);
    const jlen = dv.getUint32(12, true);
    const json = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 20, jlen)));
    const binOff = 20 + jlen + 8;
    const acc = i => {
      const a = json.accessors[i], bv = json.bufferViews[a.bufferView];
      const comps = { SCALAR: 1, VEC3: 3, VEC4: 4 }[a.type];
      const off = binOff + (bv.byteOffset || 0) + (a.byteOffset || 0);
      const T = { 5126: Float32Array, 5121: Uint8Array, 5125: Uint32Array, 5123: Uint16Array }[a.componentType];
      return { arr: new T(buf.slice(off, off + a.count * comps * T.BYTES_PER_ELEMENT)), count: a.count, comps, min: a.min, max: a.max };
    };
    const mesh = json.meshes[0], p0 = mesh.primitives[0].attributes;
    const pos = acc(p0.POSITION), nrm = acc(p0.NORMAL), col = acc(p0.COLOR_0), w0 = acc(p0._WEIGHTS0), w1 = acc(p0._WEIGHTS1);
    // normals -> int8 (snorm) to halve memory
    const n8 = new Int8Array(nrm.count * 4);
    for (let i = 0; i < nrm.count; i++) { n8[i * 4] = Math.round(nrm.arr[i * 3] * 127); n8[i * 4 + 1] = Math.round(nrm.arr[i * 3 + 1] * 127); n8[i * 4 + 2] = Math.round(nrm.arr[i * 3 + 2] * 127); }
    const small = pos.count < 65536;
    const groups = mesh.primitives.map((p, i) => {
      const ix = acc(p.indices).arr;
      return { index: small ? Uint16Array.from(ix) : ix, ...mesh.extras.groups[i] };
    });
    // coarse height grid (max y per 8 m cell, holes filled from neighbours) for camera clamping / altitude queries
    const G = 128, cell = 1024 / G, hg = new Float32Array((G + 1) * (G + 1)).fill(-1e9);
    for (let i = 0; i < pos.count; i++) {
      const gx = Math.round((pos.arr[i * 3] + 512) / cell), gz = Math.round((pos.arr[i * 3 + 2] + 512) / cell);
      if (gx < 0 || gz < 0 || gx > G || gz > G) continue;
      const k = gz * (G + 1) + gx; if (pos.arr[i * 3 + 1] > hg[k]) hg[k] = pos.arr[i * 3 + 1];
    }
    for (let pass = 0; pass < 12; pass++) {
      let holes = 0; const src = hg.slice();
      for (let z = 0; z <= G; z++) for (let x = 0; x <= G; x++) {
        const k = z * (G + 1) + x; if (src[k] > -1e8) continue; let sum = 0, n = 0;
        for (let dz = -1; dz <= 1; dz++) for (let dx = -1; dx <= 1; dx++) { const xx = x + dx, zz = z + dz; if (xx < 0 || zz < 0 || xx > G || zz > G) continue; const v = src[zz * (G + 1) + xx]; if (v > -1e8) { sum += v; n++; } }
        if (n) hg[k] = sum / n; else holes++;
      }
      if (!holes) break;
    }
    const node = json.nodes[0];
    const out = { id, pos: pos.arr, nrm: n8, col: col.arr, w0: w0.arr, w1: w1.arr, groups, heights: hg, min: pos.min, max: pos.max, translation: node.translation || [0, 0, 0] };
    const tr = [out.pos.buffer, n8.buffer, out.col.buffer, hg.buffer, out.w0.buffer, out.w1.buffer, ...groups.map(g => g.index.buffer)];
    self.postMessage(out, tr);
  } catch (e) { self.postMessage({ id, error: String(e) }); }
};
