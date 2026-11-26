<script setup lang="ts">
import { useConnectionWorkflow, reportLeaseEnd, LEASE_DISCONNECT_WARNING } from './composables/useConnectionWorkflow';
import { hostState, invalidateNativeSession, mockConnectionsAvailable, resolveNativeClose } from './services/nativeHost';
const nativeHostState = hostState();
/** Browser demo only; the desktop app never offers Mock mode or sample data. */
const demo = mockConnectionsAvailable();
/** Host, user and database names are typed exactly: no autocapitalize, autocorrect or spellcheck. */
const exactText = { autocapitalize: 'off', autocorrect: 'off', spellcheck: 'false', autocomplete: 'off' } as const;
let liveStatusTimer: ReturnType<typeof setInterval> | undefined;
import { insertRecords, RecordConstraintError } from './services/recordMutations';
import { searchRows, searchOperators as availableSearchOperators, type SearchOperator } from './services/search';
import DatabaseObjects from './components/DatabaseObjects.vue';
import IndexManager from './components/IndexManager.vue';
import { relations, objectSections } from './services/objectCapabilities';
import type { ObjectSection } from './types/objects';
import DatabaseOverview from './components/DatabaseOverview.vue';
import { csvRecords } from './services/csvImport';
import { computed, ref, watch, onMounted, onBeforeUnmount, nextTick } from 'vue';
import {
  Database,
  ChevronDown,
  ArrowUp,
  ArrowDown,
  ChevronRight,
  Plus,
  X,
  Settings,
  Search,
  Table2,
  RefreshCw,
  PanelLeftClose,
  PanelLeftOpen,
  LayoutList,
  Code2,
  ListFilter,
  Download,
  Upload,
  Wrench,
  Pencil,
  Copy,
  Trash2,
  KeyRound,
  ArrowUpDown,
  ChevronLeft,
  ChevronsLeft,
  ChevronsRight,
  Play,
  Clock3,
  Check,
  CircleHelp,
  Server,
  CheckCircle2,
  AlertCircle,
  LoaderCircle,
  Save,
  Terminal,
  FolderOpen,
  ArrowRight,
  SlidersHorizontal,
  Moon,
  Sun,
  Square,
} from 'lucide-vue-next';
import { cancelLiveJob, readWriteRoute, tlsOptional, usesTls, setTls, tlsNote, tlsOffLabel, isLoopbackHost, passwordRsa, setPasswordRsa, normalizePublicKey, writeKeywords, describeJob, describeTransactionEnd, describeUnresolvedOutcome, describeJournalProblem } from './services/liveConnections';
import { useWorkspace } from './stores/workspace';
import { filterRows, selectQuery } from './services/mockTransport';
import SqlEditor from './components/SqlEditor.vue';
import ExportWorkflow from './components/ExportWorkflow.vue';
import ResultNotice from './components/ResultNotice.vue';
import SqlQueryPreview from './components/SqlQueryPreview.vue';
import SqlText from './components/SqlText.vue';
import { buildBrowseQuery } from './services/browseQuery';
import SchemaIcon from './components/SchemaIcon.vue';
import ColumnKeys from './components/ColumnKeys.vue';
import ColumnName from './components/ColumnName.vue';
import { gridTarget } from './services/gridNavigation';
import { invisibleSegments, rawSelectionText } from './services/invisibleValues';
import { hasLineBreak } from './services/lineEndings';
import RawTextarea from './components/RawTextarea.vue';
import { dialog as vDialog } from './directives/dialog';
import EnumEditor from './components/EnumEditor.vue';
import LiveExport from './components/LiveExport.vue';
import LiveDdlDialog from './components/LiveDdlDialog.vue';
import MasterPasswordDialog from './components/MasterPasswordDialog.vue';
import { secretStoreStatus, unlockSecretStore, resetSecretStore, secretStoreNote, savedPlaceholder } from './services/secretStore';
import { onSecretStoreLocked, NativeOperationError } from './services/nativeHost';
import type { SecretStoreKindDto } from '../../../contracts/generated/v1';
import { showCreateTableSql, columnDefinitions, dropColumnSql, dropIndexSql, type PgColumnState } from './services/liveDdl';
import LiveSqlImport from './components/LiveSqlImport.vue';
import { liveCsvImportPlan, MAX_IMPORT_BYTES } from './services/liveImport';
import { gridResize as vGridResize } from './directives/gridResize';
import { queryWidthKey } from './services/gridWidths';
import { liveCatalogSupported, ddlStatements, catalogRows } from './services/liveSchema';
import { liveEditBlocker, liveEditableColumn, liveUpdateSql, liveInsertSql, liveDeleteSql, liveMutationOutcome, type LiveValue } from './services/liveMutations';
import { validateValue, choiceDefinition, hasColumnDefault } from './services/valueValidation';
import { connectionColors, environments, environmentLabel } from './services/connectionPreferences';
import type { Connection, Row, View, Column } from './types';
import {
  columnCapabilities,
  getColumnCapabilities,
  columnType,
  defaultExpressions,
  createColumnDraft,
  validateColumn,
  formatColumnType,
  expressionValue,
} from './services/columnDefinition';
const store = useWorkspace();
const w = computed(() => store.workspace);
const isLive = computed(() => store.connection?.mode==='live');
const liveSession = computed(() => store.liveSessions[store.activeId]);
/** MySQL, MariaDB and PostgreSQL live profiles list tables and browse pages through read-only SELECTs. */
const liveCatalog = computed(() => isLive.value && liveCatalogSupported(store.connection.engine));
/** Read-write is chosen per connect and reported back by the native host. */
const liveReadWrite = computed(() => isLive.value && liveSession.value?.status === 'connected' && liveSession.value.info?.access === 'read_write');
const liveLease = computed(() => liveSession.value?.lease);
/** Mirrors the native host's read-write gate; the renderer can never enable it itself. */
const readWriteAvailable = nativeHostState.kind === 'desktop' && nativeHostState.info.readWriteSql === true;
/** SQLite files are excluded from release builds (A5); mirrors the host's build capability. */
const sqliteAvailable = nativeHostState.kind === 'desktop' && nativeHostState.info.sqlite === true;
const jobSucceeded = computed(() => query.value.job?.outcome === 'succeeded');
const table = computed(() => store.table);
const tableMetadata = computed(() => {
  if (!table.value || isLive.value || (table.value.kind && table.value.kind!=='table')) return [];
  const entries = [{ label: 'Rows', value: table.value.rows.length.toLocaleString() }];
  // These are the known mock storage settings for application tables only.
  if (
    (store.connection.engine === 'MySQL' || store.connection.engine === 'MariaDB') &&
    w.value.database !== 'information_schema'
  ) {
    entries.push(
      { label: 'Engine', value: 'InnoDB' },
      { label: 'Collation', value: 'utf8mb4_unicode_ci' },
    );
  }
  return entries;
});
/** A mock view or materialized view snapshot. */
const isMockView = computed(() => !isLive.value && !!table.value?.kind && table.value.kind !== 'table');
/** No row actions: mock views and every live table (live browse is read-only). */
/** Why the open live table is read-only (null = editable): read-only session, view, no key. */
const liveEditReason = computed(() => {
  if (!isLive.value || !table.value) return null;
  const blocker = liveEditBlocker(store.connection.engine, table.value.columns, table.value.kind);
  if (blocker) return blocker;
  return liveReadWrite.value ? null : 'This live session is read-only. Reconnect with read-write access to edit rows.';
});
/** No row actions: mock views and live tables that cannot be edited (live CRUD needs read-write). */
const isReadOnly = computed(() => isMockView.value || (isLive.value && !!table.value && liveEditReason.value !== null));
/** Runs one generated statement as a read-write job; success only for exactly one row. */
async function runLiveMutation(sql: string, action: 'updated' | 'inserted' | 'deleted', reloadOnConflict = true) {
  const job = await store.runLiveJob(store.activeId, sql);
  const outcome = liveMutationOutcome(job, action);
  // A conflicted cell edit keeps its editor (bound to the loaded row) open with the message.
  if ((outcome.ok && !outcome.inTransaction) || (!outcome.ok && outcome.kind === 'conflict' && reloadOnConflict)) void loadLiveBrowse();
  return outcome;
}
const liveValue = (value: Row[string] | undefined, useDefault?: boolean): LiveValue => (useDefault ? { default: true } : value === null || value === undefined ? null : String(value));
const objectManager = ref<InstanceType<typeof DatabaseObjects>>();
const objectSection = computed(() => w.value.scope === 'database' && objectSections(store.connection.engine).includes(w.value.view as ObjectSection) ? w.value.view as ObjectSection : undefined);
const viewError = ref('');
function requestObjectConfirmation(request:{title:string;message:string;acceptLabel:string;action:()=>void}) { confirmation.value={...request,productionName:store.connection.environment==='Production'?store.connection.name:undefined}; }
function refreshBrowse() {
  viewError.value='';
  if(isMockView.value) { try{store.previewView(w.value.table);}catch(error){viewError.value=(error as Error).message;} }
  if(isLive.value) void loadLiveBrowse();
  browseNoticeVersion.value++;
}
const browseGridOptions = computed(() => ({
  key: `browse:${store.activeId}:${w.value.database}:${w.value.table}`,
  columns: [
    ...(!isReadOnly.value ? [{ name: '_selection', width: 36, resizable: false }, { name: '_actions', width: 104, resizable: false }] : []),
    ...(table.value?.columns ?? []).map((c) => ({ name: c.name, type: c.type })),
  ],
  onError: (message: string) => notify(message, 'error'),
}));
const searchGridOptions = computed(() => ({
  ...browseGridOptions.value,
  columns: [...(table.value?.columns || []).map(c => ({ name: c.name, type: c.type })), ...(!isReadOnly.value ? [{ name: '_actions', width: 104, resizable: false }] : [])],
}));
const resultQuery = computed({
  get: () => query.value.executedSql || '',
  set: (value: string) => {
    query.value.executedSql = value;
  },
});
const resultGridOptions = computed(() => ({
  key: `sql:${store.activeId}:${query.value.resultDatabase || w.value.database}:${queryWidthKey(resultQuery.value, result.value?.columns ?? [])}`,
  columns: (result.value?.columns ?? []).map((name) => ({
    name,
    type: result.value?.columnTypes?.[name],
  })),
  onError: (message: string) => notify(message, 'error'),
}));
const browsePlan = computed(() =>
  buildBrowseQuery({
    engine: store.connection.engine,
    table: w.value.table,
    columns: table.value?.columns ?? [],
    filter: w.value.filter,
    sort: w.value.sort,
    descending: w.value.descending,
    offset: (w.value.page - 1) * store.pageSize,
    limit: store.pageSize,
  }),
);
const tabs = [
  { name: 'Browse', icon: Table2 },
  { name: 'Structure', icon: LayoutList },
  { name: 'SQL', icon: Code2 },
  { name: 'Search', icon: Search },
  { name: 'Insert', icon: Plus },
  { name: 'Export', icon: Download },
  { name: 'Import', icon: Upload },
  { name: 'Operations', icon: Wrench },
  { name: 'Routines', icon: Code2 }, { name: 'Triggers', icon: Wrench }, { name: 'Events', icon: Clock3 },
] as const;
const toast = ref(''),
  toastType = ref('success');
let toastTimer: ReturnType<typeof setTimeout>;
function notify(message: string, type = 'success') {
  toast.value = message;
  toastType.value = type;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = ''), 4500);
}
const media = window.matchMedia('(prefers-color-scheme: dark)');
const systemDark = ref(media.matches);
function onThemeChange(e: MediaQueryListEvent) {
  systemDark.value = e.matches;
}
const dark = computed(
  () => store.theme === 'dark' || (store.theme === 'system' && systemDark.value),
);
watch(dark, (value) => document.documentElement.classList.toggle('dark', value), {
  immediate: true,
});
watch(
  () => store.theme,
  (value) => localStorage.setItem('stoneql-theme', value),
);
const sidebarWidth = ref(250),
  sidebarHidden = ref(false),
  connectionMenu = ref(false);
const treeSearch = computed({
    get: () => w.value.treeFilter,
    set: (value: string) => {
      w.value.treeFilter = value;
    },
  }),
  expanded = computed({
    get: () => w.value.expandedDatabases,
    set: (value: string[]) => {
      w.value.expandedDatabases = value;
    },
  });
const treeDatabases = computed(() =>
  w.value.databases
    .map((d) => ({
      ...d,
      tables: relations(d).filter(
        (t) =>
          !treeSearch.value ||
          `${d.name} ${t.name}`.toLowerCase().includes(treeSearch.value.toLowerCase()),
      ),
    }))
    .filter((d) => isLive.value ? !treeSearch.value || d.name.toLowerCase().includes(treeSearch.value.toLowerCase()) : d.tables.length),
);
function resize(e: MouseEvent) {
  const start = e.clientX,
    width = sidebarWidth.value;
  const move = (event: MouseEvent) =>
    (sidebarWidth.value = Math.max(190, Math.min(420, width + event.clientX - start)));
  const stop = () => {
    window.removeEventListener('mousemove', move);
    window.removeEventListener('mouseup', stop);
  };
  window.addEventListener('mousemove', move);
  window.addEventListener('mouseup', stop);
}
const filtered = computed(() =>
  filterRows(
    table.value?.rows ?? [],
    w.value.filter,
    w.value.sort,
    w.value.descending,
    table.value?.columns.find((c) => c.name === w.value.sort)?.type,
  ),
);
const liveBrowse = ref<{ key: string; sql: string; rows: Row[]; total: number | null; totalError?: string; loading: boolean }>({ key: '', sql: '', rows: [], total: 0, loading: false });
let liveBrowseGeneration = 0, liveFilterTimer: ReturnType<typeof setTimeout> | undefined;
const liveBrowseKey = computed(() => liveCatalog.value && w.value.scope === 'table' && w.value.view === 'Browse' && table.value?.columns.length && liveSession.value?.status === 'connected'
  ? JSON.stringify([store.activeId, w.value.table, w.value.filter, w.value.sort, w.value.descending, w.value.page, store.pageSize]) : '');
/** Fetches the current page; a newer request or a connection change discards older replies. */
async function loadLiveBrowse() {
  const key = liveBrowseKey.value, generation = ++liveBrowseGeneration, profile = store.activeId;
  if (!key) return;
  viewError.value = '';
  const sameTable = !!liveBrowse.value.key && JSON.stringify(JSON.parse(liveBrowse.value.key).slice(0, 2)) === JSON.stringify(JSON.parse(key).slice(0, 2));
  liveBrowse.value = sameTable ? { ...liveBrowse.value, loading: true } : { key, sql: '', rows: [], total: 0, loading: true };
  try {
    const page = await store.browseLive(profile);
    if (generation === liveBrowseGeneration) liveBrowse.value = { key, ...page, loading: false };
  } catch (error) {
    if (generation !== liveBrowseGeneration) return;
    liveBrowse.value = { key, sql: '', rows: [], total: 0, loading: false };
    viewError.value = (error as Error).message;
  }
}
watch(liveBrowseKey, (key, previous) => {
  clearTimeout(liveFilterTimer);
  if (!key) return;
  // Typing in the filter waits for a pause; paging, sorting and table changes load at once.
  const without = (value: string) => { const parts = JSON.parse(value); parts.splice(2, 1); return JSON.stringify(parts); };
  const filterOnly = !!previous && without(previous) === without(key);
  if (filterOnly) liveFilterTimer = setTimeout(loadLiveBrowse, 300);
  else void loadLiveBrowse();
});
const liveCatalogError = ref<Record<string, string>>({});
const liveCatalogHandles = new Map<string, string>();
/** Lists the database's tables; returns to the database scope when the open table disappeared. */
async function loadLiveCatalog(profile = store.activeId) {
  const ws = store.workspaces[profile];
  liveCatalogError.value[profile] = '';
  try {
    // MySQL/MariaDB list every database the account can see; the session's one is loaded.
    await store.loadLiveDatabases(profile);
    await store.loadLiveTables(profile);
    const current = ws?.databases.find((d) => d.name === store.liveDatabase(profile));
    if (ws && ws.scope === 'table' && !current?.tables.some((t) => t.name === ws.table)) Object.assign(ws, { scope: 'database', table: '', view: 'SQL' });
  } catch (error) {
    liveCatalogError.value[profile] = (error as Error).message;
  }
}
watch(() => [store.activeId, liveCatalog.value, liveSession.value?.status, liveSession.value?.info?.handle] as const, ([profile, catalog, status, handle]) => {
  if (!catalog || status !== 'connected' || !handle || liveCatalogHandles.get(profile) === handle) return;
  liveCatalogHandles.set(profile, handle);
  void loadLiveCatalog(profile);
}, { immediate: true });
/** Live pages without a total (COUNT(*) failed) count what is known and allow one more page while pages are full. */
const liveTotalKnown = computed(() => !isLive.value || liveBrowse.value.total !== null);
/** Re-reads the live catalog and, if a table is still open, its columns and keys. */
async function refreshLiveMetadata(profile: string) {
  await loadLiveCatalog(profile);
  const ws = store.workspaces[profile];
  if (ws?.scope === 'table' && ws.table) {
    try { await store.loadLiveColumns(profile, ws.table); } catch (error) { if (store.activeId === profile) notify((error as Error).message, 'error'); }
    if (store.activeId === profile) void loadLiveBrowse();
  }
}
const rowTotal = computed(() => !isLive.value ? filtered.value.length : liveBrowse.value.total ?? (w.value.page - 1) * store.pageSize + liveBrowse.value.rows.length);
const pages = computed(() => liveTotalKnown.value ? Math.max(1, Math.ceil(rowTotal.value / store.pageSize)) : w.value.page + (liveBrowse.value.rows.length === store.pageSize ? 1 : 0));
const browseNoticeVersion = ref(0);
watch([filtered, () => w.value.page, () => store.pageSize], () => {
  browseNoticeVersion.value++;
});
const rows = computed(() =>
  isLive.value ? liveBrowse.value.rows : filtered.value.slice((w.value.page - 1) * store.pageSize, w.value.page * store.pageSize),
);
const selected = ref<Row[]>([]);
watch(
  () => [store.activeId, w.value.database, w.value.table, w.value.filter] as const,
  (next, previous) => {
    selected.value = [];
    gridFocus.value = { row: 0, column: 0 };
    if (
      previous &&
      next[0] === previous[0] &&
      next[1] === previous[1] &&
      next[2] === previous[2] &&
      next[3] !== previous[3]
    )
      w.value.page = 1;
  },
);
watch(pages, (value) => {
  if (w.value.page > value) w.value.page = value;
});
function sort(name: string) {
  if (w.value.sort !== name) {
    w.value.sort = name;
    w.value.descending = false;
  } else if (!w.value.descending) w.value.descending = true;
  else {
    w.value.sort = '';
    w.value.descending = false;
  }
}
function selectAll(event: Event) {
  selected.value = (event.target as HTMLInputElement).checked ? [...rows.value] : [];
}
const gridFocus = ref({ row: 0, column: 0 }),
  browseGrid = ref<HTMLTableElement>();
function focusGridCell(row: number, column: number) {
  gridFocus.value = { row, column };
  nextTick(() =>
    browseGrid.value?.tBodies?.[0]?.rows[row]
      ?.querySelectorAll<HTMLElement>('td[data-grid-column]')
      [column]?.focus(),
  );
}
function gridKeydown(event: KeyboardEvent, row: Row, rowIndex: number, columnIndex: number) {
  if (event.target !== event.currentTarget) return;
  if (event.key === 'Enter' || event.key === 'F2') {
    event.preventDefault();
    editCell(row, table.value!.columns[columnIndex]!.name, event);
    return;
  }
  if (event.key === ' ' && !isReadOnly.value) {
    event.preventDefault();
    selected.value = selected.value.includes(row)
      ? selected.value.filter((r) => r !== row)
      : [...selected.value, row];
    return;
  }
  const target = gridTarget(
    { row: rowIndex, column: columnIndex },
    event.key,
    event.shiftKey,
    rows.value.length,
    table.value?.columns.length || 0,
  );
  if (target) {
    event.preventDefault();
    focusGridCell(target.row, target.column);
  }
}
/**
 * Anchors the floating editor below its cell, or above it (by its bottom edge) when it does not fit
 * below and there is more room above; the chosen side's free space caps its height.
 */
