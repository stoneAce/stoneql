import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { auditProblems, linuxExceptions } from '../tools/audit-exceptions.mjs';

const warning = (kind, id, name, version) => ({ kind, advisory: id ? { id } : null, package: { name, version } });
const report = (list = [], warnings = {}) => ({ vulnerabilities: { found: list.length > 0, count: list.length, list }, warnings });
const approved = () => ({
  unsound: [warning('unsound', 'RUSTSEC-2024-0429', 'glib', '0.18.5')],
  unmaintained: [warning('unmaintained', 'RUSTSEC-2024-0370', 'proc-macro-error', '1.0.4')],
});

test('the Linux release audit allows exactly the two approved advisories', () => {
  assert.deepEqual(linuxExceptions.map((exception) => exception.id).sort(), ['RUSTSEC-2024-0370', 'RUSTSEC-2024-0429']);
  for (const exception of linuxExceptions) {
    assert.ok(exception.reason.length > 40 && exception.path.startsWith('stoneql-desktop → '), exception.id);
  }
  assert.deepEqual(auditProblems(report([], approved()), linuxExceptions), []);
});

test('any other advisory fails, including a vulnerability or another version of an allowed crate', () => {
  const vulnerable = report([{ advisory: { id: 'RUSTSEC-2099-0001' }, package: { name: 'glib', version: '0.18.5' } }], approved());
  assert.deepEqual(auditProblems(vulnerable, linuxExceptions), ['not allowed: RUSTSEC-2099-0001 vulnerability glib 0.18.5']);
  const other = approved();
  other.unmaintained.push(warning('unmaintained', 'RUSTSEC-2099-0002', 'atk', '0.18.2'));
  other.yanked = [warning('yanked', null, 'gtk', '0.18.2')];
  assert.deepEqual(auditProblems(report([], other), linuxExceptions), ['not allowed: RUSTSEC-2099-0002 unmaintained atk 0.18.2', 'not allowed: (no advisory) yanked gtk 0.18.2']);
  const moved = approved();
  moved.unsound = [warning('unsound', 'RUSTSEC-2024-0429', 'glib', '0.19.0')];
  assert.deepEqual(auditProblems(report([], moved), linuxExceptions), ['not allowed: RUSTSEC-2024-0429 unsound glib 0.19.0', 'stale exception: RUSTSEC-2024-0429 (glib 0.18.5) is no longer reported; remove it']);
});

test('an exception that is no longer reported fails as stale, and a malformed report fails', () => {
  assert.deepEqual(auditProblems(report([], { unsound: approved().unsound }), linuxExceptions), ['stale exception: RUSTSEC-2024-0370 (proc-macro-error 1.0.4) is no longer reported; remove it']);
  assert.deepEqual(auditProblems(null, linuxExceptions), ['cargo audit report is missing or malformed']);
});

test('the macOS and Windows release audits have no exceptions', () => {
  const source = readFileSync(new URL('../tools/audit-release.mjs', import.meta.url), 'utf8');
  assert.match(source, /const exceptions = linux \? linuxExceptions : \[\];[\s\S]*auditProblems\(report, exceptions\)[\s\S]*\} else \{\n\s*const audit = spawnSync\('cargo', \['audit', '--file', file, \.\.\.auditArgs\]/);
  assert.doesNotMatch(source, /--ignore/);
});
