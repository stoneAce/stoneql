import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createPinia, setActivePinia } from 'pinia';
import { loadModule } from './helpers/vueHost.ts';
const preferences = new Map<string, string>();
Object.assign(globalThis, { localStorage: { getItem: (key: string) => preferences.get(key) ?? null, setItem: (key: string, value: string) => preferences.set(key, value) } });
const { useWorkspace } = await loadModule('../../src/stores/workspace.ts');
const { bootstrapNativeHost } = await loadModule('../../src/services/nativeHost.ts');
const { connections } = await loadModule('../../src/data/sample.ts');

const empty = { index: 0, columns: [], rows: [], affectedRows: '0', truncated: false };
/**
 * A desktop host whose handles end like the native runtime's: `expire()` ends every open
 * handle (its one-hour lifetime), `lose()` makes the next query fail as a dropped connection.
 */
function host() {
  const calls: Array<{ command: string; payload: any }> = [], open = new Set<string>();
  let handles = 0, lost = false;
  const reply = (command: string, payload: any) => {
    switch (command) {
      case 'host_info': return { runtime: 'desktop', appVersion: '0.1.0', sessionId: 'session_1', nonce: payload.nonce, simulated: true };
      case 'contract_probe': return { values: payload.values, simulated: true };
      case 'list_unresolved_outcomes': return { outcomes: [], limit: 16 };
      case 'connect_direct': { const handle = `h${++handles}`; open.add(handle); return { handle, engineVersion: '8.4', database: 'shop', expiresAfterSeconds: 3600, access: payload.access ?? 'read_only' }; }
      case 'switch_database': if (!open.has(payload.handle)) throw { code: 'FORBIDDEN', message: 'no' }; return { database: payload.database };
      case 'database_status': return { connected: open.has(payload.handle) };
      case 'disconnect_database': open.delete(payload.handle); return { connected: false };
      case 'read_only_query':
        if (lost) { lost = false; open.delete(payload.handle); throw { code: 'CONNECTION_UNAVAILABLE', message: 'The database connection was lost.' }; }
        if (!open.has(payload.handle)) throw { code: 'FORBIDDEN', message: 'The live session is no longer available.' };
        return empty;
      default: throw { code: 'INTERNAL', message: command };
    }
  };
  const global = { __TAURI__: { core: { async invoke(command: string, args: { message: string }) {
    const request = JSON.parse(args.message);
    calls.push({ command, payload: request.payload });
    return { version: 1, requestId: request.requestId, payload: reply(command, request.payload) };
  } } } };
  return { global, calls, expire: () => open.clear(), lose: () => { lost = true; } };
}
async function connected(native: ReturnType<typeof host>, overrides: Record<string, unknown> = {}, access = 'read_only') {
  preferences.clear();
  await bootstrapNativeHost(native.global);
  setActivePinia(createPinia());
  const store = useWorkspace();
  const profile = { ...connections[0], id: 'live1', mode: 'live', engine: 'MySQL', host: 'db.test', port: 3306, database: 'shop', environment: 'Development', transport: { type: 'direct' }, passwordRef: 'ref_db', ...overrides };
  store.connections.push(profile);
  store.connect(profile.id);
  // As the connection dialog does: the typed password is used once, then only the saved profile remains.
  await store.connectLiveProfile({ ...profile, password: 'typed' }, undefined, access);
  await store.switchLiveDatabase(profile.id, 'reports');
  return store;
}
const connects = (native: ReturnType<typeof host>) => native.calls.filter((c) => c.command === 'connect_direct');

test('an expired live session reconnects by itself with the saved password, same access and database', async () => {
  const native = host();
  const store = await connected(native);
  native.expire();
  await store.refreshLiveStatus('live1');
  assert.equal(store.liveSessions.live1.status, 'connected');
  assert.equal(store.liveSessions.live1.info.handle, 'h2');
  assert.equal(store.liveSessions.live1.info.database, 'reports', 'the database the user switched to is kept');
  const again = connects(native).at(-1)!.payload;
  assert.equal(again.password, '', 'the typed password is not kept in memory');
  assert.equal(again.passwordRef, 'ref_db');
  assert.equal(again.access, undefined, 'read-only stays read-only');
  await store.executeLive('live1', 'SELECT 1');
});

test('a query on a handle that ended before the status check reconnects and runs once more', async () => {
  const native = host();
  const store = await connected(native);
  native.expire();
  await store.executeLive('live1', 'SELECT 1');
  assert.equal(store.liveSessions.live1.info.handle, 'h2');
  native.lose();
  await store.executeLive('live1', 'SELECT 1');
  assert.equal(store.liveSessions.live1.info.handle, 'h3');
  assert.equal(connects(native).length, 3);
});

test('no automatic reconnect after an explicit disconnect, without a saved password, or for Production read-write', async () => {
  let native = host();
  let store = await connected(native);
  await store.disconnectLiveProfile('live1');
  await assert.rejects(store.executeLive('live1', 'SELECT 1'), /Connect this live profile/);
  assert.equal(connects(native).length, 1);

  native = host();
  store = await connected(native, { passwordRef: undefined });
  native.expire();
  await store.refreshLiveStatus('live1');
  assert.equal(store.liveSessions.live1.status, 'disconnected');
  await assert.rejects(store.executeLive('live1', 'SELECT 1'), /password is not saved/);
  assert.equal(connects(native).length, 1);

  native = host();
  store = await connected(native, { environment: 'Production' }, 'read_write');
  native.expire();
  await store.refreshLiveStatus('live1');
  await assert.rejects(store.executeLive('live1', 'SELECT 1'), /Production/);
  assert.equal(connects(native).length, 1);
});
