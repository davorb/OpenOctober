// unpack xml-files/* (type 3) with keystream.bin ; usage: node unpack_xml.js <in> [out]
const fs = require('fs'), path = require('path');
const ks = require('./keystreams').need('keystream.bin');
const d = fs.readFileSync(process.argv[2]);
const o = Buffer.alloc(d.length);
for (let i = 0; i < d.length; i++) o[i] = d[i] ^ (i < ks.length ? ks[i] : 0);
process.argv[3] ? fs.writeFileSync(process.argv[3], o) : process.stdout.write(o);
