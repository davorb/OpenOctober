#!/usr/bin/env node
// Derives, from YOUR OWN unpacked game files in ../../export, the position-indexed byte streams that tools/unpack_all.js needs
// to make the archive's xml-files/ and shader sources readable. Nothing here ships with the repository: the results are written
// to game-viewer/.cache/keystreams/ (git-ignored) and are specific to the copy of the game you point this at.
//
//   node tools/make_keystreams.js            # about 1 min; skips streams that already exist
//   node tools/make_keystreams.js --force    # derive them again
const fs = require('fs'), path = require('path'), { spawnSync } = require('child_process');
const K = require('./keystreams'), EXPORT = path.resolve(__dirname, '../../export');
const force = process.argv.includes('--force');

if (!fs.existsSync(path.join(EXPORT, 'xml-files')) || !fs.existsSync(path.join(EXPORT, 'xml'))) {
  console.error('export/xml-files and export/xml are missing. Unpack your own copy of the game first (see the README, step 1).');
  process.exit(1);
}
fs.mkdirSync(K.dir, { recursive: true });
// nearly every file starts like this, which pins down the first bytes where there is too little text to go on
const XML_HEAD = '<?xml version=\\"1.0\\" encoding=\\"UTF-8\\" standalone=\\"yes\\"?>\\r\\n';
const run = (file, ...a) => {
  const r = spawnSync(process.execPath, [path.join(__dirname, file), ...a], { stdio: ['ignore', 'ignore', 'inherit'], maxBuffer: 1 << 28 });
  if (r.status !== 0) { console.error(`${file} failed (${r.status})`); process.exit(1); }
};
const step = (out, what, fn) => {
  if (!force && fs.existsSync(K.file(out))) return console.log(`${out}: already there`);
  console.log(`${out}: ${what}`); fn(); console.log(`${out}: ${fs.statSync(K.file(out)).size} bytes`);
};
step('keystream.bin', 'xml-files', () => run('solve_keystream.js', path.join(EXPORT, 'xml-files'), K.file('keystream.bin'), '1100000', '--exclude', 'shaders', '--self', '2', '--header', XML_HEAD));
step('keystream_tail.bin', 'the few xml-files longer than 1.1 MB', () => run('solve_tail.js', K.file('keystream_tail.bin'), '1', '11300000'));
step('ks_shader.bin', 'shaders', () => run('solve_keystream.js', path.join(EXPORT, 'xml-files/shaders'), K.file('ks_shader.bin'), '100000', '--self', '4'));
