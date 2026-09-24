// Assemble only explicitly approved release artifacts, never the working tree.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const app = path.resolve(__dirname, '..');
const native = path.join(app, 'src-tauri');
const config = JSON.parse(fs.readFileSync(path.join(native, 'tauri.conf.json'), 'utf8'));
const version = config.version;
const releaseLabel = `V${version}`;
const output = path.resolve(process.argv[2] || '');
if (!process.argv[2] || path.basename(output) !== `Deadalus-${version}-windows-x64`) {
  throw new Error('Specify a new Deadalus-<version>-windows-x64 output directory.');
}
if (fs.existsSync(output) && process.argv[3] !== '--refresh-generated') throw new Error('Output already exists; refusing to overwrite.');
const installer = path.join(native, 'target', 'release', 'bundle', 'nsis', `Deadalus_${version}_x64-setup.exe`);
if (!fs.statSync(installer).isFile()) throw new Error('Release installer missing');
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: app, encoding: 'utf8' }).trim();
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--offline', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc'], { cwd: native, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }));
const notices = ['Deadalus third-party dependency notices', 'Includes build-time/transitive components as a conservative inventory.', 'License texts are copied from locally installed dependency distributions.', ''];
let missingTexts = 0;
function addNotice(name, version, license, dir, repository) {
  notices.push(`\n=== ${name} ${version} ===\nDeclared license: ${license || 'See upstream distribution'}`);
  const files = fs.readdirSync(dir, { withFileTypes: true }).filter(e => e.isFile() && /^(licen[cs]e|copying|notice)([._-]|$)/i.test(e.name));
  if (!files.length) {
    let recovered = false;
    const vcs = path.join(dir, '.cargo_vcs_info.json');
    if (repository?.startsWith('https://github.com/') && fs.existsSync(vcs)) {
      const sha = JSON.parse(fs.readFileSync(vcs, 'utf8')).git.sha1;
      const repo = repository.replace('https://github.com/', '').replace(/\/$/, '').replace(/\.git$/, '');
      notices.push(`Upstream source: https://github.com/${repo}/tree/${sha}`);
      for (const filename of ['LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'LICENSE-BSD', 'LICENSE-MPL', 'LICENSE.txt']) {
        const url = `https://raw.githubusercontent.com/${repo}/${sha}/${filename}`;
        try {
          const text = execFileSync('curl.exe', ['--fail', '--silent', '--show-error', '--location', '--max-time', '15', url], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
          notices.push(`\n--- ${url} ---\n${text}`);
          recovered = true;
        } catch { /* Try other upstream license filenames. */ }
      }
    }
    if (!recovered && license === 'MPL-2.0') {
      const url = 'https://www.mozilla.org/media/MPL/2.0/index.txt';
      const text = execFileSync('curl.exe', ['--fail', '--silent', '--show-error', '--location', '--max-time', '15', url], { encoding: 'utf8' });
      if (!text.includes('Mozilla Public License')) throw new Error('Invalid MPL license response');
      notices.push(`\n--- ${url} ---\n${text}`);
      recovered = true;
    }
    if (!recovered) { missingTexts++; notices.push('No license text found; consult the upstream package for full terms.'); }
  }
  for (const file of files) notices.push(`\n--- ${file.name} ---\n${fs.readFileSync(path.join(dir, file.name), 'utf8')}`);
}
for (const pkg of metadata.packages.filter(p => p.source)) addNotice(pkg.name, pkg.version, pkg.license, path.dirname(pkg.manifest_path), pkg.repository);
const lock = JSON.parse(fs.readFileSync(path.join(app, 'package-lock.json'), 'utf8'));
for (const [relative, pkg] of Object.entries(lock.packages)) {
  if (!relative || pkg.dev) continue;
  const dir = path.join(app, relative);
  if (fs.existsSync(dir)) addNotice(relative.split('node_modules/').pop(), pkg.version, pkg.license, dir);
}
fs.mkdirSync(output, { recursive: true });
const setupName = `Deadalus-${version}-windows-x64-setup.exe`;
fs.copyFileSync(installer, path.join(output, setupName));
const write = (name, content) => fs.writeFileSync(path.join(output, name), '\ufeff' + content.replace(/\r?\n/g, '\r\n'), 'utf8');
write('开始使用-Quick-Start.txt', `Deadalus ${releaseLabel} — Windows x64 发布版

安装
1. 适用于 Windows 10/11 64 位（x64），本包不是 ARM64 原生版本。
2. 双击 ${setupName}，选择中文或英文，按向导安装。
3. 安装完成后从开始菜单打开 Deadalus。不需要安装 Node.js、Rust 或开发环境。
4. 应用依赖 Microsoft Edge WebView2。安装包内置引导程序；如系统缺少运行时，需要联网下载。
5. 本版本未做代码签名，Windows 可能显示“未知发布者”或安全提示。请仅从作者发布的仓库下载并核对 SHA-256；不要关闭系统防护。受组织策略限制时联系管理员。

首次使用
1. 左上角 D 打开设置，通用设置可切换中文/英文。
2. apikeys管理中选择服务商、输入 Key，并选择 Agent 或 Embedding 用途后保存、启用。
   同一个 Key 如需两种用途，必须分别保存。请使用 API 平台 Key，不是网页聊天登录信息。
3. LLM 分析需要 Agent 凭据；向量化需要 OpenAI 或 Qwen 的 Embedding 凭据。
4. 创建 Embedding Profile，填写名称和简介，模型配置自动生成。
5. 在“索引与同步”中扫描变更或全量重建，确认估算后开始。模型调用需要网络并可能产生服务商费用。
6. 可查看 Skills 类别和向量图，整理自定义类别及项目专用 Skills。

数据与隐私
- 发布包不含作者的 API Key、个人 Skills、副本库、向量数据库、任务日志、缓存或开发文档。
- 首次使用会在本机创建数据，读取本机配置的 Agent Skills；不会携带作者的测试索引。
- 用户数据位于 Windows 用户应用数据目录中的 app.deadalus.desktop（通常为 %APPDATA%\\app.deadalus.desktop）。
- Key 密文凭据由 Windows Credential Manager 管理；本地元数据仅保存用途和掩码等信息。
- 扫描文件是本地操作；LLM 分析、翻译和向量化会向所配置服务商发送所需的 Skill 文本/用户请求。请自行确认上传权限和敏感内容。
- 任务日志可能记录请求及分析结果；反馈问题前请脱敏，不要上传整个数据目录。
- 覆盖复制、卸载会修改实际 Agent/项目 Skills，请先备份并仔细阅读二次确认。已知是初步发布版，建议先使用测试项目。

故障与卸载
- 模型调用失败：检查对应用途 Key 是否启用、模型权限、余额、网络和区域支持。
- 窗口无法显示：检查 WebView2 是否正常安装。本发布版不依赖 localhost:1420 开发服务器。
- 用 Windows“已安装的应用”卸载；如涉及本地数据清理，请先备份，不要直接删除仍在使用的项目 Skills。
- 问题反馈：https://github.com/aawang1/deadalus-skills-manager/issues

English quick start
Run the x64 setup executable. Node.js and Rust are not required. Missing WebView2 requires internet access.
The installer is unsigned; verify the download source and SHA-256. Do not disable Windows protection.
Open settings using D. Save and enable Agent and Embedding credentials separately, create a named Profile,
then review the estimate before rebuilding the index. Cloud API calls may incur charges and send Skill text
to your selected providers. No developer keys, personal Skills or index data are bundled.
Back up Skills before overwrite/uninstall operations. This is an initial release; test on a non-critical project first.
`);
write('RELEASE-NOTES.txt', `Deadalus ${releaseLabel} — Windows x64 release
Release title/tag: ${releaseLabel} (suggested; not created automatically)
Base source commit: ${commit}; includes local version and packaging updates.
Packaging overlay: src-tauri/tauri.release.conf.json

Included: Skill library and project categories; Agent-assisted organization; embedding Profiles and visualization;
Chinese/English UI; purpose-scoped API keys; automatic embedding configuration; blue D application icon.

Distribution: current-user NSIS installer, Chinese/English installation UI, embedded online WebView2 bootstrapper.
Signing: not code-signed. No automatic GitHub upload was performed.
Verification: frontend tests/build/i18n checks and Rust unit tests passed before packaging.
Limitations: clean-machine installation, signing, and live paid model calls are not covered by this build verification.
Third-party notices inventory includes ${missingTexts} dependencies without a root-level license text;
review upstream license terms before broad public distribution. No new license for the application's own code is granted here.
`);
write('THIRD-PARTY-NOTICES.txt', notices.join('\n'));
const sums = fs.readdirSync(output).filter(name => name !== 'SHA256SUMS.txt').sort().map(name => `${crypto.createHash('sha256').update(fs.readFileSync(path.join(output, name))).digest('hex')}  ${name}`);
fs.writeFileSync(path.join(output, 'SHA256SUMS.txt'), sums.join('\n') + '\n');
console.log(JSON.stringify({ output, installer: setupName, dependencyLicenseTextsMissing: missingTexts, files: fs.readdirSync(output) }, null, 2));
