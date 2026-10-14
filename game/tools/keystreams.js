// Where the locally derived keystreams live (git-ignored; made by make_keystreams.js from the user's own game files).
const path = require('path'), fs = require('fs');
const dir = path.resolve(__dirname, '../.cache/keystreams');
module.exports = {
  dir,
  file: n => path.join(dir, n),
  need(n) {
    const f = path.join(dir, n);
    if (!fs.existsSync(f)) { console.error(`${f} is missing. Run: node tools/make_keystreams.js`); process.exit(1); }
    return fs.readFileSync(f);
  },
};