function positionCellEditor(anchor?: HTMLElement, height = 0) {
  if (!anchor?.getBoundingClientRect) return { left: 16, top: 120, bottom: null, maxHeight: null };
  const rect = anchor.getBoundingClientRect(),
    width = Math.min(340, window.innerWidth - 24),
    left = Math.max(12, Math.min(rect.left, window.innerWidth - width - 12)),
    below = window.innerHeight - 8 - rect.bottom,
    above = rect.top - 8;
  return height <= below || below >= above
    ? { left, top: Math.max(8, rect.bottom), bottom: null, maxHeight: Math.max(below, 0) }
    : { left, top: null, bottom: window.innerHeight - rect.top, maxHeight: above };
}
/** Re-anchors the open editor using its rendered (unclamped) height. */
function placeCellEditor() {
  const e = editing.value,
    editor = typeof document.getElementById === 'function' ? document.getElementById('inline-cell-editor') : null;
  if (e?.anchor) Object.assign(e, positionCellEditor(e.anchor, editor ? editor.scrollHeight + 2 : 0));
}
const editing = ref<{
  row: Row;
  column: string;
  value: string;
  isNull: boolean;
  useDefault: boolean;
  multiline: boolean;
  lineIssue: string;
  error: string;
  left: number;
  top: number | null;
  bottom: number | null;
  maxHeight: number | null;
  anchor?: HTMLElement;
} | null>(null);
async function editCell(row: Row, column: string, event?: MouseEvent | KeyboardEvent) {
  if(isReadOnly.value) return;
  const target = isLive.value ? table.value?.columns.find((c) => c.name === column) : undefined;
  if (target && !liveEditableColumn(target)) { notify(`${column} holds binary data and cannot be edited here. Use SQL to change it.`, 'error'); return; }
  if (!(await discard())) return;
  editing.value = {
    row,
    column,
    value: String(row[column] ?? ''),
    isNull: row[column] === null,
    useDefault: false,
    multiline: hasLineBreak(row[column]),
    lineIssue: '',
    error: '',
    anchor: event?.currentTarget as HTMLElement | undefined,
    ...positionCellEditor(event?.currentTarget as HTMLElement | undefined),
  };
  nextTick(() => {
    if (typeof document.getElementById !== 'function') return;
    placeCellEditor();
    document
      .getElementById('inline-cell-editor')
      ?.querySelector<HTMLElement>('.set-choices input:enabled,select:enabled,textarea:enabled,input:not([type=checkbox]):enabled')
      ?.focus();
  });
}
/** Enter saves, except in the multi-line editor where it inserts a newline and Ctrl/Cmd+Enter saves; buttons keep native Enter. */
function cellEditorEnter(event: KeyboardEvent) {
  const target = event.target as HTMLElement;
  if (event.isComposing || target.tagName === 'BUTTON') return;
  if (target.tagName === 'TEXTAREA' && !event.ctrlKey && !event.metaKey) return;
  event.preventDefault();
  saveCell();
}
/** Copies marked grid text as the characters it stands for, never as marker glyphs. */
function copyRawSelection(event: ClipboardEvent) {
  const selection = document.getSelection();
  if (!selection?.rangeCount || selection.isCollapsed || !event.clipboardData) return;
  const text = rawSelectionText(selection.getRangeAt(0).cloneContents());
  if (text === null) return;
  event.clipboardData.setData('text/plain', text);
  event.preventDefault();
}
function saveCell() {
  const e = editing.value,
    c = table.value?.columns.find((c) => c.name === e?.column);
  if (!e || !c) return;
  if (e.lineIssue && !e.isNull && !e.useDefault) {
    e.error = e.lineIssue;
    return;
  }
  const checked = validateValue(c, e.isNull ? null : e.value, store.connection.engine, {
    row: e.row,
    table: table.value,
    tables: store.database?.tables,
    useDefault: e.useDefault,
  });
  if (!checked.valid) {
    e.error = checked.error;
    return;
  }
  if (isLive.value && table.value) {
    const sql = liveUpdateSql(store.connection.engine, table.value.name, table.value.columns, e.row, { [c.name]: liveValue(checked.value, e.useDefault) });
    e.error = '';
    void runLiveMutation(sql, 'updated', false).then((outcome) => {
      if (editing.value !== e) return;
      if (outcome.ok) { editing.value = null; notify(outcome.text); }
      else { e.error = outcome.text; notify(outcome.text, 'error'); }
    }, (error) => { if (editing.value === e) e.error = (error as Error).message; });
    return;
  }
  try {
    store.updateRow(e.row, { [c.name]: e.useDefault ? store.defaultValue(c) : checked.value! });
  } catch (error) {
    e.error = (error as Error).message;
    return;
  }
  editing.value = null;
  notify('Cell updated successfully.');
}
const insertValues = ref<Record<string, string>>({}),
  recordNulls = ref<Record<string, boolean>>({}),
  recordDefaults = ref<Record<string, boolean>>({}),
  recordErrors = ref<Record<string, string>>({}),
  recordLineIssues = ref<Record<string, string>>({}),
  recordMultiline = ref<Record<string, boolean>>({}),
  insertOriginal = ref(''),
  insertMode = ref<Row | null>(null);
