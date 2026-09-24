const ts = require('typescript');
const fs = require('fs');
const catalog = ts.createSourceFile('en.ts', fs.readFileSync('src/locales/en.ts', 'utf8'), 99, true);
const keys = new Set();
function read(node) {
  if (ts.isPropertyAssignment(node) && ts.isStringLiteral(node.name)) keys.add(node.name.text);
  ts.forEachChild(node, read);
}
read(catalog);
const errors = [];
for (const path of ['src/App.tsx', ...fs.readdirSync('src/components').filter(x => x.endsWith('.tsx') && !x.includes('.test.')).map(x => 'src/components/' + x)]) {
  const source = ts.createSourceFile(path, fs.readFileSync(path, 'utf8'), 99, true, 4);
  function visit(node) {
    if (ts.isCallExpression(node) && node.expression.getText(source) === 'tr' && ts.isStringLiteral(node.arguments[0]) && !keys.has(node.arguments[0].text)) errors.push(`${path}: missing ${node.arguments[0].text}`);
    if (ts.isJsxText(node) && /[\u4e00-\u9fff]/.test(node.text) && node.text.trim() !== '中文') errors.push(`${path}: unlocalized JSX ${node.text.trim()}`);
    ts.forEachChild(node, visit);
  }
  visit(source);
}
if (errors.length) { console.error(errors.join('\n')); process.exitCode = 1; }
else console.log(`UI language catalog verified: ${keys.size} entries.`);
