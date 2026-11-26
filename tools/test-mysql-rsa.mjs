// TLS-off RSA sign-in against real MySQL servers with an empty authentication cache
// (`npm run test:mysql-rsa`). Each run owns its containers and removes them.
//   - MySQL 8.4.6: `caching_sha2_password` handshake.
//   - MySQL 8.0.46 with `--default-authentication-plugin=mysql_native_password`: every
//     `caching_sha2_password` sign-in goes through an authentication switch (regression test for
//     the vendored driver's switch-nonce fix, experiments/sqlx-rsa-key/).
// Each server is checked fresh, then restarted (same data directory and key pair) and checked
// again. `--network-address` (Linux only) connects to the container's own address on the Docker
// bridge instead of a published loopback port: a real non-loopback address, never published.
import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { connect, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { randomBytes } from 'node:crypto';
import { assertLocalFixtureDocker, fixtureEnvironment } from './fixture-environment.mjs';

const root = resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
if (args.some((argument) => argument !== '--network-address')) throw Error('Unknown option');
const networkAddress = args.includes('--network-address');
if (networkAddress && process.platform !== 'linux') throw Error('--network-address needs Docker on Linux (container addresses are routable from the host)');
const base = fixtureEnvironment(process.env);
const run = (command, argv, env = base, { quiet = false } = {}) => {
  const result = spawnSync(command, argv, { cwd: root, env, encoding: 'utf8', timeout: 300_000 });
  if (result.status !== 0 || result.error) throw Error(quiet ? `${command} fixture operation failed; credentials and raw errors withheld` : `${command} failed: ${result.stderr || result.error?.message}`);
  return result.stdout.trim();
};
assertLocalFixtureDocker(base, (context) => run('docker', ['context', 'inspect', ...(context ? [context] : []), '--format', '{{.Endpoints.docker.Host}}']));

const servers = [
  { name: 'mysql84', image: 'mysql:8.4.6@sha256:869218921e61d6c3c89820955d63cca42971f0e3e6c1e2792247bbd944ebc6e9', args: [], handshake: 'caching_sha2_password' },
  { name: 'mysql80', image: 'mysql:8.0.46@sha256:7dcddc01f13bab2f15cde676d44d01f61fc9f99fe7785e86196dfc07d358ae2b', args: ['--default-authentication-plugin=mysql_native_password'], handshake: 'mysql_native_password' },
];

run('cargo', ['build', '--locked', '-p', 'stoneql-runtime', '--example', 'rsa_sign_in']);
const binary = resolve(root, 'target/debug/examples/rsa_sign_in');

// A free loopback port, published fixed so the server keeps its address across a restart.
const freePort = () => new Promise((done, fail) => {
  const server = createServer().once('error', fail).listen(0, '127.0.0.1', () => { const { port } = server.address(); server.close(() => done(port)); });
});
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
// The plugin named in the server's initial handshake packet.
function handshakePlugin(host, port) {
  return new Promise((done, fail) => {
    const socket = connect(port, host);
    socket.setTimeout(10_000, () => { socket.destroy(); fail(Error('handshake timeout')); });
    socket.once('data', (data) => {
      socket.destroy();
      done(['caching_sha2_password', 'mysql_native_password', 'sha256_password'].find((name) => data.includes(name)) ?? 'unknown');
    });
    socket.once('error', fail);
  });
}

const temp = mkdtempSync(join(tmpdir(), 'stoneql-mysql-rsa-'));
const report = { networkAddress, servers: [] };
const owned = [];
try {
  const password = randomBytes(24).toString('hex');
  const init = join(temp, 'init');
  mkdirSync(init);
  chmodSync(init, 0o755);
  const users = ['rsa_none', 'rsa_wrong', 'rsa_trusted', 'rsa_retrieve'].map((user) => `CREATE USER '${user}'@'%' IDENTIFIED WITH caching_sha2_password BY '${password}'; GRANT SELECT ON stoneql_rsa.* TO '${user}'@'%';`);
  writeFileSync(join(init, '001-rsa.sql'), [...users, `CREATE USER 'rsa_ssl'@'%' IDENTIFIED WITH caching_sha2_password BY '${password}' REQUIRE SSL; GRANT SELECT ON stoneql_rsa.* TO 'rsa_ssl'@'%';`].join('\n') + '\n', { mode: 0o644 });
  const env = { ...base, STONEQL_FIXTURE_PASSWORD: password };
  for (const server of servers) {
    const name = `stoneql-rsa-${randomBytes(6).toString('hex')}-${server.name}`;
    const hostPort = networkAddress ? 0 : await freePort();
    owned.push(name);
    run('docker', ['run', '-d', '--name', name, '--label', 'stoneql.fixture=mysql-rsa', '--memory', '1g',
      '-e', 'MYSQL_ROOT_PASSWORD', '-e', 'MYSQL_DATABASE=stoneql_rsa', '-v', `${init}:/docker-entrypoint-initdb.d:ro`,
      ...(networkAddress ? [] : ['-p', `127.0.0.1:${hostPort}:3306`]), server.image, ...server.args], { ...env, MYSQL_ROOT_PASSWORD: password }, { quiet: true });
    const ready = async () => {
      const deadline = Date.now() + 180_000;
      // Over TCP inside the container: the entrypoint's socket-only init server does not count.
      while (Date.now() < deadline) {
        const probe = spawnSync('docker', ['exec', name, 'sh', '-c', 'MYSQL_PWD="$MYSQL_ROOT_PASSWORD" exec mysql -uroot --protocol=TCP -h127.0.0.1 -N -e "SELECT 1"'], { encoding: 'utf8', timeout: 10_000 });
        if (probe.status === 0 && probe.stdout.trim() === '1') break;
        await new Promise((done) => setTimeout(done, 1000));
      }
      if (Date.now() >= deadline) throw Error(`${server.name}: fixture not ready`);
      if (networkAddress) {
        const address = run('docker', ['inspect', '--format', '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}', name]);
        if (!/^\d+\.\d+\.\d+\.\d+$/.test(address) || address.startsWith('127.')) throw Error(`${server.name}: no non-loopback container address`);
        return { host: address, port: 3306 };
      }
      const published = run('docker', ['port', name, '3306']);
      if (published !== `127.0.0.1:${hostPort}`) throw Error('Loopback fixture required');
      return { host: '127.0.0.1', port: hostPort };
    };
    let endpoint = await ready();
    const plugin = await handshakePlugin(endpoint.host, endpoint.port);
    if (plugin !== server.handshake) throw Error(`${server.name}: handshake offers ${plugin}, expected ${server.handshake}`);
    const key = join(temp, `${server.name}-public-key.pem`);
    writeFileSync(key, run('docker', ['exec', name, 'cat', '/var/lib/mysql/public_key.pem']) + '\n');
    const steps = [];
    for (const step of ['first', 'after-restart']) {
      if (step === 'after-restart') {
        run('docker', ['restart', name]);
        endpoint = await ready();
      }
      const result = spawnSync(binary, [], { cwd: root, env: { ...env, STONEQL_RSA_HOST: endpoint.host, STONEQL_RSA_PORT: String(endpoint.port), STONEQL_SERVER_PUBLIC_KEY: key, STONEQL_RSA_STEP: step }, encoding: 'utf8', timeout: 180_000 });
      if (result.status !== 0) throw Error(`${server.name} ${step}: ${`${result.stdout}${result.stderr}`.replaceAll(password, '[redacted]').trim()}`);
      steps.push(JSON.parse(result.stdout.trim().split('\n').at(-1)));
      console.log(`${server.name}: TLS-off RSA sign-in passed (${step}; ${networkAddress ? 'container network address' : 'published loopback port'}; handshake ${plugin})`);
    }
    // One handle across the pool's idle timeout and a server restart (see examples/rsa_sign_in.rs).
    const held = spawn(binary, [], { cwd: root, env: { ...env, STONEQL_RSA_HOST: endpoint.host, STONEQL_RSA_PORT: String(endpoint.port), STONEQL_SERVER_PUBLIC_KEY: key, STONEQL_RSA_STEP: 'recover' }, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = '';
    held.stdout.on('data', (data) => { output += data; });
    held.stderr.on('data', (data) => { output += data; });
    const exited = new Promise((done) => held.once('close', done));
    try {
      for (let waited = 0; !output.includes('HELD'); waited += 500) {
        if (waited > 120_000 || held.exitCode !== null) throw Error(`${server.name} recover: no handle (${output.replaceAll(password, '[redacted]').trim()})`);
        await sleep(500);
      }
      await sleep(70_000);
      const connections = run('docker', ['exec', name, 'sh', '-c', `MYSQL_PWD="$MYSQL_ROOT_PASSWORD" exec mysql -uroot -N -e "SELECT COUNT(*) FROM information_schema.PROCESSLIST WHERE USER = 'rsa_trusted'"`], { ...env, MYSQL_ROOT_PASSWORD: password }, { quiet: true });
      if (connections !== '0') throw Error(`${server.name} recover: ${connections} connection(s) still held after 70 s idle`);
      run('docker', ['restart', name]);
      const after = await ready();
      if (after.host !== endpoint.host || after.port !== endpoint.port) throw Error(`${server.name} recover: the server address changed`);
      held.stdin.end('\n');
      const code = await exited;
      if (code !== 0) throw Error(`${server.name} recover: ${output.replaceAll(password, '[redacted]').trim()}`);
    } finally { held.kill(); }
    steps.push({ step: 'recover', idleConnections: 0, passed: true });
    console.log(`${server.name}: one handle stayed usable across 70 s idle (no server connection held) and a server restart`);
    report.servers.push({ server: server.name, image: server.image, handshake: plugin, steps });
  }
} finally {
  for (const name of owned) spawnSync('docker', ['rm', '-f', '-v', name], { encoding: 'utf8' });
  const left = run('docker', ['ps', '-a', '--filter', 'label=stoneql.fixture=mysql-rsa', '-q']);
  rmSync(temp, { recursive: true, force: true });
  report.cleanup = { remainingContainers: left ? left.split('\n').length : 0 };
  console.log(`STONEQL_RSA_REPORT ${JSON.stringify(report)}`);
  if (left) throw Error('Owned fixture containers remain');
}
