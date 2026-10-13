// Embedded in the reviewed mise lifecycle actions; every npm invocation uses argv.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const [mode, manifestPath, previousPrefix, policyJson] = process.argv.slice(1);
const node = fs.realpathSync(process.execPath);
const prefix = process.platform === 'win32' ? path.dirname(node) : path.dirname(path.dirname(node));
const root = path.join(prefix, process.platform === 'win32' ? 'node_modules' : 'lib/node_modules');
const npmCli = path.join(root, 'npm/bin/npm-cli.js');
const env = { ...process.env, PATH: `${path.dirname(node)}${path.delimiter}${process.env.PATH || ''}` };

function npm(args, capture = true) {
  const result = spawnSync(node, [npmCli, ...args, '--prefix', prefix], {
    env, encoding: 'utf8', stdio: capture ? ['ignore', 'pipe', 'pipe'] : ['ignore', 'inherit', 'inherit'],
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`npm ${args[0]} 失败（${result.status ?? result.signal}）${capture ? `: ${result.stderr || ''}` : ''}`);
  return result.stdout;
}

function config(name) {
  const value = npm(['config', 'get', name, '--global', '--json']).trim();
  return value === 'undefined' || value === '' ? null : JSON.parse(value);
}

function allowList(value) {
  if (value == null) return [];
  const values = Array.isArray(value) ? value : [value];
  if (values.some(item => typeof item !== 'string')) throw new Error('无法读取 npm allow-scripts 配置');
  return [...new Set(values.flatMap(item => item.split(',')).map(item => item.trim()).filter(Boolean))];
}

function validName(name) {
  return /^(?:@[a-z0-9_.-]+\/)?[a-z0-9][a-z0-9_.-]*$/i.test(name) && name.length <= 214;
}

function save(manifest, exclusive = false) {
  fs.mkdirSync(path.dirname(manifestPath), { recursive: true, mode: 0o700 });
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), { flag: exclusive ? 'wx' : 'w', mode: 0o600 });
}

function snapshot() {
  if (fs.realpathSync(previousPrefix) !== prefix) throw new Error('Node 环境与待升级的全局安装不一致，已停止更新');
  const listing = JSON.parse(npm(['ls', '--global', '--depth=0', '--long', '--json']));
  if (!listing || typeof listing !== 'object' || Array.isArray(listing) || listing.error) throw new Error('无法读取全局 npm 包清单');
  if (listing.dependencies !== undefined && (!listing.dependencies || typeof listing.dependencies !== 'object' || Array.isArray(listing.dependencies))) throw new Error('全局 npm 包清单格式无效');
  const policies = JSON.parse(policyJson);
  const configured = allowList(config('allow-scripts'));
  const packages = Object.entries(listing.dependencies || {}).map(([name, entry]) => {
    if (!validName(name) || !entry || typeof entry.version !== 'string') throw new Error(`全局 npm 包清单不完整：${name}`);
    const installed = path.join(root, name);
    // Preserve npm-linked local packages as local sources, never substitute a registry namesake.
    const linked = fs.lstatSync(installed).isSymbolicLink();
    return {
      name, version: entry.version, source: linked ? fs.realpathSync(installed) : `${name}@latest`,
      allowScripts: [...new Set([...configured, ...allowList(policies[name])])],
    };
  });
  const manifest = {
    schemaVersion: 1, capturedAt: new Date().toISOString(), previousPrefix: prefix,
    configuredAllowScripts: configured, ignoreScripts: config('ignore-scripts') === true,
    packages, results: [],
  };
  save(manifest, true);
  console.log(`已保存 ${packages.length} 个全局 npm 包及 allow-scripts：${manifestPath}`);
  for (const pkg of packages) console.log(`  ${pkg.name}@${pkg.version} → ${pkg.source}`);
}

function clearCorepackShims(name) {
  const removed = [];
  const bins = name === 'pnpm' ? ['pnpm', 'pnpx'] : name === 'yarn' ? ['yarn', 'yarnpkg'] : [];
  for (const bin of bins) {
    const shim = path.join(prefix, 'bin', bin);
    const expected = path.join(root, 'corepack', 'dist', `${bin}.js`);
    // Only replace a verified Corepack symlink when restoring the real package.
    // Unrelated executable files remain protected by npm's normal EEXIST check.
    if (fs.existsSync(shim) && fs.lstatSync(shim).isSymbolicLink()
      && fs.existsSync(expected) && fs.realpathSync(shim) === fs.realpathSync(expected)) {
      removed.push({ path: shim, target: fs.readlinkSync(shim) });
      fs.unlinkSync(shim);
    }
  }
  return removed;
}

function restore() {
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  if (manifest.schemaVersion !== 1 || manifest.previousPrefix !== fs.realpathSync(previousPrefix)
    || !Array.isArray(manifest.packages)) throw new Error('全局 npm 包迁移清单无效');
  manifest.restoredPrefix = prefix;
  manifest.results = [];
  // npm first so later installs use its script policy; Corepack before the actual package managers.
  const priority = name => name === 'npm' ? 0 : name === 'corepack' ? 1 : 2;
  const packages = [...manifest.packages].sort((a, b) => priority(a.name) - priority(b.name));
  for (const pkg of packages) {
    let removed = [];
    try {
      if (!validName(pkg.name) || (pkg.source !== `${pkg.name}@latest` && !path.isAbsolute(pkg.source))) throw new Error('包来源无效');
      removed = clearCorepackShims(pkg.name);
      const args = ['install', '--global', '--loglevel=info', '--foreground-scripts'];
      if (manifest.ignoreScripts) args.push('--ignore-scripts');
      else args.push('--strict-allow-scripts');
      const allowed = allowList(pkg.allowScripts);
      if (allowed.length) args.push(`--allow-scripts=${allowed.join(',')}`);
      // --prefix must precede --; npm receives package sources as data, not options.
      console.log(`恢复 ${pkg.name}：${pkg.source}${allowed.length ? `（allow-scripts: ${allowed.join(',')}）` : ''}`);
      const result = spawnSync(node, [npmCli, ...args, '--prefix', prefix, '--', pkg.source], {
        env, stdio: ['ignore', 'inherit', 'inherit'],
      });
      if (result.error || result.status !== 0) throw result.error || new Error(`npm 退出码 ${result.status ?? result.signal}`);
      manifest.results.push({ name: pkg.name, status: 'succeeded' });
    } catch (error) {
      for (const shim of removed) {
        try { fs.symlinkSync(shim.target, shim.path); }
        catch (restoreError) { if (restoreError.code !== 'EEXIST') console.error(`无法恢复 ${shim.path}：${restoreError.message}`); }
      }
      console.error(`恢复 ${pkg.name} 失败：${error.message}`);
      manifest.results.push({ name: pkg.name, status: 'failed', error: error.message });
    }
    save(manifest);
  }
  const failed = manifest.results.filter(item => item.status === 'failed');
  if (failed.length) throw new Error(`${failed.length} 个全局 npm 包恢复失败，清单已保留：${manifestPath}。如果安装脚本被阻止，请在 DBox 设置中记录该 CLI 的 allow-scripts 后重试。`);
  console.log(`已在 ${prefix} 恢复 ${packages.length} 个全局 npm 包；清单：${manifestPath}`);
}

try {
  if (mode === 'snapshot') snapshot();
  else if (mode === 'restore') restore();
  else throw new Error('未知的 Node 迁移动作');
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
