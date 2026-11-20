// Release-graph dependency audit (M002 alpha gate). `cargo audit` reads the whole Cargo.lock,
// including development/test-only packages (for example SQLite, kept for regressions, A5). This
// check audits only what the shipped macOS desktop build resolves: the normal+build dependency
// graph of `stoneql-desktop --features custom-protocol` for aarch64 and x86_64 Apple targets, or,
// with `--linux`, for x86_64-unknown-linux-gnu (the `.deb` built by `desktop:deb`), or, with
// `--windows`, for x86_64-pc-windows-msvc (the NSIS installer).
// It writes a pruned copy of Cargo.lock to a temporary directory and runs `cargo audit --file`
// on it. The full audit (`cargo audit`) stays separate and unchanged. The macOS audit ignores
// nothing. The Linux audit fails on any advisory (vulnerability or warning) except the approved
// exceptions in audit-exceptions.mjs, and fails when one of those is no longer reported. The
// Windows audit is as strict, with no exceptions.
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { auditProblems, linuxExceptions } from './audit-exceptions.mjs';

const root = resolve(import.meta.dirname, '..');
const linux = process.argv.includes('--linux');
const windows = process.argv.includes('--windows');
if (linux && windows) throw Error('Choose --linux or --windows');
const targets = linux ? ['x86_64-unknown-linux-gnu'] : windows ? ['x86_64-pc-windows-msvc'] : ['aarch64-apple-darwin', 'x86_64-apple-darwin'];
const auditArgs = process.argv.slice(2).filter((arg) => arg !== '--linux' && arg !== '--windows');
const run = (args) => {
  const result = spawnSync('cargo', args, { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (result.status !== 0) throw Error(`cargo ${args[0]} failed: ${result.stderr.trim().split('\n').at(-1)}`);
  return result.stdout;
};

const keep = new Set(); // "name version"
for (const target of targets) {
  const tree = run(['tree', '--locked', '-q', '-p', 'stoneql-desktop', '--features', 'custom-protocol', '--target', target, '-e', 'normal,build', '--prefix', 'none', '--format', '{p}']);
  for (const line of tree.split('\n')) {
    const match = /^(\S+) v(\S+)/.exec(line.trim());
    if (match) keep.add(`${match[1]} ${match[2]}`);
  }
}

const lock = readFileSync(join(root, 'Cargo.lock'), 'utf8');
const [header, ...blocks] = lock.split(/\n(?=\[\[package\]\]\n)/);
const field = (block, name) => new RegExp(`^${name} = "([^"]+)"`, 'm').exec(block)?.[1];
const kept = blocks.filter((block) => keep.has(`${field(block, 'name')} ${field(block, 'version')}`));
const names = new Map(); // name -> versions kept, to prune dependency references
for (const block of kept) names.set(field(block, 'name'), [...(names.get(field(block, 'name')) ?? []), field(block, 'version')]);
const pruned = kept.map((block) => block.replace(/^dependencies = \[\n([\s\S]*?)\n\]/m, (_, body) => {
  const deps = body.split('\n').filter((line) => {
    const [name, version] = line.trim().replace(/^"|",?$/g, '').split(' ');
    return names.has(name) && (!version || names.get(name).includes(version));
  });
  return deps.length ? `dependencies = [\n${deps.join('\n')}\n]` : 'dependencies = []';
}));
const missing = [...keep].filter((id) => !kept.some((block) => `${field(block, 'name')} ${field(block, 'version')}` === id));
if (missing.length) throw Error(`release graph packages missing from Cargo.lock: ${missing.join(', ')}`);

const temp = mkdtempSync(join(tmpdir(), 'stoneql-audit-'));
try {
  const file = join(temp, 'Cargo.lock');
  writeFileSync(file, [header, ...pruned].join('\n'));
  console.log(`release graph: ${kept.length} of ${blocks.length} locked packages (${targets.join(', ')})`);
  if (linux || windows) {
    const exceptions = linux ? linuxExceptions : [];
    const name = linux ? 'audit:release:linux' : 'audit:release:windows';
    const audit = spawnSync('cargo', ['audit', '--file', file, '--json', ...auditArgs], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
    let report;
    try { report = JSON.parse(audit.stdout.trim().split('\n').at(-1)); } catch { report = null; }
    const problems = auditProblems(report, exceptions);
    if (report) console.log(`${report.database?.['advisory-count']} advisories loaded; ${report.vulnerabilities.count} vulnerabilities`);
    for (const exception of exceptions) console.log(`allowed (approved exception) ${exception.id} ${exception.kind} ${exception.package} ${exception.version}: ${exception.path}. ${exception.reason}`);
    for (const problem of problems) console.error(problem);
    if (!report) console.error(audit.stderr.trim().split('\n').slice(-5).join('\n'));
    console.log(problems.length ? `${name} FAILED (${problems.length} problem(s))` : `${name} passed: no advisories${exceptions.length ? ' besides the approved exceptions' : ''}`);
    process.exitCode = problems.length ? 1 : 0;
  } else {
    const audit = spawnSync('cargo', ['audit', '--file', file, ...auditArgs], { cwd: root, stdio: 'inherit' });
    process.exitCode = audit.status ?? 1;
  }
} finally {
  rmSync(temp, { recursive: true, force: true });
}
