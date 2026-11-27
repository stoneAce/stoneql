<script setup lang="ts">
import { sqlDialects } from '../sql/dialects';
import type { Engine } from '../types';
import { ref, onMounted, onBeforeUnmount, watch } from 'vue';
import { EditorView, basicSetup } from 'codemirror';
import { sql } from '@codemirror/lang-sql';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { Compartment, Prec } from '@codemirror/state';
const props = defineProps<{
  modelValue: string;
  dark: boolean;
  schema: Record<string, string[]>;
  engine: Engine;
}>();
const emit = defineEmits<{ 'update:modelValue': [value: string]; execute: [] }>();
const host = ref<HTMLElement>();
const editorHeight = ref(180);
function setEditorHeight(height: number) {
  editorHeight.value = Math.round(Math.max(96, Math.min(window.innerHeight * 0.6, height)));
}
function resizeWithKeyboard(event: KeyboardEvent) {
  const separator = event.currentTarget !== host.value;
  if (!['ArrowUp', 'ArrowDown'].includes(event.key) || (!separator && (event.target !== host.value || !event.altKey)))
    return;
  event.preventDefault();
  setEditorHeight((host.value?.offsetHeight ?? editorHeight.value) + (event.key === 'ArrowDown' ? 24 : -24));
}
function resizeWithPointer(event: PointerEvent) {
  if (event.button !== 0) return;
  event.preventDefault();
  const handle = event.currentTarget as HTMLElement;
  const start = event.clientY;
  const height = host.value?.offsetHeight ?? editorHeight.value;
  const move = (moveEvent: PointerEvent) => setEditorHeight(height + moveEvent.clientY - start);
  const stop = () => {
    handle.removeEventListener('pointermove', move);
    handle.removeEventListener('pointerup', stop);
    handle.removeEventListener('pointercancel', stop);
  };
  handle.setPointerCapture(event.pointerId);
  handle.addEventListener('pointermove', move);
  handle.addEventListener('pointerup', stop);
  handle.addEventListener('pointercancel', stop);
}
let editor: EditorView | undefined;
const theme = new Compartment();
const language = new Compartment();
// CSS variables keep both SQL themes aligned with the surrounding workspace.
const highlighting = HighlightStyle.define([
  { tag: tags.keyword, color: 'var(--syntax-keyword)', fontWeight: '600' },
  { tag: [tags.string, tags.special(tags.string)], color: 'var(--syntax-string)' },
  { tag: [tags.number, tags.bool, tags.null], color: 'var(--syntax-number)' },
  { tag: [tags.typeName, tags.className], color: 'var(--syntax-type)' },
  { tag: tags.comment, color: 'var(--syntax-comment)' },
  {
    tag: [tags.variableName, tags.propertyName, tags.operator, tags.punctuation],
    color: 'var(--text)',
  },
]);
onMounted(() => {
  editor = new EditorView({
    parent: host.value,
    doc: props.modelValue,
    extensions: [
      basicSetup,
      language.of(sql({ dialect: sqlDialects[props.engine], schema: props.schema })),
      theme.of(EditorView.theme({}, { dark: props.dark })),
      syntaxHighlighting(highlighting),
      EditorView.theme({
        '&': {
          fontSize: 'var(--font-body)',
          height: '100%',
          minHeight: '0',
          backgroundColor: 'var(--bg)',
          color: 'var(--text)',
        },
        '.cm-gutters': {
          backgroundColor: 'var(--panel)',
          color: 'var(--muted)',
          borderColor: 'var(--line)',
        },
        '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--panel)' },
        // The selection is drawn behind the text, so the opaque line highlight would hide it.
        '&.cm-has-selection .cm-activeLine': { backgroundColor: 'transparent' },
        '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--accent)' },
        '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
          backgroundColor: 'var(--selection)',
        },
        '.cm-tooltip': {
          backgroundColor: 'var(--panel)',
          color: 'var(--text)',
          borderColor: 'var(--line)',
        },
        '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
          backgroundColor: 'var(--selection)',
          color: 'var(--text)',
        },
        '.cm-panels': { backgroundColor: 'var(--toolbar)', color: 'var(--text)' },
        '.cm-searchMatch': {
          backgroundColor: 'var(--accent-bg)',
          outline: '1px solid var(--hover-line)',
        },
        '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'var(--selection)' },
        '.cm-scroller': { overflow: 'auto' },
        '.cm-content': { padding: '8px 0', fontFamily: 'var(--font-mono)' },
      }),
      EditorView.editorAttributes.compute(['selection'], (state): Record<string, string> =>
        state.selection.ranges.some((range) => !range.empty) ? { class: 'cm-has-selection' } : {},
      ),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) emit('update:modelValue', update.state.doc.toString());
      }),
      // Above basicSetup's keymap, whose Mod-Enter (Ctrl on Linux/Windows, Cmd on macOS)
      // would otherwise insert a blank line instead of running the query.
      Prec.highest(
        EditorView.domEventHandlers({
          keydown: (event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
              event.preventDefault();
              emit('execute');
              return true;
            }
            return false;
          },
        }),
      ),
    ],
  });
});
watch(
  () => props.modelValue,
  (value) => {
    if (editor && value !== editor.state.doc.toString())
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: value } });
  },
);
watch(
  () => props.dark,
  (value) =>
    editor?.dispatch({ effects: theme.reconfigure(EditorView.theme({}, { dark: value })) }),
);
watch(() => [props.engine, props.schema] as const, () => editor?.dispatch({ effects: language.reconfigure(sql({ dialect: sqlDialects[props.engine], schema: props.schema })) }));
onBeforeUnmount(() => editor?.destroy());
</script>
<template>
  <div
    ref="host"
    class="sql-editor"
    :style="{ height: editorHeight + 'px' }"
    tabindex="0"
    role="group"
    aria-label="SQL editor. Resize vertically by dragging the handle below it or using Alt and Up or Down arrows."
    @keydown="resizeWithKeyboard"
  />
  <div
    class="sql-editor-resizer"
    role="separator"
    aria-orientation="horizontal"
    aria-label="Resize SQL editor"
    :aria-valuenow="editorHeight"
    tabindex="0"
    title="Drag to resize the editor"
    @pointerdown="resizeWithPointer"
    @keydown="resizeWithKeyboard"
  />
</template>
