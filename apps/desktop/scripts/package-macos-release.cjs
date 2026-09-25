// Public artifacts only. Never copy a working tree, user home, or runtime data.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const app = path.resolve(__dirname, '..');
const native = path.join(app, 'src-tauri');
const version = JSON.parse(fs.readFileSync(path.join(native, 'tauri.conf.json'), 'utf8')).version;
const label = `Deadalus-${version}-macos-universal`;
const output = path.join(app, 'release-output', label);
if (process.platform !== 'darwin') throw new Error('Run on macOS after the universal build.');
if (fs.existsSync(output)) throw new Error('Refusing to overwrite an existing release directory.');
const dmgDir = path.join(native, 'target/universal-apple-darwin/release/bundle/dmg');
const images = fs.readdirSync(dmgDir).filter(name => name.endsWith('.dmg'));
if (images.length !== 1) throw new Error('Expected exactly one freshly built disk image.');
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: app, encoding: 'utf8' }).trim();
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--format-version', '1', '--filter-platform', 'aarch64-apple-darwin'], { cwd: native, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }));
const notices = ['Deadalus third-party dependency notices', 'Conservative inventory includes transitive/build dependencies.'];
let missing = 0;
function collect(name, version, license, dir, repository) {
  notices.push(`\n=== ${name} ${version} ===\nDeclared license: ${license || 'See upstream'}`);
  const files = fs.readdirSync(dir, { withFileTypes: true }).filter(e => e.isFile() && /^(licen[cs]e|copying|notice)([._-]|$)/i.test(e.name));
  if (files.length) {
    for (const file of files) notices.push(`\n--- ${file.name} ---\n${fs.readFileSync(path.join(dir, file.name), 'utf8')}`);
    return;
  }
  let recovered = false;
  const vcs = path.join(dir, '.cargo_vcs_info.json');
  if (repository?.startsWith('https://github.com/') && fs.existsSync(vcs)) {
    const sha = JSON.parse(fs.readFileSync(vcs, 'utf8')).git.sha1;
    const repo = repository.replace('https://github.com/', '').replace(/\/$/, '').replace(/\.git$/, '');
    for (const file of ['LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'LICENSE-BSD', 'LICENSE-MPL', 'LICENSE.txt']) {
      const url = `https://raw.githubusercontent.com/${repo}/${sha}/${file}`;
      try {
        const content = execFileSync('curl', ['-fLsS', '--max-time', '15', url], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
        notices.push(`\n--- ${url} ---\n${content}`); recovered = true;
      } catch { /* Try the next upstream license filename. */ }
    }
  }
  if (!recovered && license === 'MPL-2.0') {
    const url = 'https://www.mozilla.org/media/MPL/2.0/index.txt';
    const content = execFileSync('curl', ['-fLsS', '--max-time', '20', url], { encoding: 'utf8' });
    if (!content.includes('Mozilla Public License')) throw new Error('Invalid license response');
    notices.push(`\n--- ${url} ---\n${content}`); recovered = true;
  }
  if (!recovered) { missing++; notices.push('License text missing; consult upstream before distribution.'); }
}
for (const p of metadata.packages.filter(p => p.source)) collect(p.name, p.version, p.license, path.dirname(p.manifest_path), p.repository);
const lock = JSON.parse(fs.readFileSync(path.join(app, 'package-lock.json'), 'utf8'));
for (const [relative, pkg] of Object.entries(lock.packages)) {
  if (relative && !pkg.dev && fs.existsSync(path.join(app, relative))) collect(relative.split('node_modules/').pop(), pkg.version, pkg.license, path.join(app, relative));
}
if (missing) throw new Error(`Missing license texts for ${missing} dependencies; refusing to publish incomplete notices.`);
fs.mkdirSync(output, { recursive: true });
fs.copyFileSync(path.join(dmgDir, images[0]), path.join(output, `${label}.dmg`));
fs.writeFileSync(path.join(output, 'THIRD-PARTY-NOTICES.txt'), notices.join('\n'));
fs.writeFileSync(path.join(output, 'Quick-Start.txt'), `Deadalus V${version} — macOS 12 or newer, Apple Silicon and Intel\n\n打开 DMG，将 Deadalus 拖入 Applications，然后从应用程序打开。\n本包仅作 ad-hoc 签名，未经 Apple Developer 签名或公证。macOS 可能阻止打开；仅在核对来源和校验值后，按照系统“隐私与安全性”的提示处理，不要关闭 Gatekeeper。受组织策略限制时联系管理员。\n设置中的 Agent 和 Embedding 凭据分开保存；Mac 使用系统钥匙串。\n检索历史及应用数据保存在本机，扫描自己的 Skills；本包不带开发者的凭据、私有 Skills 或索引。分析/翻译/向量化会向所配置服务商发送文本并可能计费。\n\nOpen the DMG and drag Deadalus into Applications. This build is ad-hoc signed, NOT Developer ID signed or notarized. Verify its source before following macOS Privacy & Security guidance. Do not disable Gatekeeper.\nConfigure Agent and Embedding credentials separately. Credentials use macOS Keychain. Remote analysis may incur fees and upload required text. Back up Skills before overwrite/uninstall operations.\n`);
fs.writeFileSync(path.join(output, 'RELEASE-NOTES.txt'), `Deadalus V${version}\nSource commit: ${commit}\nUniversal macOS build (arm64 + x86_64), minimum macOS 12.0.\nIncludes Agent search history, pending state, renaming, editable recommendations and persistent view panning.\nCI validates frontend/native tests, both binary architectures, ad-hoc signature and disk-image integrity.\nNot Apple-notarized. Clean-machine installation, native UI, real Keychain interaction and paid model calls still require manual acceptance testing.\nNo automatic GitHub Release publication.\n`);
const sums = fs.readdirSync(output).sort().map(name => `${crypto.createHash('sha256').update(fs.readFileSync(path.join(output, name))).digest('hex')}  ${name}`);
fs.writeFileSync(path.join(output, 'SHA256SUMS.txt'), sums.join('\n') + '\n');
execFileSync('ditto', ['-c', '-k', '--keepParent', output, `${output}.zip`]);
console.log(JSON.stringify({ archive: `${output}.zip`, commit, files: fs.readdirSync(output) }));