function recordSnapshot() {
  return JSON.stringify([insertValues.value, recordNulls.value, recordDefaults.value]);
}
const insertDirty = computed(
  () =>
    w.value.view === 'Insert' &&
    insertOriginal.value !== '' &&
    recordSnapshot() !== insertOriginal.value,
);
async function discard() {
  if (
    (cellIsDirty() || insertDirty.value) &&
    !(await askDiscard('You have unsaved changes. Discard them?'))
  )
    return false;
  editing.value = null;
  insertValues.value = {};
  insertOriginal.value = '';
  recordNulls.value = {};
  recordDefaults.value = {};
  recordErrors.value = {};
  recordLineIssues.value = {};
  return true;
}
function tabKeydown(event: KeyboardEvent) {
  if (!['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
  const current = event.currentTarget as HTMLElement;
  const controls = Array.from(current.closest('[role="tablist"]')?.querySelectorAll<HTMLButtonElement>('[role="tab"]') || []);
  const index = controls.indexOf(current as HTMLButtonElement);
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? controls.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + controls.length) % controls.length;
  event.preventDefault(); controls[next]?.focus();
}
async function navigate(view: View) {
  // Live views are exactly the tabs offered for the current scope and access.
  if(isLive.value && !visibleTabs.value.some((tab)=>tab.name===view)) return;
  if (isReadOnly.value && ['Insert','Operations','Import'].includes(view)) return;
  if (!(await discard())) return;
  w.value.view = view;
  insertMode.value = null;
  if (view === 'Insert') prepareInsert();
}
async function openDatabaseContext(name?: string) {
  if(isLive.value) {
    if (name && !(await useLiveDatabase(name))) return;
    w.value.view='SQL';w.value.scope='database';return;
  }
  if (!(await discard())) return;
  if (name) {
    const database = w.value.databases.find(d=>d.name===name);
    if (!database) return;
    w.value.database = name;
    w.value.table = database.tables[0]?.name || '';
    w.value.scope = 'database';
    if (!expanded.value.includes(name)) expanded.value.push(name);
  } else w.value.scope = 'server';
  w.value.view = 'Structure';
}
/** Explorer row: collapse an open list; otherwise open the database and show its tables. */
async function toggleDatabase(name: string) {
  if (expanded.value.includes(name) && !treeSearch.value) {
    expanded.value = expanded.value.filter((n) => n !== name);
    return;
  }
  await openDatabaseContext(name);
  // Live: a refused switch (for example an open transaction) leaves the list collapsed.
  if (isLive.value && w.value.database !== name) return;
  if (!expanded.value.includes(name)) expanded.value.push(name);
}
async function openOverviewTable(database: string, name: string, view: View) {
  await chooseTable(database,name);
  if (w.value.scope === 'table' && w.value.database === database && w.value.table === name) await navigate(view);
}
const visibleTabs = computed(() => {
  // A live SQL file imports into the database, so read-write MySQL/MariaDB offer Import there too.
  if(isLive.value) return tabs.filter(tab=>tab.name==='SQL' || (w.value.scope==='database' && tab.name==='Export' && liveCatalog.value && liveSession.value?.status==='connected') || (w.value.scope==='database' && tab.name==='Import' && liveReadWrite.value && ['MySQL','MariaDB'].includes(store.connection.engine)) || (table.value && w.value.scope==='table' && (['Browse','Structure','Export'].includes(tab.name) || (['Insert','Import'].includes(tab.name) && !isReadOnly.value))));
  if(w.value.scope==='server') return tabs.filter(t=>t.name==='Structure');
  if(w.value.scope==='database') return tabs.filter(t=>['Structure','SQL','Export','Import',...objectSections(store.connection.engine)].includes(t.name) && (table.value || !['SQL','Import'].includes(t.name)));
  return tabs.filter(t=>!['Routines','Triggers','Events'].includes(t.name)&&(!isReadOnly.value||!['Insert','Import','Operations'].includes(t.name)));
});
let tableChoice = 0;
/** Live: makes `name` the session's database (Browse, SQL, edits, import and export use it). */
async function useLiveDatabase(name: string): Promise<boolean> {
  if (!isLive.value || store.liveDatabase(store.activeId) === name) return true;
  try { await store.switchLiveDatabase(store.activeId, name); return true; }
  catch (error) { notify((error as Error).message, 'error'); return false; }
}
async function chooseTable(db: string, name: string) {
  if (await discard()) {
    if (isLive.value) {
      if (!(await useLiveDatabase(db))) return;
      // Only the latest choice applies; a slower earlier column load cannot open its table.
      const profile = store.activeId, ticket = ++tableChoice;
      try { await store.loadLiveColumns(profile, name); }
      catch (error) { if (ticket === tableChoice) notify((error as Error).message, 'error'); return; }
      if (store.activeId !== profile || ticket !== tableChoice) return;
    }
    store.selectTable(db, name);
    viewError.value='';
    searchResults.value = null;
  }
}
async function switchConnection(id: string) {
  if (await discard()) {
    store.connect(id);
    if(store.connection.mode==='live')void store.refreshLiveStatus(id);
    connectionMenu.value = false;
  }
}
function prepareInsert(row?: Row, copy = false) {
  insertValues.value = {};
  recordNulls.value = {};
  recordDefaults.value = {};
  recordErrors.value = {};
  recordLineIssues.value = {};
  recordMultiline.value = {};
  insertMode.value = row && !copy ? row : null;
  for (const c of table.value?.columns ?? []) {
    const generated = !!(c.autoIncrement);
    recordDefaults.value[c.name] = (copy && generated) || (!row && hasColumnDefault(c));
    recordNulls.value[c.name] =
      !recordDefaults.value[c.name] && (row ? row[c.name] === null : c.nullable);
    insertValues.value[c.name] = row
      ? generated && copy
        ? ''
        : String(row[c.name] ?? '')
      : (c.default ?? '');
    recordMultiline.value[c.name] = hasLineBreak(insertValues.value[c.name]);
  }
  insertOriginal.value = recordSnapshot();
}
async function editRow(row: Row, copy = false) {
  if(isReadOnly.value) return;
  if (!(await discard())) return;
  w.value.view = 'Insert';
  prepareInsert(row, copy);
}
function commitInsert() {
  const t = table.value;
  if (!t) return;
  const values: Row = {},
    errors: Record<string, string> = {};
  for (const c of t.columns) {
    if (recordLineIssues.value[c.name] && !recordNulls.value[c.name] && !recordDefaults.value[c.name]) {
      errors[c.name] = recordLineIssues.value[c.name]!;
      continue;
    }
    const checked = validateValue(
      c,
      recordNulls.value[c.name] ? null : (insertValues.value[c.name] ?? ''),
      store.connection.engine,
      {
        row: insertMode.value ?? undefined,
        table: t,
        tables: store.database?.tables,
        useDefault: recordDefaults.value[c.name],
      },
    );
    if (!checked.valid) errors[c.name] = checked.error;
    else if (checked.value !== undefined) values[c.name] = checked.value;
  }
  recordErrors.value = errors;
  if (Object.keys(errors).length) return;
  if (isLive.value) { void commitLiveRecord(t, values); return; }
  try {
    if (insertMode.value) {
      const patch: Row = {};
      for (const c of t.columns)
        patch[c.name] = recordDefaults.value[c.name] ? store.defaultValue(c) : values[c.name]!;
      store.updateRow(insertMode.value, patch);
    } else store.insert(values);
  } catch (error) {
    notify((error as Error).message, 'error');
    return;
  }
  notify(insertMode.value ? 'Record updated successfully.' : '1 row inserted successfully.');
  insertValues.value = {};
  recordNulls.value = {};
  recordDefaults.value = {};
  recordErrors.value = {};
  recordLineIssues.value = {};
  insertOriginal.value = '';
  insertMode.value = null;
  w.value.view = 'Browse';
}
/** Live Insert/Edit: one INSERT, or one UPDATE of the editable columns that changed. */
async function commitLiveRecord(t: NonNullable<typeof table.value>, values: Row) {
  const engine = store.connection.engine, original = insertMode.value;
  let sql: string;
  try {
    if (original) {
      const changes: Record<string, LiveValue> = {};
      for (const c of t.columns) {
        if (!liveEditableColumn(c)) continue;
        const next = liveValue(values[c.name], recordDefaults.value[c.name]);
        if (typeof next === 'object' && next !== null) changes[c.name] = next;
        else if (next !== (original[c.name] === null || original[c.name] === undefined ? null : String(original[c.name]))) changes[c.name] = next;
      }
      if (!Object.keys(changes).length) { notify('Nothing changed.'); return; }
      sql = liveUpdateSql(engine, t.name, t.columns, original, changes);
    } else {
      const inserted: Record<string, LiveValue> = {};
      for (const c of t.columns) if (liveEditableColumn(c)) inserted[c.name] = liveValue(values[c.name], recordDefaults.value[c.name]);
      sql = liveInsertSql(engine, t.name, t.columns, inserted);
    }
  } catch (error) { notify((error as Error).message, 'error'); return; }
  try {
    const outcome = await runLiveMutation(sql, original ? 'updated' : 'inserted');
    if (!outcome.ok) { notify(outcome.text, 'error'); return; }
    notify(outcome.text);
    insertValues.value = {}; recordNulls.value = {}; recordDefaults.value = {}; recordErrors.value = {}; recordLineIssues.value = {};
    insertOriginal.value = ''; insertMode.value = null; w.value.view = 'Browse';
  } catch (error) { notify((error as Error).message, 'error'); }
}
const confirmation = ref<{
  title: string;
  message: string;
  action: () => void;
  productionName?: string;
  kind?: 'discard' | 'destructive' | 'notice';
  acceptLabel?: string;
  cancel?: () => void;
} | null>(null);
const productionAcknowledged = ref(false),
  confirmationError = ref('');
let pendingDiscard: Promise<boolean> | undefined;
function askDiscard(message: string, acceptLabel = 'Discard changes'): Promise<boolean> {
  if (pendingDiscard) return pendingDiscard;
  pendingDiscard = new Promise((resolve) => {
    const finish = (accepted: boolean) => {
      pendingDiscard = undefined;
      resolve(accepted);
    };
    confirmation.value = {
      title: 'Unsaved changes',
      message,
      kind: 'discard',
      acceptLabel,
      action: () => finish(true),
      cancel: () => finish(false),
    };
  });
  return pendingDiscard;
}
/** Confirmation dialog as a promise; a PROD profile also needs the acknowledgement checkbox. */
function askConfirm(request: { title: string; message: string; acceptLabel: string }): Promise<boolean> {
  return new Promise((resolve) => {
    confirmation.value = {
      ...request,
      kind: 'notice',
      productionName: store.connection.environment === 'Production' ? store.connection.name : undefined,
      action: () => resolve(true),
      cancel: () => resolve(false),
    };
  });
}
/** Live MySQL/MariaDB schema changes need a read-write session; the server enforces privileges. */
const liveDdlAvailable = computed(() => isLive.value && liveReadWrite.value && (store.connection.engine === 'MySQL' || store.connection.engine === 'MariaDB'));
/** Column and index changes apply to base tables (with or without a primary key); PostgreSQL too. */
const liveStructureEditable = computed(() => isLive.value && liveReadWrite.value && ['MySQL', 'MariaDB', 'PostgreSQL'].includes(store.connection.engine) && !!table.value && (table.value.kind ?? 'table') === 'table');
const liveDdl = ref<null | { mode: 'add-column' | 'edit-column' | 'create-database' | 'add-index'; column?: string; definition?: string; pg?: PgColumnState; error?: string }>(null);
const liveDdlBusy = ref(false);
const LIVE_DDL_IN_TRANSACTION = 'Commit or roll back the open transaction first: a schema change ends it.';
/** The server's exact column definitions, from SHOW CREATE TABLE (run as a job). */
async function liveColumnDefinitions(name: string) {
  const job = await store.runLiveJob(store.activeId, showCreateTableSql(store.connection.engine, name));
  if (job.outcome !== 'succeeded' || !job.resultSets[0]) throw new Error(describeJob(job, false).text);
  const row = catalogRows(job.resultSets[0])[0] ?? {};
  return columnDefinitions(String(row['Create Table'] ?? ''));
}
async function openLiveColumn(column?: Column) {
  if (!table.value) return;
  if (liveLease.value) return notify(LIVE_DDL_IN_TRANSACTION, 'error');
  if (!column) { liveDdl.value = { mode: 'add-column' }; return; }
  // PostgreSQL: the catalog state; only changed parts are altered, nothing is restated.
  if (store.connection.engine === 'PostgreSQL') {
    liveDdl.value = { mode: 'edit-column', column: column.name, pg: { type: column.type, nullable: !!column.nullable, default: column.defaultKind === 'custom' ? column.default ?? '' : '' } };
    return;
  }
  try {
    const definition = (await liveColumnDefinitions(table.value.name)).get(column.name);
    if (definition === undefined) throw new Error(`Column ${column.name} was not found in the table definition. Refresh and try again.`);
    liveDdl.value = { mode: 'edit-column', column: column.name, definition };
  } catch (error) { notify((error as Error).message, 'error'); }
}
function openCreateDatabase() {
  if (liveLease.value) return notify(LIVE_DDL_IN_TRANSACTION, 'error');
  liveDdl.value = { mode: 'create-database' };
}
function openLiveIndex() {
  if (liveLease.value) return notify(LIVE_DDL_IN_TRANSACTION, 'error');
  liveDdl.value = { mode: 'add-index' };
}
async function dropLiveIndex(name: string) {
  if (!table.value) return;
  const sql = dropIndexSql(store.connection.engine, table.value.name, name);
  await applyLiveDdl(sql, true, `Drop index ${name} on ${table.value.name}? Queries that use it may become slower; a unique index stops enforcing uniqueness.`);
}
async function dropLiveColumn(column: Column) {
  if (!table.value) return;
  const sql = dropColumnSql(store.connection.engine, table.value.name, column.name);
  await applyLiveDdl(sql, true, `Drop column ${column.name} from ${table.value.name}? Its values are deleted on the server and cannot be restored from StoneQL.`);
}
/** Runs exactly the previewed statement; destructive or PROD changes are confirmed first. */
async function applyLiveDdl(sql: string, destructive: boolean, warning?: string): Promise<boolean> {
  const profile = store.activeId, creating = liveDdl.value?.mode === 'create-database';
  const fail = (text: string) => { if (liveDdl.value) liveDdl.value.error = text; else notify(text, 'error'); return false; };
  if (liveLease.value) return fail(LIVE_DDL_IN_TRANSACTION);
  if ((destructive || store.connection.environment === 'Production') && !(await askConfirm({
    title: destructive ? 'Apply a destructive schema change?' : 'Apply schema change?',
    acceptLabel: 'Apply',
    message: `${warning ?? (destructive ? 'Changing the column definition can convert or truncate stored values.' : 'This changes the schema on the server.')} SQL: ${sql}`,
  }))) return false;
  liveDdlBusy.value = true;
  try {
    const job = await store.runLiveJob(profile, sql);
    if (job.outcome !== 'succeeded') {
      const failed = job.statements.find((s) => s.failure);
      return fail(failed?.failure?.denied
        ? `The server denied this account the change (${creating ? 'it needs the CREATE privilege' : store.connection.engine === 'PostgreSQL' ? 'it must own the table' : 'it needs the ALTER or INDEX privilege'}). Nothing was changed.`
        : describeJob(job, false).text);
    }
    liveDdl.value = null;
    notify(creating ? 'Database created.' : 'Schema change applied.');
    if (creating) await store.loadLiveDatabases(profile).catch(() => undefined);
    else await refreshLiveMetadata(profile);
    return true;
  } catch (error) { return fail((error as Error).message); }
  finally { liveDdlBusy.value = false; }
}
function cancelConfirmation() {
  confirmation.value?.cancel?.();
  confirmation.value = null;
}
function cellIsDirty() {
  const e = editing.value;
  return (
    !!e &&
    (e.useDefault ||
      e.value !== String(e.row[e.column] ?? '') ||
      e.isNull !== (e.row[e.column] === null))
  );
}
async function cancelCellEdit() {
  if (cellIsDirty() && !(await askDiscard('Discard this cell edit?'))) return;
  editing.value = null;
  focusGridCell(gridFocus.value.row, gridFocus.value.column);
}
function runConfirmation() {
  if (!confirmation.value) return;
  try {
    confirmation.value.action();
    confirmation.value = null;
  } catch (error) {
    confirmationError.value = (error as Error).message;
  }
}
function copySelected() {
  // Mock-only (local rows); a live table must never report a copy that did not happen.
  if (isLive.value) return notify('Not available for live tables yet; nothing was changed on the server.', 'error');
  try {
    const count = store.copyRows(selected.value).length;
    selected.value = [];
    notify(`${count} rows copied.`);
  } catch (error) {
    notify(`Copy failed: ${(error as Error).message}`, 'error');
  }
}
watch(confirmation, () => {
  productionAcknowledged.value = false;
  confirmationError.value = '';
});
function removeRows(items: Row[]) {
  if (!items.length) return;
  confirmation.value = {
    title: `Delete ${items.length === 1 ? 'record' : `${items.length} records`}?`,
    acceptLabel: `Delete ${items.length} record${items.length === 1 ? '' : 's'}`,
    message: `Remove ${items.length === 1 ? table.value?.columns.filter(c => c.primary).map(c => `${c.name} = ${items[0]![c.name]}`).join(', ') || 'the selected record' : `${items.length} selected records`} from ${isLive.value ? 'live' : 'mock'} table ${w.value.table}?${isLive.value ? ' Each row is deleted by its primary key in its own statement; this cannot be undone from StoneQL.' : ''}`,
    productionName:
      store.connection.environment === 'Production' ? store.connection.name : undefined,
    action: () => {
      if (isLive.value && table.value) { void deleteLiveRows(table.value, items); return; }
      store.deleteRows(items);
      if (searchResults.value !== null) runSearch();
      selected.value = [];
      notify(`${items.length} record(s) deleted.`);
    },
  };
}
/** One DELETE per row, stopping at the first non-success so nothing is reported falsely. */
async function deleteLiveRows(t: NonNullable<typeof table.value>, items: Row[]) {
  let deleted = 0;
  for (const row of items) {
    try {
      const outcome = await runLiveMutation(liveDeleteSql(store.connection.engine, t.name, t.columns, row), 'deleted');
      if (!outcome.ok) { notify(`${deleted} of ${items.length} deleted. ${outcome.text}`, 'error'); break; }
      deleted++;
    } catch (error) { notify(`${deleted} of ${items.length} deleted. ${(error as Error).message}`, 'error'); break; }
  }
  selected.value = [];
  if (deleted === items.length) notify(`${deleted} record(s) deleted.`);
}
const connectionSearch = ref(''),
  groupName = ref(''),
  groupEditing = ref<string | null>(null),
  groupError = ref('');
const ungroupedCollapsed = ref(false);
const managerGroups = computed(() => [
  ...store.groups,
  { id: '', name: 'Ungrouped', collapsed: ungroupedCollapsed.value },
]);
function groupedConnections(id: string) {
  return store.connections.filter(
    (c) =>
      (c.groupId || '') === id &&
      `${c.name} ${c.host} ${environmentLabel(c)}`
        .toLowerCase()
        .includes(connectionSearch.value.toLowerCase()),
  );
}
function beginGroup(id = 'new') {
  groupEditing.value = id;
  groupName.value = store.groups.find((g) => g.id === id)?.name || '';
  groupError.value = '';
}
function saveGroup() {
  try {
    if (groupEditing.value === 'new') store.createGroup(groupName.value);
    else store.renameGroup(groupEditing.value!, groupName.value);
    groupEditing.value = null;
    groupError.value = '';
  } catch (error) {
    groupError.value = (error as Error).message;
  }
}
function deleteGroup(id: string) {
  const group = store.groups.find((g) => g.id === id);
  confirmation.value = {
    title: `Delete group “${group?.name}”?`,
    acceptLabel: 'Delete group',
    message: 'Connections will be preserved and moved to Ungrouped.',
    action: () => {
      store.deleteGroup(id);
      notify('Group deleted. Connections moved to Ungrouped.');
    },
  };
}
const columnDraftInitial = ref('');
const manager = ref(false);
const {
  connectionDialog, connectionDraft, connectionDraftInitial, connectionError, connectionEditing,
  testing, picking, testMessage, testOk, databaseOptions, pendingHostKey, hostKeyTrust, attemptId,
  newConnection, closeConnectionDialog, changeTransport, readCertificate, chooseSqliteFile,
  fetchHostKey, trustHostKey, rejectHostKey, forgetHostKey, chooseSshKeyFile, cancelAttempt,
  testConnection, saveConnection, removeConnection, closeConnection,
} = useConnectionWorkflow({
  manager, notify, discard, askDiscard, confirm: (request) => { confirmation.value = request; },
  confirmReadWrite: (name) => new Promise((resolve) => {
    confirmation.value = {
      title: 'Open read-write?', kind: 'notice', acceptLabel: 'Open read-write', productionName: name,
      message: `Statements run in this session can change or remove data on “${name}”. This confirmation is a safeguard, not authorization: database privileges decide what is allowed.`,
      action: () => resolve(true), cancel: () => resolve(false),
    };
  }),
});
/** Where saved passwords go (shown in the connection dialog), and the master-password prompt. */
const secretStoreKind = ref<SecretStoreKindDto>();
async function refreshSecretStore() {
  if (nativeHostState.kind !== 'desktop') return;
  try { secretStoreKind.value = (await secretStoreStatus()).kind; } catch { secretStoreKind.value = undefined; }
}
watch(connectionDraft, (draft, previous) => { if (draft && !previous) void refreshSecretStore(); });
const masterPassword = ref<{ mode: 'create' | 'unlock'; busy: boolean; error: string; done: (unlocked: boolean) => void } | null>(null);
let pendingUnlock: Promise<boolean> | undefined;
/** Asked by the native bridge when the encrypted file is locked; resolves true once unlocked. */
function askMasterPassword(): Promise<boolean> {
  if (pendingUnlock) return pendingUnlock;
  pendingUnlock = secretStoreStatus().then((status) => new Promise<boolean>((resolve) => {
    if (status.unlocked) return resolve(true);
    masterPassword.value = { mode: status.vaultExists ? 'unlock' : 'create', busy: false, error: '', done: (unlocked) => { masterPassword.value = null; resolve(unlocked); } };
  }), () => false).finally(() => { pendingUnlock = undefined; });
  return pendingUnlock;
}
if (nativeHostState.kind === 'desktop') onSecretStoreLocked(askMasterPassword);
async function submitMasterPassword(password: string) {
  const prompt = masterPassword.value;
  if (!prompt) return;
  prompt.busy = true; prompt.error = '';
  try {
    await unlockSecretStore(password, prompt.mode === 'create');
    prompt.done(true);
  } catch (error) {
    prompt.busy = false;
    prompt.error = error instanceof NativeOperationError && error.code === 'MASTER_PASSWORD_REJECTED' ? 'Incorrect master password. Try again.' : (error as Error).message;
  }
}
function resetMasterPassword() {
  confirmation.value = {
    title: 'Reset saved passwords?',
    kind: 'destructive',
    acceptLabel: 'Delete saved passwords',
    message: 'This deletes StoneQL\u2019s encrypted file. Every password saved in it is lost and cannot be recovered; connections keep their other settings and ask for the password again. You then set a new master password.',
    action: () => {
      void resetSecretStore().then(() => { if (masterPassword.value) Object.assign(masterPassword.value, { mode: 'create', error: '' }); },
        (error) => { if (masterPassword.value) masterPassword.value.error = (error as Error).message; });
    },
  };
}
async function closeColumnDialog() {
  if (
    JSON.stringify(columnDraft.value) !== columnDraftInitial.value &&
    !(await askDiscard('Discard column definition changes?'))
  )
    return;
  columnDialog.value = false;
}
async function closeManager() {
  if (
    groupEditing.value !== null &&
    groupName.value.trim() &&
    !(await askDiscard('Discard the unsaved group name?'))
  )
    return;
  groupEditing.value = null;
  manager.value = false;
}
const queryPlans = new WeakMap<object, ReturnType<typeof buildBrowseQuery>>();
async function openSqlText(text: string, newTab = false) {
  if (!(await discard())) return;
  if (!newTab && query.value.sql !== query.value.saved && query.value.sql !== text && !(await askDiscard('Replace unsaved query text?'))) return;
  w.value.view = 'SQL';
  if (newTab) newQuery();
  query.value.sql = text;
  if (text === browsePlan.value.sql) queryPlans.set(query.value, browsePlan.value);
}
async function restoreHistory(text: string) {
  if (
    query.value.sql !== query.value.saved &&
    !(await askDiscard('Replace unsaved query text with this history entry?'))
  )
    return;
  query.value.sql = text;
  showHistory.value = false;
}
const settings = ref(false),
  help = ref(false);
const query = computed(() => w.value.queries.find((q) => q.id === w.value.queryId)!);
const schema = computed(() =>
  Object.fromEntries(
    relations(store.database).map((t) => [t.name, t.columns.map((c) => c.name)]),
  ),
);
const running = computed({
    get: () => query.value.running || false,
    set: (value: boolean) => {
      query.value.running = value;
    },
  }),
  result = computed({
    get: () => query.value.result ?? null,
    set: (value) => {
      query.value.result = value;
    },
  }),
  sqlError = computed({
    get: () => query.value.error || '',
    set: (value: string) => {
      query.value.error = value;
    },
  }),
  showHistory = computed({
    get: () => w.value.showHistory,
    set: (value: boolean) => {
      w.value.showHistory = value;
    },
  }),
  resultPage = computed({
    get: () => query.value.resultPage || 1,
    set: (value: number) => {
      query.value.resultPage = value;
    },
  });
const resultRows = computed(() =>
  result.value
    ? filterRows(
        result.value.rows,
        '',
        query.value.resultSort || '',
        query.value.resultDescending || false,
        result.value.columnTypes?.[query.value.resultSort || ''],
      )
    : [],
);
function sortResults(column: string) {
  const q = query.value;
  if (q.resultSort !== column) {
    q.resultSort = column;
    q.resultDescending = false;
  } else if (!q.resultDescending) q.resultDescending = true;
  else {
    q.resultSort = '';
    q.resultDescending = false;
  }
  q.resultPage = 1;
}
function newQuery() {
  const id = Date.now();
  w.value.queries.push({ id, title: `Query ${w.value.queries.length + 1}`, sql: '', saved: '' });
  w.value.queryId = id;
}
async function closeQuery(id: number) {
  const q = w.value.queries.find((q) => q.id === id);
  if (q && q.sql !== q.saved && !(await askDiscard('Discard unsaved query changes?'))) return;
  if (w.value.queries.length === 1) return notify('Keep at least one query tab open.', 'error');
  w.value.queries = w.value.queries.filter((q) => q.id !== id);
  if (w.value.queryId === id) w.value.queryId = w.value.queries[0]!.id;
}
async function execute() {
  if (liveReadWrite.value) return executeJob();
  const profileId=store.activeId, live=isLive.value;
  const ws = w.value,
    q = query.value,
    database = ws.database,
    text = q.sql,
    previewPlan = live ? undefined : queryPlans.get(q) || browsePlan.value;
  if (q.running) return;
  q.running = true;
  q.result = null;
  q.error = '';
  q.executedSql = text;
  q.resultDatabase = database;
  if(!live)await new Promise((resolve) => setTimeout(resolve, 400));
  try {
    q.result = live
      ? await store.executeLive(profileId,text)
      : selectQuery(
      text,
      relations(ws.databases.find((d) => d.name === database)),
      previewPlan,
    );
    q.resultPage = 1;
    q.resultSort = '';
    q.resultDescending = false;
    ws.history.unshift({ sql: text, time: new Date().toLocaleTimeString(), status: 'Success' });
  } catch (error) {
    q.error = (error as Error).message;
    ws.history.unshift({ sql: text, time: new Date().toLocaleTimeString(), status: 'Error' });
  } finally {
    q.running = false;
  }
}
/** Read-write live SQL: a native job with per-statement outcomes and every result set. */
async function executeJob() {
  const profileId = store.activeId, ws = w.value, q = query.value, text = q.sql;
  if (q.running) return;
  const writes = writeKeywords(text);
  if (writes.length && !(await askConfirm({
    title: 'Run statements that change data?',
    acceptLabel: 'Run statements',
    message: `This script contains ${writes.join(', ')} and runs ${liveLease.value ? 'inside the open transaction' : 'with autocommit: each successful statement is committed immediately'} on “${store.connection.name}”. Database privileges remain the authority.`,
  }))) return;
  q.running = true; q.result = null; q.resultSets = []; q.job = undefined; q.error = ''; q.executedSql = text; q.resultDatabase = ws.database;
  try {
    const job = await store.runLiveJob(profileId, text, (jobId) => { q.jobId = jobId; });
    q.job = job; q.resultSets = job.resultSets; q.resultIndex = 0; q.result = job.resultSets[0] ?? null;
    q.resultPage = 1; q.resultSort = ''; q.resultDescending = false;
    const notice = describeJob(job, job.statements.some((s) => s.inTransaction));
    q.error = notice.kind === 'success' ? '' : notice.text;
    ws.history.unshift({ sql: text, time: new Date().toLocaleTimeString(), status: job.outcome === 'succeeded' ? 'Success' : 'Error' });
    // Metadata invalidation: schema statements that ran (even if a later one failed) make the
    // loaded catalog stale; re-read it and the open table's columns.
    if (ddlStatements(text).length && job.statements.some((s) => s.status !== 'not_run')) void refreshLiveMetadata(profileId);
  } catch (error) {
    q.error = (error as Error).message;
    ws.history.unshift({ sql: text, time: new Date().toLocaleTimeString(), status: 'Error' });
  } finally { q.running = false; q.jobId = undefined; }
}
async function cancelExecution() {
  const jobId = query.value.jobId;
  if (!jobId) return;
  try { await cancelLiveJob(jobId); notify('Cancellation requested. The result shows whether it was confirmed.'); }
  catch (error) { notify((error as Error).message, 'error'); }
}
function selectResultSet(index: number) {
  const q = query.value, set = q.resultSets?.[index];
  if (!set) return;
  q.resultIndex = index; q.result = set; q.resultPage = 1; q.resultSort = ''; q.resultDescending = false;
}
async function transaction(action: 'begin' | 'commit' | 'rollback') {
  const profileId = store.activeId;
  try {
    if (action === 'begin') { await store.beginLiveTransaction(profileId); notify('Transaction started. Statements run in it until you commit or roll back.'); return; }
    if (action === 'commit' && !(await askConfirm({ title: 'Commit transaction?', acceptLabel: 'Commit', message: `Make this transaction's changes on “${store.connection.name}” permanent.` }))) return;
    const notice = describeTransactionEnd(await store.endLiveTransaction(profileId, action), action);
    if (store.connection.mode === 'live') void refreshLiveMetadata(profileId);
    notify(notice.text, notice.kind === 'success' ? 'success' : 'error');
  } catch (error) { notify((error as Error).message, 'error'); }
}
/** Explicit acknowledgement of one unknown outcome: hides it; the recorded outcome never changes. */
async function acknowledgeOutcome(outcomeId: string) {
  try { await store.acknowledgeUnresolvedOutcome(outcomeId); }
  catch (error) { notify((error as Error).message, 'error'); }
}
/** Where an outcome comes from: this session's profile, an earlier session (reload) or an earlier run (restart). */
function outcomeProfile(outcome: { outcomeId: string; restored: boolean; journalDamage: unknown }) {
  if (outcome.journalDamage) return 'Unknown-outcome journal';
  if (outcome.restored) return 'Before a restart';
  const id = store.outcomeProfiles[outcome.outcomeId];
  return id ? store.connections.find((c) => c.id === id)?.name || 'Removed profile' : 'Before a reload';
}
function outcomeAge(seconds: number) {
  return seconds < 60 ? 'less than a minute ago' : seconds < 3600 ? `${Math.floor(seconds / 60)} min ago` : `${Math.floor(seconds / 3600)} h ago`;
}
/** Disconnecting rolls back an open transaction: confirm first, then show its outcome. */
async function disconnectLive() {
  const profileId = store.activeId;
  if (liveLease.value && !(await askDiscard(LEASE_DISCONNECT_WARNING, 'Disconnect and roll back'))) return;
  try { reportLeaseEnd(await store.disconnectLiveProfile(profileId), notify); }
  catch (error) { notify((error as Error).message, 'error'); }
}
const searchFilters = ref<Record<string, string>>({}),
  searchOperators = ref<Record<string, string>>({}),
  searchResults = ref<Row[] | null>(null);
const searchError = ref('');
function runSearch() {
  searchError.value = '';
  if (!table.value) return;
  try {
    searchResults.value = searchRows(table.value, table.value.columns.map(c => ({ column: c.name, operator: (searchOperators.value[c.name] || 'contains') as SearchOperator, value: searchFilters.value[c.name] || '' })));
  } catch (error) { searchError.value = (error as Error).message; searchResults.value = null; }
}
function resetSearch() { searchFilters.value = {}; searchOperators.value = Object.fromEntries((table.value?.columns || []).map(c => [c.name, 'contains'])); searchResults.value = null; searchError.value = ''; }
watch(() => [store.activeId, w.value.database, w.value.table], resetSearch, { immediate: true });
const importFormat = ref('CSV'),
  importFile = ref<File | null>(null),
  transferBusy = ref(false),
  transferProgress = ref(0);
function download(text: string, name: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
async function progress() {
  transferBusy.value = true;
  transferProgress.value = 0;
  for (let i = 0; i < 5; i++) {
    await new Promise((r) => setTimeout(r, 100));
    transferProgress.value += 20;
  }
  transferBusy.value = false;
}
const importNotice = ref(''), importError = ref('');
/** Live CSV import: all batches in one transaction; any failure rolls the whole import back. */
async function importLiveCsv(file: File, target: NonNullable<typeof table.value>) {
  if (file.size > MAX_IMPORT_BYTES) { importError.value = `Choose a file up to ${MAX_IMPORT_BYTES / 1048576} MiB.`; return; }
  const profile = store.activeId, engine = store.connection.engine;
  if (store.liveSessions[profile]?.lease) { importError.value = 'Commit or roll back the open transaction first; an import runs in its own transaction.'; return; }
  let plan;
  try { plan = liveCsvImportPlan(engine, target.name, target.columns, await file.text()); }
  catch (error) { importError.value = (error as Error).message; return; }
  if (!plan.records) { importNotice.value = 'The file has no records; nothing was imported.'; return; }
  try { await store.beginLiveTransaction(profile); }
  catch (error) { importError.value = (error as Error).message; return; }
  let failure = '';
  for (const batch of plan.batches) {
    try {
      const job = await store.runLiveJob(profile, batch.sql);
      if (job.outcome !== 'succeeded') { failure = `Records ${batch.first}–${batch.last}: ${describeJob(job, true).text}`; break; }
    } catch (error) { failure = `Records ${batch.first}–${batch.last}: ${(error as Error).message}`; break; }
  }
  try {
    if (failure) {
      const end = await store.endLiveTransaction(profile, 'rollback');
      importError.value = `${failure} Import rolled back: ${describeTransactionEnd(end, 'rollback').text}`;
      return;
    }
    const notice = describeTransactionEnd(await store.endLiveTransaction(profile, 'commit'), 'commit');
    if (notice.kind === 'success') importNotice.value = `${plan.records} records imported into ${target.name} in one transaction.`;
    else importError.value = notice.text;
  } catch (error) {
    importError.value = `${failure ? `${failure} ` : ''}${(error as Error).message}`;
  } finally { void loadLiveBrowse(); }
}
async function importData() {
  importNotice.value = ''; importError.value = '';
  const file = importFile.value, target = table.value, database = store.database, engine = store.connection.engine;
  if (!file || !target || !database) { importError.value = 'Choose a file and target table first.'; return; }
  if (isLive.value) {
    if (importFormat.value === 'SQL') return; // Live SQL import has its own panel (LiveSqlImport).
    await importLiveCsv(file, target);
    return;
  }
  if (file.size > 2_000_000) { importError.value = 'Choose a file smaller than 2 MB.'; return; }
  const format = importFormat.value;
  await progress();
  if (format === 'SQL') {
    importNotice.value = 'Simulation only: SQL was not executed and no records were changed.';
    return;
  }
  try {
    const incoming = csvRecords(await file.text(), target);
    insertRecords(database, target, incoming, engine);
    importNotice.value = `${incoming.length} records added to ${target.name} in mock memory only.`;
  } catch (error) {
    const detail = error as RecordConstraintError;
    importError.value = `${detail.rowIndex ? `CSV record ${detail.rowIndex + 1}${detail.column ? ` (${detail.column})` : ''}: ` : ''}${detail.message}`;
  }
}
const operationName = ref(''),
  copyName = ref(''),
  copyData = ref(true);
function renameTable() {
  const name = operationName.value.trim();
  if (!/^[a-zA-Z_]\w*$/.test(name)) return notify('Use a valid table name.', 'error');
  if (store.database?.tables.some((t) => t.name === name))
    return notify('A table with this name already exists.', 'error');
  try {
    store.renameTable(name);
  } catch (error) {
    notify((error as Error).message, 'error');
    return;
  }
  operationName.value = '';
  notify('Table renamed.');
}
function copyTable() {
  try {
    store.copyTable(copyName.value.trim(), copyData.value);
    copyName.value = '';
    notify('Table copied in mock memory.');
  } catch (error) {
    notify((error as Error).message, 'error');
  }
}
function destructiveOperation(drop: boolean) {
  confirmation.value = {
    acceptLabel: drop ? 'Drop table' : 'Truncate table',
    title: drop ? `Drop table ${w.value.table}?` : `Truncate table ${w.value.table}?`,
    productionName:
      store.connection.environment === 'Production' ? store.connection.name : undefined,
    message: drop
      ? 'The table structure and every record will be removed from this workspace.'
      : 'Every record in this table will be permanently removed.',
    action: () => {
      store.removeTable(drop);
      notify(drop ? 'Table dropped.' : 'Table truncated.');
    },
  };
}
const columnDialog = ref(false),
  columnDraft = ref(createColumnDraft(store.connection?.engine ?? 'MySQL')),
  columnOriginal = ref(''),
  columnError = ref('');
const columnCap = computed(() => getColumnCapabilities(store.connection.engine, table.value));
const columnSpec = computed(() => columnType(columnDraft.value, store.connection.engine));
const columnExpressions = computed(() =>
  defaultExpressions(columnDraft.value, store.connection.engine),
);
const columnPreview = computed(() => {
  try {
    return formatColumnType(columnDraft.value, store.connection.engine);
  } catch {
    return columnDraft.value.dataType.toLowerCase();
  }
});
function editColumn(column?: Column) {
  columnOriginal.value = column?.name ?? '';
  columnDraft.value = createColumnDraft(store.connection.engine, column);
  columnError.value = '';
  columnDraftInitial.value = JSON.stringify(columnDraft.value);
  columnDialog.value = true;
}
function changeColumnType() {
  const d = columnDraft.value;
  const nativeColumn = table.value?.columns.find(
    (c) => c.enumValues && (c.enumName || c.type) === d.dataType,
  );
  if (nativeColumn) {
    d.enumName = nativeColumn.enumName || nativeColumn.type;
    d.enumValues = [...nativeColumn.enumValues!];
  }
  if (!columnSpec.value?.numeric) d.unsigned = false;
  if (!columnSpec.value?.integer) d.autoIncrement = false;
  if (!columnSpec.value?.text) d.collation = '';
  if (columnSpec.value?.parameters === 'decimal' && d.precision > columnCap.value.maxPrecision)
    d.precision = 10;
  if (!columnExpressions.value.includes(d.expression))
    d.expression = columnExpressions.value[0] ?? '';
  if (!columnExpressions.value.length && d.defaultKind === 'expression') d.defaultKind = 'none';
}
function saveColumn() {
  if (!table.value) return;
  if (isLive.value) return notify('Not available for live tables yet; nothing was changed on the server.', 'error');
  const errors = validateColumn(
    columnDraft.value,
    store.connection.engine,
    table.value,
    columnOriginal.value,
  );
  if (errors.length) {
    columnError.value = errors.join(' ');
    return;
  }
  try {
    store.updateColumn(columnDraft.value, columnOriginal.value);
    columnDialog.value = false;
    columnError.value = '';
    notify('Structure updated.');
  } catch (error) {
    columnError.value = (error as Error).message;
  }
}
function dropColumn(column: Column) {
  if (isLive.value) return notify('Not available for live tables yet; nothing was changed on the server.', 'error');
  if (column.primary)
    return notify('Primary key columns cannot be removed in this prototype.', 'error');
  confirmation.value = {
    title: `Drop column ${column.name}?`,
    acceptLabel: 'Drop column',
    message: 'This column and its values will be removed from every record.',
    productionName:
      store.connection.environment === 'Production' ? store.connection.name : undefined,
    action: () => {
      if (!table.value) return;
      store.removeColumn(column);
      notify('Column dropped.');
    },
  };
}
async function keyboard(event: KeyboardEvent) {
  if (event.key === 'Escape' && !event.defaultPrevented) {
    if (confirmation.value) cancelConfirmation();
    else if (columnDialog.value) await closeColumnDialog();
    else if (connectionDialog.value) await closeConnectionDialog();
    else if (settings.value) settings.value = false;
    else if (help.value) help.value = false;
    else if (manager.value) await closeManager();
    else if (editing.value) await cancelCellEdit();
    return;
  }
  if (!store.shortcuts) return;
  if ((event.metaKey || event.ctrlKey) && event.key === ',') {
    event.preventDefault();
    settings.value = true;
  }
}
function hasUnsavedChanges() {
  return cellIsDirty() || insertDirty.value ||
    (connectionDialog.value && JSON.stringify(connectionDraft.value) !== connectionDraftInitial.value) ||
    (columnDialog.value && JSON.stringify(columnDraft.value) !== columnDraftInitial.value) ||
    (manager.value && groupEditing.value !== null && !!groupName.value.trim()) ||
    Object.values(store.workspaces).some((ws) => ws.queries.some((q) => q.sql !== q.saved));
}
/** An open transaction lease or a running live job ends with the window; its outcome cannot be shown afterwards. */
function hasOpenLiveWork() {
  return Object.values(store.liveSessions).some((session) => session.lease) ||
    store.connections.some((c) => c.mode === 'live' && store.workspaces[c.id]?.queries.some((q) => q.running));
}
const LIVE_WORK_CLOSE_WARNING = 'A live transaction is open or a live statement is running. Closing StoneQL ends it: an open transaction is rolled back or ends with an unknown outcome, and a running statement may continue on the server. An outcome the server did not confirm is listed the next time StoneQL starts, until you acknowledge it.';
let reloadConfirmed = false;
function beforeUnload(event: BeforeUnloadEvent) {
  if (!reloadConfirmed && (hasUnsavedChanges() || hasOpenLiveWork())) { event.preventDefault(); event.returnValue = ''; }
}
/** The native host refused a reload or navigation because it holds a lease or job for this session. */
let nativeReloadHandling = false;
async function nativeReloadRequested() {
  if (nativeReloadHandling || nativeHostState.kind !== 'desktop') return;
  nativeReloadHandling = true;
  try {
    if (!(await askDiscard(`A live transaction is open, or a live statement is running or has results not yet shown. Reloading StoneQL ends it: an open transaction is rolled back or ends with an unknown outcome, and a running statement may continue on the server. An outcome the server did not confirm is listed after the reload, and after a restart, until you acknowledge it.${hasUnsavedChanges() ? ' Unsaved changes are discarded too.' : ''} Reload StoneQL?`, 'Reload StoneQL'))) return;
    reloadConfirmed = true;
    await invalidateNativeSession();
    window.location.reload();
  } catch { reloadConfirmed = false; notify('Reload could not be completed natively. Check live connections before continuing.', 'error'); }
  finally { nativeReloadHandling = false; }
}
let nativeCloseHandling = false;
async function nativeCloseRequested() {
  if (nativeCloseHandling) { void resolveNativeClose('prompted').catch(() => {}); return; }
  if (nativeHostState.kind === 'browser') return;
  nativeCloseHandling = true;
  try {
    const decision = hasOpenLiveWork() ? askDiscard(`${LIVE_WORK_CLOSE_WARNING}${hasUnsavedChanges() ? ' Unsaved changes are discarded too.' : ''} Close StoneQL?`, 'Close StoneQL') : hasUnsavedChanges() ? askDiscard('Close StoneQL and discard unsaved changes?', 'Close StoneQL') : Promise.resolve(true);
    await nextTick();
    await resolveNativeClose('prompted');
    await resolveNativeClose((await decision) ? 'confirm' : 'cancel');
  } catch { notify('Native close could not be confirmed. Your edits remain open.', 'error'); }
  finally { nativeCloseHandling = false; }
}
function repositionEditor() {
  placeCellEditor();
}
onMounted(() => {
  // Unknown outcomes recorded before a reload stay listed until acknowledged.
  if(nativeHostState.kind==='desktop')void store.refreshUnresolvedOutcomes();
  if(nativeHostState.kind==='desktop')liveStatusTimer=setInterval(()=>{if(isLive.value)void store.refreshLiveStatus(store.activeId);},30000);
  window.addEventListener('scroll', repositionEditor, true);
  window.addEventListener('resize', repositionEditor);
  media.addEventListener('change', onThemeChange);
  window.addEventListener('keydown', keyboard);
  window.addEventListener('beforeunload', beforeUnload);
  window.addEventListener('stoneql-close-requested', nativeCloseRequested);
  window.addEventListener('stoneql-reload-requested', nativeReloadRequested);
});
onBeforeUnmount(() => {
  if(liveStatusTimer)clearInterval(liveStatusTimer);
  clearTimeout(toastTimer);
  window.removeEventListener('scroll', repositionEditor, true);
  window.removeEventListener('resize', repositionEditor);
  media.removeEventListener('change', onThemeChange);
  window.removeEventListener('keydown', keyboard);
  window.removeEventListener('beforeunload', beforeUnload);
  window.removeEventListener('stoneql-close-requested', nativeCloseRequested);
  window.removeEventListener('stoneql-reload-requested', nativeReloadRequested);
});
</script>

<template>
  <div class="app-shell">
    <section v-if="store.unresolvedOutcomes.length || store.unresolvedError || store.journalProblem" class="unresolved-outcomes" role="alert" aria-label="Unknown outcomes">
      <div class="unresolved-heading">
        <AlertCircle :size="14" /><strong>Outcome unknown — the server may or may not have applied this work.</strong>
        <span class="muted">Verify on the server. Acknowledging only hides an entry; it does not change it. Kept, also across restarts, until you acknowledge it.</span>
      </div>
      <div v-if="store.unresolvedError" class="notice error"><span class="notice-message">{{ store.unresolvedError }}</span></div>
      <div v-if="store.journalProblem" class="notice error journal-problem"><span class="notice-message">{{ describeJournalProblem(store.journalProblem) }}</span></div>
      <div v-for="outcome in store.unresolvedOutcomes" :key="outcome.outcomeId" class="notice error unresolved-outcome">
        <span class="notice-message"><strong>{{ outcomeProfile(outcome) }}</strong> · {{ describeUnresolvedOutcome(outcome) }} <span class="unresolved-age">{{ outcome.restored ? `Recorded ${outcomeAge(outcome.secondsAgo)}, before StoneQL restarted.` : `Recorded ${outcomeAge(outcome.secondsAgo)}.` }}</span></span>
        <button class="button" type="button" :aria-label="`Acknowledge unknown outcome: ${describeUnresolvedOutcome(outcome)}`" @click="acknowledgeOutcome(outcome.outcomeId)">Acknowledge</button>
      </div>
    </section>
    <div class="connection-tabs" role="tablist" aria-label="Connections">
      <div class="tab-leading"><Server :size="14" /> CONNECTIONS</div>
      <div
        v-for="id in store.opened"
        :key="id"
        class="connection-tab"
        :style="{ '--connection-color': store.connections.find((c) => c.id === id)?.color }"
        :class="{ active: store.activeId === id }"
      >
        <button role="tab" :aria-selected="store.activeId === id" @keydown="tabKeydown" @click="switchConnection(id)">
          <span
            class="dot"
            :style="{ background: store.connections.find((c) => c.id === id)?.color }"
          ></span
          >{{ store.connections.find((c) => c.id === id)?.name
          }}<span
            v-if="store.connections.find((c) => c.id === id)?.environment === 'Production'"
            class="environment-tag prod"
            >PROD</span
          ><span class="engine-label">{{
            store.connections.find((c) => c.id === id)?.engine
          }}</span></button
        ><button
          class="tab-close"
          :aria-label="`Close ${store.connections.find(c => c.id === id)?.name || 'connection'} workspace`"
          @click="closeConnection(id)"
        >
          <X :size="12" />
        </button>
      </div>
      <button class="new-tab" title="Manage connections" @click="manager = true">
        <Plus :size="15" /></button
      ><span class="connection-spacer"></span
      ><button class="subtle" @click="manager = true">
        <SlidersHorizontal :size="13" /> Manage connections
      </button>
    </div>
    <div class="workbench" v-if="store.connections.length && store.opened.length">
      <aside v-if="!sidebarHidden" class="sidebar" :style="{ width: sidebarWidth + 'px' }">
        <div class="sidebar-header">
          <span>EXPLORER</span>
          <div>
            <button
              class="icon-button"
              title="Refresh explorer"
              @click="liveCatalog ? (liveSession?.status === 'connected' ? loadLiveCatalog() : notify('Connect this live profile to list its tables.', 'error')) : notify(isLive ? 'Live schema discovery is available for MySQL, MariaDB and PostgreSQL; use SQL with known table names.' : 'Mock explorer refreshed from session metadata.')"
            >
              <RefreshCw :size="13" /></button
            ><button class="icon-button" title="Collapse sidebar" @click="sidebarHidden = true">
              <PanelLeftClose :size="14" />
            </button>
          </div>
        </div>
        <div class="server-select">
          <button @click="connectionMenu = !connectionMenu">
            <span class="server-square"
              ><span class="dot" :style="{ background: store.connection?.color }"></span
              ><Server :size="16" /></span
            ><span
              ><strong>{{ store.connection?.name }}</strong
              ><span
                v-if="store.connection"
                class="environment-tag"
                :class="{ prod: store.connection.environment === 'Production' }"
                >{{ environmentLabel(store.connection) }}</span
              ><small>{{ store.connection?.host }}:{{ store.connection?.port }}</small></span
            ><ChevronDown :size="13" />
          </button>
          <div v-if="connectionMenu" class="connection-menu">
            <button v-for="c in store.connections" :key="c.id" @click="switchConnection(c.id)">
              <span class="dot" :style="{ background: c.color }"></span>{{ c.name
              }}<Check v-if="c.id === store.activeId" :size="13" /></button
            ><button
              @click="
                manager = true;
                connectionMenu = false;
              "
            >
              <Plus :size="13" /> Manage connections
            </button>
          </div>
        </div>
        <div class="tree-search">
          <Search :size="13" /><input
            v-model="treeSearch"
            placeholder="Filter databases & tables…"
            aria-label="Filter databases and tables"
          />
        </div>
        <div class="tree-caption">
          <span>DATABASES</span><span>{{ w.databases.length }}</span>
          <button v-if="liveDdlAvailable" type="button" class="icon-button" title="Create database" aria-label="Create database" @click="openCreateDatabase"><Plus :size="13" /></button>
        </div>
        <nav class="database-tree">
          <div v-for="db in treeDatabases" :key="db.name" class="database-group">
            <!-- One control: arrow, icon, name and count toggle the table list (Enter/Space too). -->
            <button
              type="button"
              class="database-row database-context-link"
              :aria-expanded="expanded.includes(db.name) || !!treeSearch"
              :aria-label="`${db.name}, ${isLive && !liveCatalog ? 'tables not listed' : isLive && db.name !== w.database && !db.tables.length ? 'open to list tables' : `${db.tables.length} tables`}`"
              @click="toggleDatabase(db.name)"
            >
              <ChevronDown v-if="expanded.includes(db.name) || treeSearch" :size="12" aria-hidden="true" /><ChevronRight v-else :size="12" aria-hidden="true" /><Database :size="15" :class="{ 'accent-icon': db.name === w.database }" aria-hidden="true" /><strong>{{ db.name }}</strong><span :title="isLive && !liveCatalog ? 'Schema discovery is not available for this engine yet' : ''">{{ isLive && !liveCatalog ? '—' : isLive && db.name !== w.database && !db.tables.length ? '' : db.tables.length }}</span>
            </button>
            <div v-if="expanded.includes(db.name) || treeSearch" class="table-tree">
              <button
                v-for="t in db.tables"
                :key="t.name"
                :class="{ selected: w.scope === 'table' && db.name === w.database && t.name === w.table }"
                @click="chooseTable(db.name, t.name)"
              >
                <Table2 :size="13" /><span>{{ t.name }}</span
                ><small v-if="!isLive">{{ t.rows.length.toLocaleString() }}</small><small v-else-if="t.kind === 'view'">view</small>
              </button>
            </div>
          </div>
          <div v-if="!treeDatabases.length" class="tree-empty">No matching tables</div>
          <p v-if="isLive && !liveCatalog" class="tree-empty">Schema discovery is available for MySQL, MariaDB and PostgreSQL. Use SQL with known table names.</p>
          <p v-else-if="isLive && liveSession?.status !== 'connected'" class="tree-empty">Connect to list tables.</p>
          <p v-else-if="isLive && liveCatalogError[store.activeId]" class="tree-empty" role="alert">Tables could not be listed: {{ liveCatalogError[store.activeId] }}</p>
          <p v-else-if="isLive && w.liveCatalogTruncated" class="tree-empty">Only the first {{ w.databases.find((d) => d.name === w.database)?.tables.length }} tables are listed (native result limit).</p>
        </nav>
        <div class="sidebar-bottom">
          <div class="connection-status">
            <span class="dot" :style="isLive && liveSession?.status!=='connected' ? {background:'var(--muted)'} : {}"></span> {{ isLive ? (liveSession?.status==='connected'?'Connected':'Disconnected') : 'Connected' }}
            <span>{{
              isLive ? (liveSession?.info?.engineVersion || 'Live · read-only') : store.connection?.engine === 'PostgreSQL'
                ? 'PostgreSQL 16.2 (mock)'
                : store.connection?.engine === 'SQLite'
                  ? 'SQLite 3.45 (mock)'
                  : store.connection?.engine === 'MariaDB'
                    ? 'MariaDB 11.4 (mock)'
                    : 'MySQL 8.0.36 (mock)'
            }}</span>
          </div>
        </div>
        <div class="resize-handle" @mousedown.prevent="resize"></div>
      </aside>
      <main class="main-workspace">
        <div class="breadcrumb">
          <button
            v-if="sidebarHidden"
            class="icon-button"
            title="Open sidebar"
            @click="sidebarHidden = false"
          >
            <PanelLeftOpen :size="15" />
          </button>
          <nav class="breadcrumb-path" aria-label="Database breadcrumb">
            <Server :size="13" />
            <button class="breadcrumb-item" :title="store.connection?.host" @click="openDatabaseContext()">
              {{ store.connection?.host }}
            </button>
            <ChevronRight v-if="w.scope !== 'server'" :size="12" /><Database v-if="w.scope !== 'server'" :size="13" />
            <button
              v-if="w.scope !== 'server'" class="breadcrumb-item"
              :title="w.database"
              @click="openDatabaseContext(w.database)"
            >
              {{ w.database }}
            </button>
            <ChevronRight v-if="w.scope === 'table'" :size="12" /><Table2 v-if="w.scope === 'table'" :size="13" />
            <button
              v-if="w.scope === 'table'" class="breadcrumb-item breadcrumb-current"
              :title="w.table || 'No table selected'"
              :disabled="!table"
              aria-current="page"
              @click="navigate('Browse')"
            >
              {{ w.table || 'No table selected' }}
            </button>
          </nav>
          <div
            v-if="w.scope === 'table' && tableMetadata.length"
            class="table-metadata"
            role="group"
            aria-label="Table metadata"
          >
            <span v-for="item in tableMetadata" :key="item.label"
              ><span class="metadata-label">{{ item.label }}:</span> {{ item.value }}</span
            >
          </div>
        </div>
        <nav class="view-tabs" role="tablist" aria-label="Workspace operations">
          <button
            v-for="tab in visibleTabs"
            :key="tab.name"
            :class="{ active: w.view === tab.name }" role="tab" :aria-selected="w.view === tab.name" @keydown="tabKeydown"
            @click="navigate(tab.name)"
          >
            <component :is="tab.icon" :size="14" />{{ tab.name === 'Insert' && insertMode ? 'Edit' : tab.name }}
          </button>
        </nav>
        <DatabaseObjects v-if="!isLive" :key="store.activeId + ':' + w.database" ref="objectManager" :section="objectSection" :confirm-discard="askDiscard" @confirm="requestObjectConfirmation" />
        <div v-if="!isLive && !table && w.scope !== 'table' && !objectSection" class="content-area"><DatabaseOverview v-if="w.view === 'Structure'" :databases="w.databases" :database="store.database" :server="w.scope === 'server'" @database="openDatabaseContext" @table="openOverviewTable" @create-table="objectManager?.openTable()" @create-view="objectManager?.openView()" @definition="objectManager?.openView($event)" @drop-view="objectManager?.dropView($event)" /><ExportWorkflow v-if="w.scope === 'database' && w.view === 'Export'" /></div>
        <div
          class="content-area"
          :class="{ 'record-content': w.view === 'Insert', 'sql-content': w.view === 'SQL', 'browse-content': w.view === 'Browse' }"
          v-if="(table || isLive) && !objectSection"
        >
          <DatabaseOverview v-if="w.scope === 'server' || (w.scope === 'database' && w.view === 'Structure')" :databases="w.databases" :database="store.database" :server="w.scope === 'server'" @database="openDatabaseContext" @table="openOverviewTable" @create-table="objectManager?.openTable()" @create-view="objectManager?.openView()" @definition="objectManager?.openView($event)" @drop-view="objectManager?.dropView($event)" />
          <template v-else-if="table && w.view === 'Browse'">
            <div v-if="isLive && liveEditReason" class="notice info" data-testid="live-read-only">{{ liveEditReason }}</div>
            <div v-if="isMockView" class="notice info">Read-only {{table.kind === 'materialized-view' ? 'materialized view' : 'view'}} · mock snapshot. Refresh evaluates only the supported SELECT subset; no server query is run.</div>
            <ResultNotice v-if="viewError" kind="error" :notification-key="browseNoticeVersion">{{viewError}}</ResultNotice>
            <ResultNotice :notification-key="browseNoticeVersion">
              Showing rows
              <strong
                >{{ rowTotal ? (w.page - 1) * store.pageSize + 1 : 0 }} –
                {{ Math.min(w.page * store.pageSize, rowTotal) }}</strong
              >
              <template v-if="liveTotalKnown">({{ rowTotal }} total · {{ isLive ? 'live, read-only' : 'mock results' }})</template><template v-else>(total unavailable: {{ liveBrowse.totalError }} · live, read-only)</template>
              <template #end
                ><span class="notice-end">{{
                  isLive ? (liveBrowse.loading ? 'Loading…' : w.filter ? 'Filtered results' : 'SELECT') : w.filter ? 'Filtered results (mock)' : 'SELECT (mock)'
                }}</span></template
              >
            </ResultNotice>
            <SqlQueryPreview
              v-model:expanded="store.sqlPreviewExpanded"
              @edit="openSqlText(isLive ? liveBrowse.sql : browsePlan.sql)"
              ><SqlText :text="isLive ? liveBrowse.sql : browsePlan.sql" :engine="store.connection.engine"
            /></SqlQueryPreview>
            <div class="grid-toolbar">
              <div class="toolbar-left">
                <button class="button" @click="refreshBrowse">
                  <RefreshCw :size="13" /> Refresh</button
                ><span class="separator"></span><span class="muted">Show</span
                ><select v-model.number="store.pageSize" aria-label="Rows per page">
                  <option :value="15">15</option>
                  <option :value="25">25</option>
                  <option :value="50">50</option>
                  <option :value="100">100</option>
                  <option :value="250">250</option>
                  <option :value="500">500</option></select
                ><span class="muted">rows</span>
              </div>
              <div class="toolbar-pagination">
                <button
                  class="page-button"
                  :disabled="w.page === 1"
                  aria-label="Previous page in toolbar"
                  @click="w.page--"
                >
                  <ChevronLeft :size="14" /></button
                ><span>{{ w.page }} / {{ pages }}</span
                ><button
                  class="page-button"
                  :disabled="w.page >= pages"
                  aria-label="Next page in toolbar"
                  @click="w.page++"
                >
                  <ChevronRight :size="14" />
                </button>
              </div>
              <div class="filter-field">
                <Search :size="13" /><input
                  v-model="w.filter"
                  placeholder="Filter rows…"
                  aria-label="Filter rows"
                /><button v-if="w.filter" title="Clear filter" @click="w.filter = ''">
                  <X :size="12" />
                </button>
              </div>
              <button v-if="!isLive" class="button filter-button" @click="navigate('Search')">
                <ListFilter :size="13" /> Search
              </button>
            </div>
            <div class="data-grid-wrap">
              <table
                ref="browseGrid"
                class="data-grid browse-grid"
                role="grid"
                aria-label="Table records"
                v-grid-resize="browseGridOptions"
                @copy="copyRawSelection"
              >
                <thead>
                  <tr>
                    <th v-if="!isReadOnly" class="check-cell">
                      <input
                        type="checkbox"
                        aria-label="Select all visible records"
                        :checked="rows.length > 0 && rows.every((r) => selected.includes(r))"
                        @change="selectAll"
                      />
                    </th>
                    <th v-if="!isReadOnly" class="actions-column">Actions</th>
                    <th
                      v-for="c in table.columns"
                      :key="c.name"
                      :aria-sort="
                        w.sort === c.name ? (w.descending ? 'descending' : 'ascending') : 'none'
                      "
                    >
                      <button @click="sort(c.name)">
                        <ColumnKeys :column="c" />{{ c.name
                        }}<ArrowDown
                          v-if="w.sort === c.name && w.descending"
                          :size="12"
                          class="sorted"
                        /><ArrowUp
                          v-else-if="w.sort === c.name"
                          :size="12"
                          class="sorted"
                        /><ArrowUpDown v-else :size="11" />
                      </button>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="(row, rowIndex) in rows"
                    :key="String(row.id ?? JSON.stringify(row))"
                    :class="{ 'row-selected': selected.includes(row) }"
                  >
                    <td v-if="!isReadOnly" class="check-cell">
                      <input
                        type="checkbox"
                        v-model="selected"
                        :value="row"
                        :aria-label="`Select row ${row.id}`"
                        :tabindex="rowIndex === gridFocus.row ? 0 : -1"
                      />
                    </td>
                    <td v-if="!isReadOnly" class="row-actions">
                      <button
                        @click="editRow(row)"
                        :title="`Edit record ${row.id ?? ''}`"
                        :aria-label="`Edit record ${row.id ?? ''}`"
                        :tabindex="rowIndex === gridFocus.row ? 0 : -1"
                      >
                        <Pencil :size="14" /><span class="sr-only">Edit</span></button
                      ><button
                        @click="editRow(row, true)"
                        :title="`Copy record ${row.id ?? ''}`"
                        :aria-label="`Copy record ${row.id ?? ''}`"
                        :tabindex="rowIndex === gridFocus.row ? 0 : -1"
                      >
                        <Copy :size="14" /><span class="sr-only">Copy</span></button
                      ><button
                        class="delete-link"
                        @click="removeRows([row])"
                        :title="`Delete record ${row.id ?? ''}`"
                        :aria-label="`Delete record ${row.id ?? ''}`"
                        :tabindex="rowIndex === gridFocus.row ? 0 : -1"
                      >
                        <Trash2 :size="14" /><span class="sr-only">Delete</span>
                      </button>
                    </td>
                    <td
                      v-for="(c, columnIndex) in table.columns"
                      :key="c.name"
                      data-grid-column
                      :tabindex="
                        gridFocus.row === rowIndex && gridFocus.column === columnIndex ? 0 : -1
                      "
                      @focus="gridFocus = { row: rowIndex, column: columnIndex }"
                      @keydown="gridKeydown($event, row, rowIndex, columnIndex)"
                      @dblclick="editCell(row, c.name, $event)"
                      :title="row[c.name] === null ? 'SQL NULL' : String(row[c.name])"
                      :class="{
                        'numeric-cell': /^(int|decimal)/.test(c.type),
                      }"
                    >
                      <div
                        v-dialog="{ close: cancelCellEdit, returnFocus: editing?.anchor }"
                        id="inline-cell-editor"
                        class="cell-editor"
                        role="dialog"
                        :aria-label="`Edit cell ${editing?.column}`"
                        :style="
                          editing
                            ? {
                                left: editing.left + 'px',
                                top: editing.top === null ? 'auto' : editing.top + 'px',
                                bottom: editing.bottom === null ? 'auto' : editing.bottom + 'px',
                                maxHeight:
                                  editing.maxHeight === null
                                    ? undefined
                                    : `min(300px, 70vh, ${editing.maxHeight}px)`,
                              }
                            : undefined
                        "
                        :class="{ 'multiline-editor': editing?.multiline }"
                        @keydown.enter="cellEditorEnter"
                        v-if="editing?.row === row && editing.column === c.name"
                      >
                        <EnumEditor
                          v-if="choiceDefinition(c, store.connection.engine)"
                          v-model="editing.value"
                          :column="c"
                          :engine="store.connection.engine"
                          :label="`Edit ${c.name}`"
                          :disabled="editing.isNull || editing.useDefault"
                          :invalid="!!editing.error"
                          :describedby="editing.error ? 'inline-value-error' : undefined"
                        />
                        <RawTextarea
                          v-else-if="editing.multiline"
                          v-model="editing.value"
                          v-model:issue="editing.lineIssue"
                          :label="`Edit ${c.name}`"
                          :rows="5"
                          :disabled="editing.isNull || editing.useDefault"
                          :invalid="!!editing.error"
                          :describedby="editing.error ? 'inline-value-error' : undefined"
                        />
                        <input
                          v-else
                          v-model="editing.value"
                          :aria-label="`Edit ${c.name}`"
                          :disabled="editing.isNull || editing.useDefault"
                          :aria-invalid="!!editing.error || undefined"
                          :aria-describedby="editing.error ? 'inline-value-error' : undefined"
                        />
                        <label v-if="c.nullable" class="compact-choice"
                          ><input
                            type="checkbox"
                            v-model="editing.isNull"
                            @change="editing.useDefault = false"
                          />
                          NULL</label
                        >
                        <label v-if="hasColumnDefault(c)" class="compact-choice"
                          ><input
                            type="checkbox"
                            v-model="editing.useDefault"
                            @change="editing.isNull = false"
                          />
                          Default</label
                        >
                        <button title="Save edit" aria-label="Save cell edit" @click="saveCell">
                          <Check :size="12" /></button
                        ><button
                          title="Discard edit"
                          aria-label="Discard cell edit"
                          @click="cancelCellEdit()"
                        >
                          <X :size="12" /></button
                        ><span
                          v-if="editing.error"
                          id="inline-value-error"
                          class="field-error"
                          role="alert"
                          >{{ editing.error }}</span
                        >
                      </div>
                      <i v-else-if="row[c.name] === null" class="null-value" role="img" aria-label="SQL NULL">NULL</i
                      ><span v-else-if="!invisibleSegments(row[c.name])">{{ row[c.name] }}</span
                      ><span v-else><template v-for="(s, k) in invisibleSegments(row[c.name])" :key="k"
                        ><span v-if="s.marker" class="invisible-marker" role="img" :aria-label="s.label" :data-raw="s.raw">{{ s.marker }}</span
                        ><template v-else>{{ s.text }}</template></template
                      ></span>
                    </td>
                  </tr>
                  <tr v-if="!rows.length">
                    <td :colspan="table.columns.length + 2" class="empty-state">
                      <Search :size="26" /><strong>No records found</strong
                      ><span>{{
                        w.filter
                          ? 'Try a different filter or clear your search.'
                          : isLive ? 'This table has no rows.' : 'Insert a record to get started.'
                      }}</span
                      ><button
                        v-if="w.filter || !isLive"
                        class="button"
                        @click="w.filter ? (w.filter = '') : navigate('Insert')"
                      >
                        {{ w.filter ? 'Clear filter' : 'Insert row' }}
                      </button>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
            <div v-if="!isReadOnly" class="selection-toolbar">
              <span class="selection-arrow">↳</span
              ><button @click="selected = rows.every((r) => selected.includes(r)) ? [] : [...rows]">
                {{
                  rows.length && rows.every((r) => selected.includes(r))
                    ? 'Uncheck all'
                    : 'Check all'
                }}</button
              ><span class="muted">With selected:</span
              ><button v-if="!isLive" :disabled="!selected.length" @click="copySelected()">
                <Copy :size="12" /> Copy</button
              ><button
                :disabled="!selected.length"
                class="delete-link"
                @click="removeRows(selected)"
              >
                <Trash2 :size="12" /> Delete</button
              ><span v-if="selected.length" class="selection-count"
                >{{ selected.length }} selected</span
              ><span class="toolbar-end"><Pencil :size="11" /> Double-click a cell to edit</span>
            </div>
            <div class="pagination">
              <span class="muted"
                >Showing
                <strong
                  >{{ rowTotal ? (w.page - 1) * store.pageSize + 1 : 0 }}–{{
                    Math.min(w.page * store.pageSize, rowTotal)
                  }}</strong
                >
                <template v-if="liveTotalKnown">of <strong>{{ rowTotal }}</strong> rows</template></span
              >
              <div>
                <button
                  class="page-button"
                  :disabled="w.page === 1"
                  title="First page"
                  @click="w.page = 1"
                >
                  <ChevronsLeft :size="14" /></button
                ><button
                  class="page-button"
                  :disabled="w.page === 1"
                  title="Previous page"
                  @click="w.page--"
                >
                  <ChevronLeft :size="14" /></button
                ><span>Page</span
                ><input
                  type="number"
                  v-model.number="w.page"
                  min="1"
                  :max="pages"
                  aria-label="Page number"
                  @change="w.page = Math.max(1, Math.min(pages, w.page || 1))"
                /><span>of {{ pages }}</span
                ><button
                  class="page-button"
                  :disabled="w.page >= pages"
                  title="Next page"
                  @click="w.page++"
                >
                  <ChevronRight :size="14" /></button
                ><button
                  class="page-button"
                  :disabled="w.page >= pages"
                  title="Last page"
                  @click="w.page = pages"
                >
                  <ChevronsRight :size="14" />
                </button>
              </div>
            </div>
          </template>
          <template v-else-if="table && w.view === 'Structure'"
            ><div v-if="isMockView" class="panel"><div class="workspace-toolbar"><span>{{table.kind==='materialized-view'?'Materialized view':'View'}} · read-only mock snapshot</span><div class="row-actions"><button @click="objectManager?.openView(table)">Definition / Edit</button><button @click="refreshBrowse">{{table.kind==='materialized-view'?'Refresh materialized view (mock)':'Refresh snapshot (mock)'}}</button><button class="danger-text" @click="objectManager?.dropView(table)">Drop view</button></div></div><pre class="object-definition"><SqlText :text="table.definition || ''" :engine="store.connection.engine" /></pre><div v-if="viewError" class="notice error" role="alert">{{viewError}}</div></div><div class="workspace-toolbar">
              <span class="muted">{{ table.columns.length }} columns</span
              ><button v-if="(!isReadOnly && !isLive) || liveStructureEditable" class="button" @click="isLive ? openLiveColumn() : editColumn()"><Plus :size="13" /> Add column</button>
            </div>
            <div class="data-grid-wrap schema-grid-wrap">
              <table class="data-grid structure-grid">
                <colgroup>
                  <col class="structure-number" />
                  <col class="structure-name" />
                  <col class="structure-type" />
                  <col class="structure-null" />
                  <col class="structure-default" />
                  <col class="structure-extra" />
                  <col class="structure-actions" />
                </colgroup>
                <thead>
                  <tr>
                    <th>#</th>
                    <th>Name</th>
                    <th>Type</th>
                    <th>Null</th>
                    <th>Default</th>
                    <th>Extra</th>
                    <th v-if="(!isReadOnly && !isLive) || liveStructureEditable">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="(c, i) in table.columns" :key="c.name">
                    <td>{{ i + 1 }}</td>
                    <td>
                      <ColumnName :column="c" />
                      <span v-if="c.comment || c.collation" class="schema-detail" tabindex="0" :aria-label="[c.collation, c.comment].filter(Boolean).join(' · ')" :title="[c.collation ? `Collation: ${c.collation}` : '', c.comment ? `Comment: ${c.comment}` : ''].filter(Boolean).join(' · ')"><CircleHelp :size="12" /></span>
                    </td>
                    <td class="mono" :title="c.type">{{ c.type }}</td>
                    <td>{{ c.nullable ? 'Yes' : 'No' }}</td>
                    <td class="mono muted">
                      {{ c.defaultKind === 'null' ? 'NULL' : (c.default ?? 'None') }}
                    </td>
                    <td>{{ (c.autoIncrement) ? columnCap.autoLabel : '' }}</td>
                    <td v-if="(!isReadOnly && !isLive) || liveStructureEditable">
                      <div class="inline-actions">
                        <button :aria-label="`Change column ${c.name}`" @click="isLive ? openLiveColumn(c) : editColumn(c)"><Pencil :size="12" />Change</button
                        ><button class="delete-link" :aria-label="`Drop column ${c.name}`" @click="isLive ? dropLiveColumn(c) : dropColumn(c)">
                          <Trash2 :size="12" />Drop
                        </button>
                      </div>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
            <IndexManager v-if="!isLive && (!isReadOnly || table.kind === 'materialized-view')" :confirm-discard="askDiscard" @confirm="requestObjectConfirmation" />
            <template v-if="isLive">
              <p v-if="table.live?.error" class="notice error" role="alert">Indexes and foreign keys could not be read: {{ table.live.error }}</p>
              <div class="panel" data-testid="live-indexes">
                <h3><SchemaIcon kind="index" detail="Indexes" /> Indexes<button v-if="liveStructureEditable" type="button" class="button panel-action" @click="openLiveIndex"><Plus :size="13" /> Add index</button></h3>
                <table class="data-grid">
                  <thead><tr><th>Name</th><th>Unique</th><th>{{ store.connection.engine === 'PostgreSQL' ? 'Definition' : 'Columns' }}</th><th v-if="store.connection.engine !== 'PostgreSQL'">Type</th><th v-if="liveStructureEditable">Actions</th></tr></thead>
                  <tbody>
                    <tr v-for="index in table.live?.indexes || []" :key="index.name">
                      <td class="mono">{{ index.name }}</td><td>{{ index.unique ? 'Yes' : 'No' }}</td>
                      <td class="mono">{{ index.definition ?? index.columns.join(', ') }}</td><td v-if="store.connection.engine !== 'PostgreSQL'">{{ index.type }}</td>
                      <td v-if="liveStructureEditable"><button v-if="!index.constraint" class="delete-link" :aria-label="`Drop index ${index.name}`" @click="dropLiveIndex(index.name)"><Trash2 :size="12" />Drop</button><span v-else class="muted" title="Dropped only with its primary key or constraint">Constraint</span></td>
                    </tr>
                    <tr v-if="!table.live?.indexes.length"><td :colspan="(store.connection.engine === 'PostgreSQL' ? 3 : 4) + (liveStructureEditable ? 1 : 0)" class="muted">No indexes visible to this account.</td></tr>
                  </tbody>
                </table>
              </div>
              <div class="panel" data-testid="live-foreign-keys">
                <h3><SchemaIcon kind="foreign" detail="Relationships" /> Foreign key relationships</h3>
                <table class="data-grid">
                  <thead><tr><th>Name</th><th>Columns</th><th>References</th><th>On delete</th><th>On update</th></tr></thead>
                  <tbody>
                    <tr v-for="key in table.live?.foreignKeys || []" :key="key.name">
                      <td class="mono">{{ key.name }}</td><td class="mono">{{ key.columns.join(', ') }}</td>
                      <td class="mono">{{ key.refTable }}({{ key.refColumns.join(', ') }})</td><td>{{ key.onDelete }}</td><td>{{ key.onUpdate }}</td>
                    </tr>
                    <tr v-if="!table.live?.foreignKeys.length"><td colspan="5" class="muted">No foreign keys on this table.</td></tr>
                  </tbody>
                </table>
              </div>
            </template>
            <div v-else class="panel">
              <h3>
                <SchemaIcon kind="foreign" detail="Relationships" /> Foreign key relationships
              </h3>
              <table class="data-grid">
                <thead>
                  <tr>
                    <th>Column</th>
                    <th>References</th>
                    <th>On delete</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="c in table.columns.filter((c) => c.foreign)" :key="c.name">
                    <td><ColumnName :column="c" /></td>
                    <td>{{ c.foreign }}</td>
                    <td>RESTRICT</td>
                  </tr>
                  <tr v-if="!table.columns.some((c) => c.foreign)">
                    <td colspan="3" class="muted">
                      No foreign keys on this table. Select orders or order_items to explore
                      relationships.
                    </td>
                  </tr>
                </tbody>
              </table>
            </div></template
          >
          <template v-else-if="w.view === 'SQL'">
            <div v-if="isLive" class="editor-toolbar">
              <span>{{ store.connection?.engine === 'SQLite' ? 'Live · SQLite file' : `Live · ${store.connection?.transport.type === 'ssh' ? 'SSH tunnel · Verified host key' : 'Direct'} · ${store.connection && !usesTls(store.connection) ? tlsOffLabel(store.connection) : 'Verified TLS'}` }}</span>
              <span class="access-badge" :class="{ 'read-write': liveReadWrite }" data-testid="live-access">{{ liveReadWrite ? 'Read-write' : 'Read-only' }}</span>
              <span class="toolbar-end">{{ liveSession?.status || 'disconnected' }}</span>
              <button v-if="liveSession?.status!=='connected'" class="button" :disabled="liveSession?.status==='connecting'" @click="newConnection(store.connection)">Connect</button>
              <template v-else>
                <button class="button" @click="disconnectLive">Disconnect</button>
              </template>
            </div>
            <div v-if="liveReadWrite" class="editor-toolbar transaction-toolbar" role="group" aria-label="Transaction">
              <template v-if="!liveLease">
                <span class="muted">Autocommit · each statement commits on success</span>
                <button class="button" :disabled="running" @click="transaction('begin')">Begin transaction</button>
              </template>
              <template v-else>
                <span :class="liveLease.state === 'active' ? 'accent-text' : 'danger-text'">{{ liveLease.state === 'active' ? 'Transaction open' : 'Transaction failed · rollback only' }}</span>
                <span class="muted">Ends after 60 s idle or 15 min; outcome shown on Commit, Rollback or Disconnect</span>
                <button class="button" :disabled="running || liveLease.state !== 'active'" @click="transaction('commit')">Commit</button>
                <button class="button" :disabled="running" @click="transaction('rollback')">Rollback</button>
              </template>
            </div>
            <ResultNotice v-if="isLive && liveSession?.error" kind="error" :notification-key="liveSession.error">{{ liveSession.error }}</ResultNotice>
            <div class="query-tabs">
              <div v-for="q in w.queries" :key="q.id" :class="{ active: q.id === w.queryId }">
                <button @click="w.queryId = q.id">
                  <Code2 :size="12" /> {{ q.title }}{{ q.sql !== q.saved ? ' •' : '' }}</button
                ><button :aria-label="`Close ${q.title}`" @click="closeQuery(q.id)">
                  <X :size="11" />
                </button>
              </div>
              <button title="New query" @click="newQuery"><Plus :size="14" /></button>
            </div>
            <SqlEditor
              :key="store.activeId + w.database + w.scope"
              v-model="query.sql"
              :dark="dark"
              :schema="schema" :engine="store.connection.engine"
              @execute="store.shortcuts && execute()" />
            <div class="editor-toolbar">
              <button class="button primary" :disabled="running || (isLive && liveSession?.status!=='connected')" @click="execute">
                <LoaderCircle v-if="running" :size="13" class="spin" /><Play v-else :size="13" />{{
                  running ? 'Executing…' : 'Run query'
                }}</button
              ><button v-if="running && query.jobId" class="button danger" @click="cancelExecution"><Square :size="12" /> Cancel</button
              ><span class="muted">⌘ / Ctrl + Enter</span
              ><button
                class="button"
                @click="
                  query.saved = query.sql;
                  download(query.sql, 'query.sql', 'text/plain');
                  notify('Query saved as a SQL file.');
                "
              >
                <Save :size="13" /> Save SQL</button
              ><button class="button" @click="showHistory = !showHistory">
                <Clock3 :size="13" /> Query history
                <span class="count">{{ w.history.length }}</span></button
              ><span class="toolbar-end">{{ liveReadWrite ? 'Live read-write · scripts, bounded results' : isLive ? 'Live read-only · bounded SELECT subset' : 'Mock engine · SELECT queries supported' }}</span>
            </div>
            <div v-if="showHistory" class="panel history-panel">
              <h3>Query history</h3>
              <p v-if="!w.history.length" class="muted">Execute a query to see it here.</p>
              <button v-for="(h, i) in w.history" :key="i" @click="restoreHistory(h.sql)">
                <Clock3 :size="12" /><code>{{ h.sql }}</code
                ><span>{{ h.time }}</span
                ><span :class="h.status === 'Error' ? 'danger-text' : 'accent-text'">{{
                  h.status
                }}</span>
              </button>
            </div>
            <ResultNotice v-if="sqlError" kind="error" :notification-key="w.history.length">{{
              sqlError
            }}</ResultNotice>
            <div v-if="query.job" class="statement-outcomes" aria-label="Statement outcomes">
              <span v-for="s in query.job.statements" :key="s.index" class="statement-outcome" :class="s.status" :title="s.failure?.sqlState ? `SQLSTATE ${s.failure.sqlState}` : undefined"
                >#{{ s.index + 1 }} {{ s.status.replace('_', ' ') }}<template v-if="s.status === 'succeeded' && s.affectedRows !== '0'"> · {{ s.affectedRows }} rows</template><template v-if="s.inTransaction && s.status === 'succeeded'"> · uncommitted</template></span>
            </div>
            <div v-if="(query.resultSets?.length || 0) > 1" class="query-tabs result-sets" role="tablist" aria-label="Result sets">
              <div v-for="(set, i) in query.resultSets" :key="`${set.statement}:${set.result}`" :class="{ active: i === (query.resultIndex || 0) }">
                <button role="tab" :aria-selected="i === (query.resultIndex || 0)" @click="selectResultSet(i)">Result {{ i + 1 }} · statement {{ set.statement + 1 }}</button>
              </div>
            </div>
            <ResultNotice v-if="query.job && jobSucceeded && !result" :notification-key="w.history.length">{{ describeJob(query.job, query.job.statements.some((s) => s.inTransaction)).text }}</ResultNotice>
            <template v-if="result"
              ><ResultNotice :notification-key="w.history.length"
                >{{ result.rows.length }} rows returned<template v-if="query.job && !jobSucceeded"> by statement {{ (query.resultSets?.[query.resultIndex || 0]?.statement ?? 0) + 1 }} (job {{ query.job.outcome }}; see the outcome above)</template><template v-else>. Query executed successfully ({{ liveReadWrite ? 'live, read-write' : isLive ? 'live, read-only' : 'simulated' }},
                {{ query.resultDatabase }})</template>.</ResultNotice
              >
              <ResultNotice v-if="result.truncated" kind="warning" :notification-key="w.history.length">{{ query.job ? 'Result limited by the configured row or byte bounds.' : 'Result limited to 1,000 rows or 1 MiB.' }} Individual values were not truncated.</ResultNotice>
              <div class="data-grid-wrap">
                <table class="data-grid" v-grid-resize="resultGridOptions" @copy="copyRawSelection">
                  <thead>
                    <tr>
                      <th
                        v-for="c in result.columns"
                        :key="c"
                        :aria-sort="
                          query.resultSort === c
                            ? query.resultDescending
                              ? 'descending'
                              : 'ascending'
                            : 'none'
                        "
                      >
                        <button @click="sortResults(c)">
                          {{ result.columnLabels?.[c] || c
                          }}<ArrowDown
                            v-if="query.resultSort === c && query.resultDescending"
                            :size="12"
                          /><ArrowUp v-else-if="query.resultSort === c" :size="12" /><ArrowUpDown
                            v-else
                            :size="11"
                          />
                        </button>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr
                      v-for="(r, i) in resultRows.slice(
                        (resultPage - 1) * store.pageSize,
                        resultPage * store.pageSize,
                      )"
                      :key="i"
                    >
                      <td
                        v-for="c in result.columns"
                        :key="c"
                        :title="r[c] === null ? 'SQL NULL' : String(r[c] ?? '')"
                      >
                        <i v-if="r[c] === null" class="null-value" role="img" aria-label="SQL NULL">NULL</i
                        ><span v-else-if="!invisibleSegments(r[c])">{{ r[c] }}</span
                      ><span v-else><template v-for="(s, k) in invisibleSegments(r[c])" :key="k"
                        ><span v-if="s.marker" class="invisible-marker" role="img" :aria-label="s.label" :data-raw="s.raw">{{ s.marker }}</span
                        ><template v-else>{{ s.text }}</template></template
                      ></span>
                      </td>
                    </tr>
                    <tr v-if="!result.rows.length">
                      <td :colspan="result.columns.length">The query returned no records.</td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <div class="pagination">
                <span>{{ result.rows.length }} result rows</span>
                <div>
                  <button class="button" :disabled="resultPage === 1" @click="resultPage--">
                    Previous</button
                  ><span
                    >Page {{ resultPage }} of
                    {{ Math.max(1, Math.ceil(result.rows.length / store.pageSize)) }}</span
                  ><button
                    class="button"
                    :disabled="resultPage * store.pageSize >= result.rows.length"
                    @click="resultPage++"
                  >
                    Next
                  </button>
                </div>
              </div></template
            >
            <div v-else-if="!sqlError" class="sql-empty">
              <Terminal :size="28" /><strong>Your next query starts here.</strong
              ><span>Write SQL above and press ⌘ / Ctrl + Enter to run.</span
              ><button v-if="!isLive" @click="query.sql = `SELECT * FROM ${w.table} LIMIT 25;`">
                Load a sample query <ArrowRight :size="12" />
              </button></div
          ></template>
          <template v-else-if="table && w.view === 'Search'">
            <form @submit.prevent="runSearch">
              <div v-if="searchError" class="notice error" role="alert">{{ searchError }}</div>
              <table class="data-grid form-grid">
                <thead>
                  <tr>
                    <th>Column</th>
                    <th>Type</th>
                    <th>Operator</th>
                    <th>Value</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="c in table.columns" :key="c.name">
                    <td>
                      <strong>{{ c.name }}</strong>
                    </td>
                    <td class="mono muted" :title="c.type">{{ c.type }}</td>
                    <td>
                      <select v-model="searchOperators[c.name]" :aria-label="`${c.name} operator`">
                        <option value="contains">contains</option>
                        <option v-for="operator in availableSearchOperators.filter(o => o !== 'contains')" :key="operator" :value="operator">{{ operator }}</option>
                      </select>
                    </td>
                    <td>
                      <input
                        v-model="searchFilters[c.name]"
                        :aria-label="`Search ${c.name}`"
                        :placeholder="['IN', 'NOT IN', 'BETWEEN'].includes(searchOperators[c.name] || '') ? 'Comma-separated values' : `Any ${c.name}`"
                        :disabled="['IS NULL', 'IS NOT NULL'].includes(searchOperators[c.name] || '')"
                      />
                    </td>
                  </tr>
                </tbody>
              </table>
              <div class="form-footer">
                <button
                  class="button"
                  type="button"
                  @click="resetSearch"
                >
                  Reset</button
                ><button class="button primary"><Search :size="13" /> Search records</button>
              </div>
            </form>
            <div v-if="searchResults !== null" class="panel">
              <h3>
                {{ searchResults.length }} matching records
                <span v-if="searchResults.length > 100" class="muted">(first 100 shown)</span>
              </h3>
              <div class="data-grid-wrap">
                <table class="data-grid" v-grid-resize="searchGridOptions" @copy="copyRawSelection">
                  <thead>
                    <tr>
                      <th v-for="c in table.columns" :key="c.name"><ColumnName :column="c" /></th>
                      <th v-if="!isReadOnly">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr v-for="(row, i) in searchResults.slice(0, 100)" :key="i">
                      <td v-for="c in table.columns" :key="c.name" :title="row[c.name] === null ? 'SQL NULL' : String(row[c.name])"><i v-if="row[c.name] === null" class="null-value" role="img" aria-label="SQL NULL">NULL</i
                      ><span v-else-if="!invisibleSegments(row[c.name])">{{ row[c.name] }}</span
                      ><span v-else><template v-for="(s, k) in invisibleSegments(row[c.name])" :key="k"
                        ><span v-if="s.marker" class="invisible-marker" role="img" :aria-label="s.label" :data-raw="s.raw">{{ s.marker }}</span
                        ><template v-else>{{ s.text }}</template></template
                      ></span></td>
                      <td v-if="!isReadOnly" class="row-actions"><button @click="editRow(row)">Edit</button><button class="danger-text" @click="removeRows([row])">Delete</button></td>
                    </tr>
                    <tr v-if="!searchResults.length">
                      <td :colspan="table.columns.length + 1">
                        No records match these conditions.
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div></template
          >
          <template v-else-if="table && w.view === 'Insert'">
            <form
              class="record-form"
              :aria-label="insertMode ? 'Edit record' : 'Insert record'"
              @submit.prevent="commitInsert"
            >
              <table class="data-grid form-grid record-grid">
                <colgroup>
                  <col class="record-name-col" />
                  <col class="record-type-col" />
                  <col class="record-null-col" />
                  <col />
                </colgroup>
                <thead>
                  <tr>
                    <th>Column</th>
                    <th>Type</th>
                    <th>Null / Default</th>
                    <th>Value</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="c in table.columns" :key="c.name">
                    <td><ColumnName :column="c" /></td>
                    <td class="mono muted" :title="c.type">{{ c.type }}</td>
                    <td>
                      <input
                        v-if="c.nullable"
                        type="checkbox"
                        v-model="recordNulls[c.name]"
                        :aria-label="`NULL for ${c.name}`"
                        @change="recordDefaults[c.name] = false"
                      /><span v-else>No</span>
                      <label v-if="hasColumnDefault(c)" class="compact-choice record-default"
                        ><input
                          type="checkbox"
                          v-model="recordDefaults[c.name]"
                          @change="recordNulls[c.name] = false"
                        />
                        Default</label
                      >
                    </td>
                    <td>
                      <EnumEditor
                        v-if="choiceDefinition(c, store.connection.engine)"
                        v-model="insertValues[c.name]"
                        :column="c"
                        :engine="store.connection.engine"
                        :label="`Value for ${c.name}`"
                        :disabled="recordNulls[c.name] || recordDefaults[c.name]"
                        :invalid="!!recordErrors[c.name]"
                        :describedby="recordErrors[c.name] ? `record-error-${c.name}` : undefined"
                      />
                      <RawTextarea
                        v-else-if="/text/i.test(c.type) || recordMultiline[c.name]"
                        v-model="insertValues[c.name]"
                        v-model:issue="recordLineIssues[c.name]"
                        :label="`Value for ${c.name}`"
                        :disabled="recordNulls[c.name] || recordDefaults[c.name]"
                        :invalid="!!recordErrors[c.name]"
                        :describedby="recordErrors[c.name] ? `record-error-${c.name}` : undefined"
                      />
                      <input
                        v-else
                        v-model="insertValues[c.name]"
                        :aria-label="`Value for ${c.name}`"
                        :disabled="recordNulls[c.name] || recordDefaults[c.name]"
                        :aria-invalid="!!recordErrors[c.name] || undefined"
                        :aria-describedby="
                          recordErrors[c.name] ? `record-error-${c.name}` : undefined
                        "
                        :placeholder="(c.autoIncrement) ? 'Auto increment' : c.name"
                      />
                      <span
                        v-if="recordErrors[c.name]"
                        :id="`record-error-${c.name}`"
                        class="field-error"
                        role="alert"
                        >{{ recordErrors[c.name] }}</span
                      >
                    </td>
                  </tr>
                </tbody>
              </table>
              <div class="form-footer record-actions">
                <span v-if="insertDirty" class="unsaved" role="status">Unsaved changes</span>
                <button type="button" class="button" @click="navigate('Browse')">Cancel</button
                ><button class="button primary">
                  <Check :size="13" />{{ insertMode ? 'Save changes' : 'Insert record' }}
                </button>
              </div>
            </form></template
          >
          <LiveExport v-else-if="w.view === 'Export' && isLive" :key="store.activeId + w.scope + w.table" :whole="w.scope === 'database'" /><ExportWorkflow
            v-else-if="w.view === 'Export'"
            :key="store.activeId + w.database + w.scope"
          /><div v-else-if="isLive && w.scope === 'database' && w.view === 'Import'" class="panel transfer-panel" data-testid="database-import">
            <h3>Import a SQL file into {{ w.database }}</h3>
            <LiveSqlImport :key="store.activeId" />
          </div><template v-else-if="table && w.view === 'Import'">
            <div class="panel transfer-panel">
              <h3>File to import</h3>
              <label v-if="w.scope === 'database'" class="form-label">Target table<select v-model="w.table"><option v-for="target in store.database?.tables" :key="target.name" :value="target.name">{{ target.name }}</option></select></label>
              <div v-if="importError" class="notice error" role="alert">{{ importError }}</div>
              <div v-if="importNotice" class="notice" role="status">{{ importNotice }}</div>
              <label v-if="!(isLive && importFormat === 'SQL')" class="upload-area"
                ><Upload :size="16" /><strong>{{
                  importFile?.name || 'Choose a SQL or CSV file'
                }}</strong
                ><span>Maximum file size: {{ isLive ? '64 MiB' : '2 MB' }}</span
                ><input
                  type="file"
                  accept=".csv,.sql"
                  @change="
                    importFile = ($event.target as HTMLInputElement).files?.[0] ?? null;
                    importFormat = importFile?.name.endsWith('.sql') ? 'SQL' : 'CSV';
                  " /></label
              ><label class="form-label"
                >Format<select v-model="importFormat">
                  <option>CSV</option>
                  <option>SQL</option>
                </select></label
              >
              <LiveSqlImport v-if="isLive && importFormat === 'SQL'" :key="store.activeId" />
              <template v-else>
              <div class="notice info">
                <CircleHelp :size="15" />{{
                  importFormat === 'CSV'
                    ? 'CSV headers must match column names. Duplicate primary keys are rejected.'
                    : 'SQL import simulates progress and completion. It does not execute SQL statements.'
                }}
              </div>
              <div v-if="transferBusy" class="progress-track">
                <div :style="{ width: transferProgress + '%' }"></div>
              </div>
              <div class="form-footer">
                <button class="button primary" :disabled="transferBusy" @click="importData()">
                  <LoaderCircle v-if="transferBusy" :size="13" class="spin" /><Upload
                    v-else
                    :size="13"
                  />{{ transferBusy ? `${transferProgress}%` : 'Import' }}
                </button>
              </div>
              </template>
            </div></template
          >
          <template v-else-if="table && w.view === 'Operations'">
            <div class="operations-grid">
              <form class="panel" @submit.prevent="renameTable">
                <h3>Rename table</h3>
                <label class="form-label"
                  >New table name<input
                    v-model="operationName"
                    :placeholder="w.table"
                    required /></label
                ><button class="button" type="submit"><Pencil :size="13" /> Rename</button>
              </form>
              <form class="panel" @submit.prevent="copyTable">
                <h3>Copy table</h3>
                <label class="form-label"
                  >New table name<input
                    v-model="copyName"
                    :placeholder="`${w.table}_copy`"
                    required /></label
                ><label class="checkbox-label"
                  ><input type="checkbox" v-model="copyData" /> Include records</label
                ><button class="button"><Copy :size="13" /> Copy table</button>
              </form>
              <div class="panel danger-panel">
                <h3>Truncate table</h3>
                <p>Delete all {{ table.rows.length }} records. Keep the table structure.</p>
                <button class="button danger" @click="destructiveOperation(false)">
                  <Trash2 :size="13" /> Truncate table
                </button>
              </div>
              <div class="panel danger-panel">
                <h3>Drop table</h3>
                <p>Permanently remove this table and all of its records.</p>
                <button class="button danger" @click="destructiveOperation(true)">
                  <Trash2 :size="13" /> Drop table
                </button>
              </div>
            </div></template
          >
        </div>
        <div v-else-if="w.scope === 'table' && !objectSection" class="empty-state">
          <Database :size="32" />
          <h2>No table selected</h2>
          <p>Select a table in the database explorer.</p>
        </div>
      </main>
    </div>
    <div v-else class="empty-state">
      <Database :size="40" />
      <h1>Welcome to StoneQL</h1>
      <p>Open a connection to explore your databases.</p>
      <button class="button primary" @click="manager = true">Manage connections</button>
    </div>
    <footer class="bottom-status" aria-label="Application status and preferences">
      <span class="footer-session"
        ><Terminal :size="12" /><strong>StoneQL</strong><span class="footer-divider">|</span
        ><span class="session-detail"
          >{{ isLive ? `Live ${liveReadWrite ? 'read-write' : 'read-only'} · ${liveSession?.status || 'disconnected'}` : nativeHostState.kind === 'desktop' ? 'Not connected' : nativeHostState.kind === 'unavailable' ? 'Native host unavailable · Restart StoneQL to connect' : 'Mock transport · All changes are local to this session' }}</span
        ></span
      >
      <div class="footer-controls">
        <button title="Help" @click="help = true"><CircleHelp :size="13" /> Help</button>
        <button title="Settings" @click="settings = true"><Settings :size="13" /> Settings</button>
        <button
          type="button"
          class="footer-theme-toggle"
          :aria-label="dark ? 'Switch to light theme' : 'Switch to dark theme'"
          :title="dark ? 'Switch to light theme' : 'Switch to dark theme'"
          @click="store.theme = dark ? 'light' : 'dark'"
        >
          <Moon v-if="dark" :size="14" aria-hidden="true" /><Sun
            v-else
            :size="14"
            aria-hidden="true"
          />
        </button>
        <span class="footer-encoding">UTF-8</span>
      </div>
    </footer>
    <Transition name="toast"
      ><div
        v-if="toast"
        role="status"
        class="toast-message"
        :class="{ 'toast-error': toastType === 'error' }"
      >
        <AlertCircle v-if="toastType === 'error'" :size="17" /><CheckCircle2 v-else :size="17" />{{
          toast
        }}<button aria-label="Dismiss notification" @click="toast = ''">
          <X :size="13" />
        </button></div
    ></Transition>
    <div
      v-if="manager"
      class="modal-overlay"
      @mousedown.self="closeManager()"
    >
      <section
        class="modal manager-modal"
        v-dialog="{ close: closeManager }"
        role="dialog"
        aria-modal="true"
        aria-label="Connection manager"
      >
        <header>
          <div>
            <h2><Server :size="19" /> Connection manager</h2>
          </div>
          <!-- Always closable: with nothing open the start screen offers the manager again. -->
          <button
            class="icon-button"
            aria-label="Close connection manager"
            @click="closeManager()"
          >
            <X :size="17" />
          </button>
        </header>
        <div class="modal-body">
          <div v-if="demo" class="notice info">
            <CircleHelp :size="15" />Prototype mode. Connections are simulated and credentials stay
            in memory.
          </div>
          <div class="workspace-toolbar manager-toolbar">
            <div class="filter-field">
              <Search :size="14" /><input
                v-model="connectionSearch"
                aria-label="Filter connections"
                placeholder="Filter connections…"
              />
            </div>
            <button class="button" @click="beginGroup()"><Plus :size="13" /> New group</button>
          </div>
          <form v-if="groupEditing !== null" class="group-editor" @submit.prevent="saveGroup">
            <input
              v-model="groupName"
              aria-label="Group name"
              placeholder="Project name"
              required
            /><button class="button" type="button" @click="groupEditing = null">Cancel</button
            ><button class="button primary">Save group</button>
          </form>
          <div v-if="groupError || store.storageError" class="notice error" role="alert">
            {{ groupError || store.storageError }}
          </div>
          <section
            v-for="(group, groupIndex) in managerGroups"
            :key="group.id"
            class="connection-group"
          >
            <div class="group-heading">
              <button
                class="group-toggle"
                :aria-expanded="!group.collapsed || !!connectionSearch"
                @click="
                  group.id
                    ? (group.collapsed = !group.collapsed)
                    : (ungroupedCollapsed = !ungroupedCollapsed)
                "
              >
                <ChevronRight v-if="group.collapsed && !connectionSearch" :size="13" /><ChevronDown
                  v-else
                  :size="13"
                />{{ group.name
                }}<span class="muted">{{ groupedConnections(group.id).length }}</span></button
              ><template v-if="group.id"
                ><button
                  class="icon-button"
                  :aria-label="`Move group ${group.name} up`"
                  :disabled="groupIndex === 0"
                  @click="store.reorderGroup(group.id, -1)"
                >
                  <ArrowUp :size="13" /></button
                ><button
                  class="icon-button"
                  :aria-label="`Move group ${group.name} down`"
                  :disabled="groupIndex === store.groups.length - 1"
                  @click="store.reorderGroup(group.id, 1)"
                >
                  <ArrowDown :size="13" /></button
                ><button
                  class="icon-button"
                  :aria-label="`Rename group ${group.name}`"
                  @click="beginGroup(group.id)"
                >
                  <Pencil :size="13" /></button
                ><button
                  class="icon-button delete-link"
                  :aria-label="`Delete group ${group.name}`"
                  @click="deleteGroup(group.id)"
                >
                  <Trash2 :size="13" /></button
              ></template>
            </div>
            <template v-if="!group.collapsed || connectionSearch"
              ><div
                v-for="(c, index) in groupedConnections(group.id)"
                :key="c.id"
                class="connection-card connection-row"
                :style="{ '--connection-color': c.color }"
              >
                <span class="dot" :style="{ background: c.color }"></span>
                <div class="connection-summary">
                  <strong>{{ c.name }}</strong
                  ><span
                    class="environment-tag"
                    :class="{ prod: c.environment === 'Production' }"
                    >{{ environmentLabel(c) }}</span
                  >
                  <p>
                    {{ c.engine }} · {{ c.user }}@{{ c.host }}:{{ c.port }} ·
                    {{ c.transport.type === 'ssh' ? 'SSH' : 'Direct' }} · {{ c.mode==='live' ? 'Live read-only' : 'Mock' }}
                  </p>
                </div>
                <select
                  :value="c.groupId || ''"
                  :aria-label="`Move ${c.name} to group`"
                  @change="store.moveConnection(c.id, ($event.target as HTMLSelectElement).value)"
                >
                  <option value="">Ungrouped</option>
                  <option v-for="g in store.groups" :key="g.id" :value="g.id">
                    {{ g.name }}
                  </option></select
                ><button
                  class="icon-button"
                  :aria-label="`Move ${c.name} up`"
                  :disabled="index === 0"
                  @click="store.reorderConnection(c.id, -1)"
                >
                  <ArrowUp :size="12" /></button
                ><button
                  class="icon-button"
                  :aria-label="`Move ${c.name} down`"
                  :disabled="index === groupedConnections(group.id).length - 1"
                  @click="store.reorderConnection(c.id, 1)"
                >
                  <ArrowDown :size="12" /></button
                ><button
                  class="icon-button"
                  :aria-label="`Edit ${c.name}`"
                  @click="newConnection(c)"
                >
                  <Pencil :size="14" /></button
                ><button
                  class="icon-button delete-link"
                  :aria-label="`Remove ${c.name}`"
                  @click="removeConnection(c.id)"
                >
                  <Trash2 :size="14" /></button
                ><button
                  class="button"
                  @click="
                    switchConnection(c.id);
                    manager = false;
                  "
                >
                  Open <ArrowRight :size="13" />
                </button>
              </div>
              <p v-if="!groupedConnections(group.id).length" class="group-empty muted">
                {{
                  connectionSearch ? 'No matching connections.' : 'No connections in this group.'
                }}
              </p></template
            >
          </section>
          <div v-if="!store.connections.length" class="empty-state">
            No connections yet. Add your first connection below.
          </div>
        </div>
        <footer>
          <span class="muted">MySQL · MariaDB · PostgreSQL · SQLite</span
          ><button class="button primary" @click="newConnection()">
            <Plus :size="14" /> New connection
          </button>
        </footer>
      </section>
    </div>
    <div
      v-if="connectionDialog && connectionDraft"
      class="modal-overlay upper"
      @mousedown.self="closeConnectionDialog()"
    >
      <section
        class="modal setup-modal"
        v-dialog="{ close: closeConnectionDialog }"
        role="dialog"
        aria-modal="true"
        aria-label="Connection setup"
      >
        <header>
          <div>
            <h2>{{ connectionEditing ? 'Edit connection' : 'New connection' }}</h2>
          </div>
          <button
            class="icon-button"
            aria-label="Close connection setup"
            :disabled="testing"
            @click="closeConnectionDialog()"
          >
            <X :size="17" />
          </button>
        </header>
        <form @submit.prevent="saveConnection()">
          <div class="modal-body">
            <label v-if="demo" class="form-label">Connection mode<select :disabled="testing" v-model="connectionDraft.mode" aria-label="Connection mode"><option value="mock">Mock (session simulation)</option><option value="live" :disabled="nativeHostState.kind!=='desktop'">Live (desktop · Direct/SSH server or SQLite file)</option></select></label>
            <div v-if="connectionDraft.mode==='live'" class="form-label" data-testid="access-option"><label class="compact-choice"><input type="checkbox" :disabled="testing || !readWriteAvailable || !readWriteRoute(connectionDraft)" :checked="!readWriteAvailable || !readWriteRoute(connectionDraft) || connectionDraft.readOnly === true" @change="connectionDraft.readOnly = ($event.target as HTMLInputElement).checked ? true : undefined" /> Read-only connection</label><span class="muted">{{ !readWriteAvailable ? 'This build runs live connections read-only.' : !readWriteRoute(connectionDraft) ? 'PostgreSQL over an SSH tunnel is read-only for now.' : connectionDraft.readOnly ? 'Engine-enforced read-only SELECT subset. Saved with this connection.' : `Read-write: scripts, transactions and row edits can change data.${connectionDraft.environment==='Production' ? ' PROD asks for confirmation when connecting.' : ''}` }}</span></div>
            <div v-if="tlsOptional(connectionDraft)" class="form-label" data-testid="tls-option">
              <label class="compact-choice"><input type="checkbox" :disabled="testing" :checked="usesTls(connectionDraft)" @change="setTls(connectionDraft, ($event.target as HTMLInputElement).checked)" /> Use TLS</label>
              <span :class="usesTls(connectionDraft) || isLoopbackHost(connectionDraft.host) ? 'muted' : 'tls-warning'">{{ tlsNote(connectionDraft) }}</span>
            </div>
            <div v-if="tlsOptional(connectionDraft) && !usesTls(connectionDraft)" class="form-label" role="radiogroup" aria-labelledby="rsa-option-title" data-testid="rsa-option">
              <span id="rsa-option-title">Password without TLS</span>
              <span class="muted">Used when the server asks for a full sign-in (MySQL caching_sha2_password, for example after it restarts). RSA encryption protects only the password; queries and results are still sent unencrypted.</span>
              <label class="compact-choice"><input type="radio" name="password-rsa" :disabled="testing" :checked="passwordRsa(connectionDraft)==='none'" @change="setPasswordRsa(connectionDraft, 'none')" /> Don’t send it without TLS</label>
              <label class="compact-choice"><input type="radio" name="password-rsa" :disabled="testing" :checked="passwordRsa(connectionDraft)==='trusted'" @change="setPasswordRsa(connectionDraft, 'trusted')" /> Encrypt it with the server’s public key (trusted)</label>
              <template v-if="passwordRsa(connectionDraft)==='trusted'">
                <textarea :disabled="testing" rows="4" spellcheck="false" placeholder="-----BEGIN PUBLIC KEY-----" aria-label="Server RSA public key" :value="connectionDraft.serverPublicKeyPem" @input="connectionDraft.serverPublicKeyPem = ($event.target as HTMLTextAreaElement).value" @change="connectionDraft.serverPublicKeyPem = normalizePublicKey(($event.target as HTMLTextAreaElement).value)"></textarea>
                <span class="muted">Paste the server’s public_key.pem from its data directory, or ask its administrator. StoneQL never asks the server for it. Saved with this connection; it is not secret.</span>
              </template>
              <label class="compact-choice"><input type="radio" name="password-rsa" :disabled="testing" :checked="passwordRsa(connectionDraft)==='retrieve'" @change="setPasswordRsa(connectionDraft, 'retrieve')" /> Fetch the key from the server without verifying it</label>
              <span v-if="passwordRsa(connectionDraft)==='retrieve'" class="tls-warning">The key is not verified: anyone who can impersonate this server on the network can send their own key and read your password. Prefer TLS or a trusted key.</span>
            </div>
            <label v-if="connectionDraft.mode==='live' && connectionDraft.engine!=='SQLite' && usesTls(connectionDraft)" class="form-label">Trusted CA certificate (optional, session only)<input :disabled="testing" type="file" accept=".pem,.crt,.cer" aria-label="Trusted CA certificate" @change="readCertificate"/><span class="muted">{{ connectionDraft.rootCertificatePem ? 'CA certificate loaded for this session.' : 'System trust roots. With TLS on, the certificate and host name are always verified.' }}</span><button v-if="connectionDraft.rootCertificatePem" type="button" class="button" @click="connectionDraft.rootCertificatePem=undefined">Use system trust roots</button></label>
            <div class="form-two">
              <label class="form-label"
                >Connection name<input :disabled="testing"
                  v-model="connectionDraft.name"
                  placeholder="e.g. Local development"
                  required /></label
              ><label class="form-label"
                >Database engine<select :disabled="testing"
                  v-model="connectionDraft.engine"
                  aria-label="Database engine"
                  @change="
                    connectionDraft.port =
                      connectionDraft.engine === 'PostgreSQL'
                        ? 5432
                        : connectionDraft.engine === 'SQLite'
                          ? 0
                          : 3306;
                    if (connectionDraft.mode === 'live' && connectionDraft.engine === 'SQLite') changeTransport('direct')
                  "
                >
                  <option>MySQL</option>
                  <option>MariaDB</option>
                  <option>PostgreSQL</option>
                  <option>SQLite</option>
                </select></label
              >
            </div>
            <div class="form-two">
              <label class="form-label"
                >Environment<select :disabled="testing"
                  v-model="connectionDraft.environment"
                  aria-label="Connection environment"
                >
                  <option v-for="environment in environments" :key="environment">
                    {{ environment }}
                  </option>
                </select></label
              ><label class="form-label"
                >Group<select :disabled="testing" v-model="connectionDraft.groupId" aria-label="Connection group">
                  <option value="">Ungrouped</option>
                  <option v-for="group in store.groups" :key="group.id" :value="group.id">
                    {{ group.name }}
                  </option>
                </select></label
              >
            </div>
            <label v-if="connectionDraft.environment === 'Custom'" class="form-label"
              >Custom environment<input :disabled="testing"
                v-model="connectionDraft.customEnvironment"
                aria-label="Custom environment"
                maxlength="40"
                required
            /></label>
            <fieldset class="connection-color-picker" :disabled="testing">
              <legend>Connection color</legend>
              <button
                v-for="color in connectionColors"
                :key="color.value"
                class="color-swatch"
                type="button"
                :style="{ '--swatch-color': color.value }"
                :aria-label="`${color.name} connection color`"
                :aria-pressed="connectionDraft.color === color.value"
                :title="color.name"
                @click="connectionDraft.color = color.value"
              >
                <Check v-if="connectionDraft.color === color.value" :size="13" />
              </button>
            </fieldset>
            <div class="transport-tabs">
              <button
                type="button"
                :class="{ active: connectionDraft.transport.type === 'direct' }"
                @click="changeTransport('direct')"
              >
                <Server :size="13" /> Direct</button
              ><button
                type="button"
                :class="{ active: connectionDraft.transport.type === 'ssh' }"
                :disabled="testing || (connectionDraft.mode==='live' && connectionDraft.engine==='SQLite')"
                :title="connectionDraft.mode==='live' && connectionDraft.engine==='SQLite' ? 'SQLite files have no network tunnel' : undefined"
                @click="changeTransport('ssh')"
              >
                <KeyRound :size="13" /> SSH Tunnel</button
              ><span>Teleport / AWS SSM · coming later</span>
            </div>
            <template v-if="connectionDraft.engine !== 'SQLite'"
              ><div class="host-port">
                <label class="form-label"
                  >Host<input :disabled="testing" v-model="connectionDraft.host" v-bind="exactText" required /></label
                ><label class="form-label"
                  >Port<input :disabled="testing"
                    v-model.number="connectionDraft.port"
                    type="number"
                    min="1"
                    max="65535"
                    required
                /></label>
              </div>
              <div class="form-two">
                <label class="form-label"
                  >Username<input :disabled="testing"
                    v-model="connectionDraft.user" v-bind="exactText"
                    autocomplete="off"
                    required /></label
                ><label class="form-label"
                  >Password<input :disabled="testing"
                    v-model="connectionDraft.password"
                    type="password"
                    autocomplete="new-password"
                    :placeholder="connectionDraft.mode==='live' && connectionDraft.passwordRef ? savedPlaceholder(connectionDraft.passwordRef, secretStoreKind) : 'Optional'"
                /></label></div>
              <label v-if="connectionDraft.mode==='live'" class="checkbox-label"
                ><input :disabled="testing" type="checkbox" v-model="connectionDraft.rememberPassword" /> Remember database password</label
              ><p v-if="connectionDraft.mode==='live' && secretStoreKind" class="muted secret-store-note" data-testid="secret-store">{{ secretStoreNote(secretStoreKind) }}</p
              ></template
            ><label v-if="connectionDraft.mode!=='live' || connectionDraft.engine!=='SQLite'" class="form-label"
              >{{
                connectionDraft.engine === 'SQLite'
                  ? 'Database file path (simulated)'
                  : 'Default database'
              }}<input :disabled="testing"
                v-model="connectionDraft.database" v-bind="exactText"
                :placeholder="
                  connectionDraft.engine === 'SQLite' ? '/path/to/database.sqlite' : demo ? 'acme_store' : 'Type a name, or test the connection to choose one'
                "
            /><select v-if="databaseOptions.length && connectionDraft.engine !== 'SQLite'" :disabled="testing" aria-label="Databases on this server" :value="databaseOptions.includes(connectionDraft.database) ? connectionDraft.database : ''" @change="connectionDraft.database = ($event.target as HTMLSelectElement).value"><option value="" disabled>Choose a database on this server ({{ databaseOptions.length }})</option><option v-for="name in databaseOptions" :key="name" :value="name">{{ name }}</option></select></label>
            <div v-if="connectionDraft.mode==='live' && connectionDraft.engine==='SQLite'" class="form-label">SQLite file<p v-if="!sqliteAvailable" class="notice info" data-testid="sqlite-unavailable">This build does not include SQLite (excluded from release builds). Use a mock profile or a development build.</p><div class="sqlite-file"><button type="button" class="button" :disabled="testing || !sqliteAvailable" @click="chooseSqliteFile">Choose file…</button><span :class="{ muted: !connectionDraft.sqliteGrant }">{{ connectionDraft.sqliteGrant ? connectionDraft.database : 'No file chosen for this app session' }}</span></div><span class="muted">The native picker grants this file only. Access is not saved; choose it again after restarting.</span></div>
            <div v-if="connectionDraft.transport.type === 'ssh'" class="ssh-fields">
              <h3>SSH tunnel configuration</h3>
              <div class="host-port">
                <label class="form-label"
                  >SSH host<input :disabled="testing"
                    v-model="connectionDraft.transport.host" v-bind="exactText"
                    required
                    placeholder="bastion.example.com" /></label
                ><label class="form-label"
                  >SSH port<input :disabled="testing"
                    v-model.number="connectionDraft.transport.port"
                    type="number"
                    min="1"
                    max="65535"
                    required
                /></label>
              </div>
              <div class="form-two">
                <label class="form-label"
                  >SSH username<input :disabled="testing" v-model="connectionDraft.transport.user" v-bind="exactText" required /></label
                ><label class="form-label"
                  >Authentication<select :disabled="testing" v-model="connectionDraft.transport.auth" aria-label="SSH authentication">
                    <option value="agent">SSH agent</option>
                    <option value="private-key">Private key</option>
                    <option value="password">Password</option>
                  </select></label
                >
              </div>
              <template v-if="connectionDraft.mode === 'live'">
                <label v-if="connectionDraft.transport.auth === 'password'" class="form-label"
                  >SSH password<input :disabled="testing"
                    v-model="connectionDraft.transport.password"
                    type="password"
                    autocomplete="new-password"
                    :placeholder="connectionDraft.transport.passwordRef ? savedPlaceholder(connectionDraft.transport.passwordRef, secretStoreKind) : ''" /></label
                ><label v-if="connectionDraft.transport.auth === 'password'" class="checkbox-label"
                  ><input :disabled="testing" type="checkbox" v-model="connectionDraft.transport.rememberPassword" /> Remember SSH password</label
                ><template v-if="connectionDraft.transport.auth === 'private-key'"
                  ><div class="form-label">Private key file<div class="sqlite-file"><button type="button" class="button" :disabled="testing" @click="chooseSshKeyFile">{{ picking ? 'Opening…' : 'Choose key file…' }}</button><span :class="{ muted: !connectionDraft.transport.keyGrant }">{{ connectionDraft.transport.keyGrant ? connectionDraft.transport.keyFileName : connectionDraft.transport.keyFileName ? `${connectionDraft.transport.keyFileName} · choose again this session` : 'No key file chosen for this app session' }}</span></div><span class="muted">The key is read natively at connect time and never enters this window. Ed25519/ECDSA keys; RSA is not supported yet.</span></div
                  ><label class="form-label"
                    >Key passphrase<input :disabled="testing"
                      v-model="connectionDraft.transport.passphrase"
                      type="password"
                      autocomplete="new-password"
                      :placeholder="connectionDraft.transport.passphraseRef ? savedPlaceholder(connectionDraft.transport.passphraseRef, secretStoreKind) : 'If the key is encrypted'" /></label
                  ><label class="checkbox-label"
                    ><input :disabled="testing" type="checkbox" v-model="connectionDraft.transport.rememberPassphrase" /> Remember key passphrase</label
                ></template>
                <p v-if="connectionDraft.transport.auth === 'agent'" class="muted">
                  Uses the SSH agent available to StoneQL (SSH_AUTH_SOCK). Agent forwarding is never requested.
                </p>
                <div class="form-label">Host key<div class="sqlite-file"><button v-if="!hostKeyTrust" type="button" class="button" :disabled="testing || hostKeyTrust === undefined" @click="fetchHostKey">Fetch host key</button><button v-else type="button" class="button" :disabled="testing" @click="forgetHostKey">Forget host key</button><span :class="{ muted: !hostKeyTrust }" data-testid="host-key-trust">{{ hostKeyTrust ? `Trusted · ${hostKeyTrust.fingerprint}` : hostKeyTrust === undefined ? 'Checking trust…' : 'Not enrolled · connections are refused until you verify and trust the key' }}</span></div></div>
                <div v-if="pendingHostKey" class="notice host-key-check" role="group" aria-label="Verify SSH host key">
                  <div>
                    <strong>Verify this host key before trusting it</strong>
                    <span>{{ pendingHostKey.algorithm }} · <code>{{ pendingHostKey.fingerprint }}</code></span>
                    <span class="muted">Compare it with the fingerprint from the server administrator (for example <code>ssh-keygen -lf</code> on the server). Never trust a key you cannot verify.</span>
                  </div>
                  <div class="host-key-actions"><button type="button" class="button" @click="rejectHostKey">Do not trust</button><button type="button" class="button primary" @click="trustHostKey">Trust this host key</button></div>
                </div>
              </template>
              <template v-else>
              <label v-if="connectionDraft.transport.auth === 'password'" class="form-label"
                >SSH password<input :disabled="testing"
                  v-model="connectionDraft.transport.password"
                  type="password" /></label
              ><template v-if="connectionDraft.transport.auth === 'private-key'"
                ><label class="form-label"
                  >Private key (memory only)<textarea :disabled="testing"
                    v-model="connectionDraft.transport.privateKey"
                    rows="3"
                    placeholder="Paste a test key; never use production credentials"
                  ></textarea></label
                ><label class="form-label"
                  >Passphrase (optional)<input :disabled="testing"
                    v-model="connectionDraft.transport.passphrase"
                    type="password" /></label
              ></template>
              <p v-if="connectionDraft.transport.auth === 'agent'" class="muted">
                Simulates using the local SSH agent. No agent or tunnel is accessed.
              </p>
              </template>
            </div>
            <div v-if="testMessage" class="notice" :class="testOk ? 'success' : 'error'">
              <CheckCircle2 v-if="testOk" :size="14" /><AlertCircle v-else :size="14" />{{
                testMessage
              }}
            </div>
            <div v-if="connectionError" class="notice error">{{ connectionError }}</div>
          </div>
          <footer>
            <button type="button" class="button" :disabled="testing" @click="testConnection">
              <LoaderCircle v-if="testing" :size="13" class="spin" /><CheckCircle2
                v-else
                :size="13"
              />
              Test connection
            </button>
            <div>
              <button type="button" class="button" :disabled="testing && !attemptId" @click="attemptId ? cancelAttempt() : closeConnectionDialog()">{{ attemptId ? 'Cancel connecting' : 'Cancel' }}</button
              ><button type="button" class="button" :disabled="testing" @click="saveConnection(false)">Save</button><button class="button primary" :disabled="testing">Save & connect</button>
            </div>
          </footer>
        </form>
      </section>
    </div>
    <MasterPasswordDialog
      v-if="masterPassword"
      :key="masterPassword.mode"
      :mode="masterPassword.mode"
      :busy="masterPassword.busy"
      :error="masterPassword.error"
      @submit="submitMasterPassword"
      @cancel="masterPassword.done(false)"
      @reset="resetMasterPassword"
    />
    <LiveDdlDialog
      v-if="liveDdl"
      :key="liveDdl.mode + (liveDdl.column ?? '')"
      :mode="liveDdl.mode"
      :engine="store.connection.engine"
      :table="table?.name"
      :columns="table?.columns.map((c) => c.name)"
      :column="liveDdl.column"
      :definition="liveDdl.definition"
      :pg="liveDdl.pg"
      :busy="liveDdlBusy"
      :error="liveDdl.error"
      @close="liveDdl = null"
      @apply="(sql, destructive) => applyLiveDdl(sql, destructive)"
    />
    <div v-if="settings" class="modal-overlay" @mousedown.self="settings = false">
      <section
        class="modal settings-modal"
        v-dialog="{ close: () => (settings = false) }"
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
      >
        <header>
          <div>
            <h2><Settings :size="18" /> Settings</h2>
          </div>
          <button class="icon-button" aria-label="Close settings" @click="settings = false">
            <X :size="17" />
          </button>
        </header>
        <div class="modal-body">
          <label class="form-label"
            >Default rows per page<select v-model.number="store.defaultPageSize">
              <option :value="15">15</option>
              <option :value="25">25</option>
              <option :value="50">50</option>
              <option :value="100">100</option>
              <option :value="250">250</option>
              <option :value="500">500</option>
            </select></label
          ><label class="checkbox-label"
            ><input type="checkbox" v-model="store.shortcuts" /> Enable keyboard shortcuts</label
          >
          <div class="shortcut-list">
            <div><span>Run SQL query</span><kbd>⌘ / Ctrl + Enter</kbd></div>
            <div><span>Open settings</span><kbd>⌘ / Ctrl + ,</kbd></div>
            <div><span>Dismiss dialog / cell edit</span><kbd>Esc</kbd></div>
            <div><span>Edit a cell</span><kbd>Double-click</kbd></div>
          </div>
        </div>
        <footer>
          <span class="muted">StoneQL 0.1.1</span
          ><button class="button primary" @click="settings = false">Done</button>
        </footer>
      </section>
    </div>
    <div v-if="confirmation" class="modal-overlay highest">
      <section
        class="modal confirm-modal"
        v-dialog="{ close: cancelConfirmation }"
        role="alertdialog"
        aria-modal="true"
        :aria-label="confirmation.title"
      >
        <header>
          <h2><AlertCircle :size="19" class="danger-text" />{{ confirmation.title }}</h2>
          <button
            class="icon-button"
            aria-label="Cancel confirmation"
            @click="cancelConfirmation()"
          >
            <X :size="16" />
          </button>
        </header>
        <div class="modal-body">
          <p>{{ confirmation.message }}</p>
          <div v-if="confirmationError" class="notice error" role="alert">
            {{ confirmationError }}
          </div>
          <p v-if="confirmation.kind !== 'discard' && confirmation.kind !== 'notice'" class="muted">
            This action cannot be undone in this session.
          </p>
          <label v-if="confirmation.productionName" class="checkbox-label production-confirm"
            ><input type="checkbox" v-model="productionAcknowledged" /> PROD — I understand this
            affects “{{ confirmation.productionName }}”.</label
          >
        </div>
        <footer>
          <button class="button" data-initial-focus @click="cancelConfirmation()">Cancel</button
          ><button
            class="button danger solid"
            :disabled="!!confirmation.productionName && !productionAcknowledged"
            @click="runConfirmation()"
          >
            {{ confirmation.acceptLabel || 'Confirm' }}
          </button>
        </footer>
      </section>
    </div>
    <div v-if="columnDialog" class="modal-overlay">
      <section
        class="modal column-dialog"
        v-dialog="{ close: closeColumnDialog }"
        role="dialog"
        aria-modal="true"
        :aria-label="columnOriginal ? 'Edit column' : 'Add column'"
      >
        <header>
          <h2>{{ columnOriginal ? 'Edit column' : 'Add column' }}</h2>
          <button
            type="button"
            class="icon-button"
            aria-label="Close column editor"
            @click="closeColumnDialog()"
          >
            <X :size="16" />
          </button>
        </header>
        <form @submit.prevent="saveColumn">
          <div class="modal-body">
            <div class="form-two">
              <label class="form-label"
                >Column name<input
                  v-model="columnDraft.name"
                  aria-label="Column name"
                  autofocus
                  required
              /></label>
              <label class="form-label"
                >Data type<select
                  v-model="columnDraft.dataType"
                  aria-label="Data type"
                  @change="changeColumnType"
                >
                  <option v-for="type in columnCap.types" :key="type.name" :value="type.name">
                    {{ type.name }}
                  </option>
                </select></label
              >
            </div>
            <div v-if="columnSpec?.parameters === 'length'" class="form-two">
              <label class="form-label"
                >Length<input
                  v-model.number="columnDraft.length"
                  aria-label="Length"
                  type="number"
                  min="1"
                  :max="columnCap.maxLength"
                  required
              /></label>
              <label class="form-label"
                >Collation<select v-model="columnDraft.collation" aria-label="Collation">
                  <option value="">Database default</option>
                  <option v-for="value in columnCap.collations" :key="value">{{ value }}</option>
                </select></label
              >
            </div>
            <div v-else-if="columnSpec?.parameters === 'decimal'" class="form-two">
              <label class="form-label"
                >Precision<input
                  v-model.number="columnDraft.precision"
                  aria-label="Precision"
                  type="number"
                  min="1"
                  :max="columnCap.maxPrecision"
                  required
              /></label>
              <label class="form-label"
                >Scale<input
                  v-model.number="columnDraft.scale"
                  aria-label="Scale"
                  type="number"
                  min="0"
                  :max="Math.min(columnDraft.precision, columnCap.maxScale)"
                  required
              /></label>
            </div>
            <label v-else-if="columnSpec?.parameters === 'values'" class="form-label"
              >Values<textarea
                v-model="columnDraft.values"
                aria-label="Values"
                rows="2"
                placeholder="'pending','active','archived'"
                required
              ></textarea
              ><small class="muted"
                >Comma-separated values; quote values containing commas and escape quotes as
                ''.</small
              ></label
            >
            <label v-if="columnSpec?.text && columnSpec.parameters !== 'length'" class="form-label"
              >Collation<select v-model="columnDraft.collation" aria-label="Collation">
                <option value="">Database default</option>
                <option v-for="value in columnCap.collations" :key="value">{{ value }}</option>
              </select></label
            >
            <div class="form-two">
              <label class="form-label"
                >Key<select v-model="columnDraft.key" aria-label="Column key">
                  <option value="none">None</option>
                  <option value="primary">Primary key</option>
                  <option value="unique">Unique key</option>
                  <option value="index">Normal index</option>
                </select></label
              >
              <label class="form-label"
                >Default type<select v-model="columnDraft.defaultKind" aria-label="Default type">
                  <option value="none">None</option>
                  <option value="null">NULL</option>
                  <option value="custom">Custom value</option>
                  <option v-if="columnExpressions.length" value="expression">Expression</option>
                </select></label
              >
            </div>
            <label v-if="columnDraft.defaultKind === 'custom'" class="form-label"
              >Default value<input
                v-model="columnDraft.defaultValue"
                aria-label="Default value"
                placeholder="An empty string is allowed for text"
            /></label>
            <label v-else-if="columnDraft.defaultKind === 'expression'" class="form-label"
              >Expression<select v-model="columnDraft.expression" aria-label="Default expression">
                <option v-for="expression in columnExpressions" :key="expression">
                  {{ expression }}
                </option>
              </select></label
            >
            <p
              v-if="columnDraft.enumValues && columnDraft.enumName === columnDraft.dataType"
              class="capability-note"
            >
              Native ENUM values: {{ columnDraft.enumValues.join(', ') }}. Defined by the database
              type.
            </p>
            <div class="column-attributes">
              <label class="checkbox-label"
                ><input type="checkbox" v-model="columnDraft.nullable" /> Allow NULL</label
              >
              <label v-if="columnCap.unsigned && columnSpec?.numeric" class="checkbox-label"
                ><input type="checkbox" v-model="columnDraft.unsigned" /> UNSIGNED</label
              >
              <label v-if="columnSpec?.integer" class="checkbox-label"
                ><input type="checkbox" v-model="columnDraft.autoIncrement" />
                {{ columnCap.autoLabel }}</label
              >
            </div>
            <div v-if="columnCap.positioning" class="form-two">
              <label class="form-label"
                >Position<select v-model="columnDraft.position" aria-label="Column position">
                  <option v-if="columnOriginal" value="keep">Keep current position</option>
                  <option value="first">FIRST</option>
                  <option value="after">AFTER</option>
                  <option value="end">AT END</option>
                </select></label
              >
              <label v-if="columnDraft.position === 'after'" class="form-label"
                >After column<select v-model="columnDraft.after" aria-label="After column" required>
                  <option value="" disabled>Choose column</option>
                  <option
                    v-for="column in table?.columns.filter((c) => c.name !== columnOriginal)"
                    :key="column.name"
                    :value="column.name"
                  >
                    {{ column.name }}
                  </option>
                </select></label
              >
            </div>
            <p v-else class="capability-note">
              Physical column positioning is unavailable for this engine. Existing columns keep
              their position; new columns append.
            </p>
            <label v-if="columnCap.comments" class="form-label"
              >Comment<input v-model="columnDraft.comment" aria-label="Column comment"
            /></label>
            <div class="column-definition-preview">
              <code>{{ columnPreview }}</code
              ><span class="muted">{{ store.connection.engine }} · simulated schema only</span>
            </div>
            <div v-if="columnError" class="notice error" role="alert">
              <AlertCircle :size="15" />{{ columnError }}
            </div>
          </div>
          <footer>
            <button class="button" type="button" @click="closeColumnDialog()">Cancel</button
            ><button class="button primary" type="submit">Save column</button>
          </footer>
        </form>
      </section>
    </div>
    <div v-if="help" class="modal-overlay" @mousedown.self="help = false">
      <section
        class="modal settings-modal"
        v-dialog="{ close: () => (help = false) }"
        role="dialog"
        aria-modal="true"
        aria-label="About StoneQL"
      >
        <header>
          <h2><img src="/icon.png" width="26" height="26" alt="" /> StoneQL</h2>
          <button class="icon-button" aria-label="Close help" @click="help = false">
            <X :size="16" />
          </button>
        </header>
        <div class="modal-body help-body">
          <h3>StoneAge SQL Admin</h3>
          <p>Old habits. New engine.</p>
          <p>
            {{ demo ? 'Formerly dbAdmin. A compact database client with session-only mock data by default.' : 'Formerly dbAdmin. A compact client for MySQL, MariaDB and PostgreSQL.' }}
          </p>
          <p>
            Select a table to browse records. Double-click a cell to edit, or use Edit, Copy, and
            Delete on any row. Changes appear in every view within the connection.
          </p>
          <h3>SQL playground</h3>
          <p>Supports SELECT, column lists, equality WHERE conditions, ORDER BY, and LIMIT. Try:</p>
          <code>SELECT * FROM users WHERE status = 'active' ORDER BY id DESC LIMIT 25;</code>
          <p>
            Connections have independent datasets. Refreshing the browser resets session data. No
            mock credentials or database connections leave your browser. Explicit desktop Live profiles make real verified-TLS, read-only connections.
          </p>
        </div>
        <footer>
          <span class="muted">Version 0.1.1 · Original StoneQL assets</span
          ><button class="button primary" @click="help = false">Got it</button>
        </footer>
      </section>
    </div>
  </div>
</template>
