const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');

const sourceRoot = process.argv[2];
if (!sourceRoot) throw new Error('Pass the Tauri project directory');
const ts = createRequire(path.join(sourceRoot, 'package.json'))('typescript');
const sourcePath = path.join(sourceRoot, 'src', 'stickers', 'zh.ts');
const source = ts.createSourceFile(
  sourcePath,
  fs.readFileSync(sourcePath, 'utf8'),
  ts.ScriptTarget.Latest,
  true,
);
const data = {};
function visit(node) {
  if (ts.isVariableDeclaration(node) &&
      (node.name.text === 'ZH' || node.name.text === 'CN_SYNONYMS')) {
    const entries = {};
    for (const property of node.initializer.properties) {
      const key = property.name.text;
      const value = property.initializer;
      entries[key] = ts.isArrayLiteralExpression(value)
        ? value.elements.map((item) => item.text)
        : value.text;
    }
    data[node.name.text] = entries;
  }
  ts.forEachChild(node, visit);
}
visit(source);
if (!data.ZH || !data.CN_SYNONYMS) throw new Error('Sticker dictionaries missing');
fs.writeFileSync(
  path.join(__dirname, '..', 'assets', 'sticker_zh.json'),
  JSON.stringify(data),
);
