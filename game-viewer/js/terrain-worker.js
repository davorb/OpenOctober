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
    // 4 m height grid, rasterised from the actual triangles (used for walking, ground clamping and altitude queries)
    const G = 256, cell = 1024 / G, W = G + 1, hg = new Float32Array(W * W).fill(-1e9), P = pos.arr;
    for (const g of groups) {
      const ix = g.index;
      for (let t = 0; t < ix.length; t += 3) {
        const a = ix[t] * 3, b = ix[t + 1] * 3, c = ix[t + 2] * 3;
        const ax = (P[a] + 512) / cell, az = (P[a + 2] + 512) / cell, bx = (P[b] + 512) / cell, bz = (P[b + 2] + 512) / cell, cx = (P[c] + 512) / cell, cz = (P[c + 2] + 512) / cell;
        const x0 = Math.max(0, Math.ceil(Math.min(ax, bx, cx))), x1 = Math.min(G, Math.floor(Math.max(ax, bx, cx)));
        const z0 = Math.max(0, Math.ceil(Math.min(az, bz, cz))), z1 = Math.min(G, Math.floor(Math.max(az, bz, cz)));
        const den = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz); if (Math.abs(den) < 1e-9) continue;
        for (let z = z0; z <= z1; z++) for (let x = x0; x <= x1; x++) {
          const l1 = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / den, l2 = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / den, l3 = 1 - l1 - l2;
          if (l1 < -1e-4 || l2 < -1e-4 || l3 < -1e-4) continue;
          const y = l1 * P[a + 1] + l2 * P[b + 1] + l3 * P[c + 1], k = z * W + x; if (y > hg[k]) hg[k] = y;
        }
      }
    }
    for (let pass = 0; pass < 12; pass++) {
      let holes = 0; const src = hg.slice();
      for (let z = 0; z <= G; z++) for (let x = 0; x <= G; x++) {
        const k = z * W + x; if (src[k] > -1e8) continue; let sum = 0, n = 0;
        for (let dz = -1; dz <= 1; dz++) for (let dx = -1; dx <= 1; dx++) { const xx = x + dx, zz = z + dz; if (xx < 0 || zz < 0 || xx > G || zz > G) continue; const v = src[zz * W + xx]; if (v > -1e8) { sum += v; n++; } }
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
