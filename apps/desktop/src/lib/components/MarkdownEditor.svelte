<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { basicSetup } from 'codemirror';
  import {
    autocompletion,
    type CompletionContext,
    type CompletionResult,
  } from '@codemirror/autocomplete';
  import { defaultKeymap, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { markdown } from '@codemirror/lang-markdown';
  import { Compartment, EditorState, RangeSetBuilder } from '@codemirror/state';
  import { oneDark } from '@codemirror/theme-one-dark';
  import {
    Decoration,
    type DecorationSet,
    EditorView,
    keymap,
    ViewPlugin,
    type ViewUpdate,
  } from '@codemirror/view';
  import {
    evalDataviewInlineBatch,
    resolveTransclusion,
    runDataviewQuery,
  } from '../api';
  import {
    executeOptInDataviewJs,
    extractPreviewDirectives,
    findWikilinksInText,
    normalizeNoteKey,
    renderMarkdownPreview,
    type ParsedWikilinkMatch,
  } from '../markdown';
  import type {
    BacklinksLocation,
    DataviewQueryResult,
    DataviewValue,
    NoteDetail,
    NoteSummary,
    TransclusionResolution,
    UnlinkedMentionHit,
  } from '../types';
  import BacklinksCore from './BacklinksCore.svelte';

  type ViewMode = 'split' | 'edit' | 'preview';

  interface Props {
    note: NoteDetail | null;
    notes: NoteSummary[];
    knownNoteKeys: Set<string>;
    theme: 'light' | 'dark';
    saving: boolean;
    enableDataviewJs?: boolean;
    backlinksLocation?: BacklinksLocation;
    backlinksExcludedFiles?: string[];
    backlinksRefreshToken?: number;
    onSave: (relativePath: string, content: string) => Promise<void>;
    onOpenWikilink: (target: string, heading: string | null) => void;
    onRenameRequest: (relativePath: string) => void;
    onOpenNote?: (relativePath: string) => void;
    onLinkMention?: (hit: UnlinkedMentionHit, targetNotePath: string) => Promise<void>;
    onUpdateExcludedFiles?: (patterns: string[]) => void;
    onChangeBacklinksLocation?: (location: BacklinksLocation) => void;
  }

  let {
    note,
    notes,
    knownNoteKeys,
    theme,
    saving,
    enableDataviewJs = false,
    backlinksLocation = 'right-panel',
    backlinksExcludedFiles = [],
    backlinksRefreshToken = 0,
    onSave,
    onOpenWikilink,
    onRenameRequest,
    onOpenNote,
    onLinkMention,
    onUpdateExcludedFiles,
    onChangeBacklinksLocation,
  }: Props = $props();

  let editFooterExpanded = $state(true);

  let mode = $state<ViewMode>('split');
  let draftContent = $state('');
  let dirty = $state(false);
  let loadedPath = $state<string | null>(null);
  let cursorWikilink = $state<ParsedWikilinkMatch | null>(null);

  let resolvedTransclusions = $state<Map<string, TransclusionResolution | null>>(new Map());
  let resolvedInlineValues = $state<Map<string, DataviewValue>>(new Map());
  let resolvedDqlResults = $state<Map<string, DataviewQueryResult | { error: string }>>(new Map());
  let resolvedDvJsOutputs = $state<Map<string, string>>(new Map());
  let hydrationSeq = 0;

  let editorView: EditorView | null = null;
  let ignoreExternalDispatch = false;
  let autosaveTimer: ReturnType<typeof setTimeout> | null = null;

  const themeCompartment = new Compartment();
  const wikilinkCompartment = new Compartment();

  // Keep reactive refs accessible inside CodeMirror callbacks
  let latestKnownKeys = new Set<string>();
  let latestNoteTitles: NoteSummary[] = [];

  $effect(() => {
    latestKnownKeys = knownNoteKeys;
    latestNoteTitles = notes;
    if (editorView) {
      editorView.dispatch({
        effects: wikilinkCompartment.reconfigure(buildWikilinkExtension()),
      });
    }
  });

  $effect(() => {
    const currentTheme = theme;
    if (editorView) {
      editorView.dispatch({
        effects: themeCompartment.reconfigure(buildThemeExtension(currentTheme)),
      });
    }
  });

  $effect(() => {
    const currentNote = note;
    if (!currentNote) {
      loadedPath = null;
      draftContent = '';
      dirty = false;
      cursorWikilink = null;
      return;
    }

    if (currentNote.relative_path !== loadedPath) {
      if (autosaveTimer) {
        clearTimeout(autosaveTimer);
        autosaveTimer = null;
      }
      if (loadedPath && dirty) {
        void onSave(loadedPath, draftContent);
      }
      loadedPath = currentNote.relative_path;
      draftContent = currentNote.content;
      dirty = false;
      updateCursorWikilink(currentNote.content, 0);

      if (editorView && editorView.state.doc.toString() !== currentNote.content) {
        ignoreExternalDispatch = true;
        editorView.dispatch({
          changes: {
            from: 0,
            to: editorView.state.doc.length,
            insert: currentNote.content,
          },
          selection: { anchor: 0 },
        });
        ignoreExternalDispatch = false;
      }
    } else if (!dirty && currentNote.content !== draftContent) {
      draftContent = currentNote.content;
      updateCursorWikilink(currentNote.content, 0);
      if (editorView && editorView.state.doc.toString() !== currentNote.content) {
        ignoreExternalDispatch = true;
        editorView.dispatch({
          changes: {
            from: 0,
            to: editorView.state.doc.length,
            insert: currentNote.content,
          },
        });
        ignoreExternalDispatch = false;
      }
    }
  });

  $effect(() => {
    const currentNote = note;
    const activeMode = mode;
    const content = draftContent;
    const dvJsOn = enableDataviewJs;
    const _noteCatalogVersion = notes.reduce((acc, n) => acc + n.modified_unix_secs + 1, 0);

    if (!currentNote || activeMode === 'edit') {
      return;
    }

    const directives = extractPreviewDirectives(content);
    const hasWork =
      directives.transclusions.length > 0 ||
      directives.inlineExprs.length > 0 ||
      directives.dqlQueries.length > 0 ||
      (dvJsOn && directives.dataviewJsSnippets.length > 0);

    if (!hasWork) {
      if (resolvedTransclusions.size > 0) resolvedTransclusions = new Map();
      if (resolvedInlineValues.size > 0) resolvedInlineValues = new Map();
      if (resolvedDqlResults.size > 0) resolvedDqlResults = new Map();
      if (resolvedDvJsOutputs.size > 0) resolvedDvJsOutputs = new Map();
      return;
    }

    const seq = ++hydrationSeq;
    const timer = setTimeout(async () => {
      const nextTransclusions = new Map<string, TransclusionResolution | null>();
      const nextInline = new Map<string, DataviewValue>();
      const nextDql = new Map<string, DataviewQueryResult | { error: string }>();
      const nextDvJs = new Map<string, string>();

      const transclusionPromises = directives.transclusions.map(async (item) => {
        try {
          const res = await resolveTransclusion(item.target, item.subtarget);
          nextTransclusions.set(item.key, res);
          if (res?.content) {
            const nested = extractPreviewDirectives(res.content);
            await Promise.all(
              nested.transclusions.map(async (sub) => {
                if (!nextTransclusions.has(sub.key)) {
                  try {
                    nextTransclusions.set(
                      sub.key,
                      await resolveTransclusion(sub.target, sub.subtarget),
                    );
                  } catch {
                    nextTransclusions.set(sub.key, null);
                  }
                }
              }),
            );
          }
        } catch {
          nextTransclusions.set(item.key, null);
        }
      });

      const inlinePromise =
        directives.inlineExprs.length > 0
          ? evalDataviewInlineBatch(currentNote.relative_path, directives.inlineExprs)
              .then((list) => {
                for (const entry of list) {
                  nextInline.set(entry.expression, entry.value);
                }
              })
              .catch(() => {})
          : Promise.resolve();

      const dqlPromises = directives.dqlQueries.map(async (query) => {
        try {
          const res = await runDataviewQuery(query);
          nextDql.set(query, res);
        } catch (err) {
          nextDql.set(query, { error: String(err) });
        }
      });

      const dvJsPromise =
        dvJsOn && directives.dataviewJsSnippets.length > 0
          ? runDataviewQuery('TABLE file.tags, title, estado, prioridad FROM ""')
              .then((pagesResult) => {
                for (const snippet of directives.dataviewJsSnippets) {
                  nextDvJs.set(
                    snippet,
                    executeOptInDataviewJs(snippet, pagesResult, knownNoteKeys),
                  );
                }
              })
              .catch((err) => {
                for (const snippet of directives.dataviewJsSnippets) {
                  nextDvJs.set(
                    snippet,
                    `<div class="md-dv-card error"><div class="md-dv-header"><span>DataviewJS · Error</span></div><p class="md-dv-error-msg">${String(
                      err,
                    )}</p></div>`,
                  );
                }
              })
          : Promise.resolve();

      await Promise.all([...transclusionPromises, inlinePromise, ...dqlPromises, dvJsPromise]);
      if (seq !== hydrationSeq) return;

      resolvedTransclusions = nextTransclusions;
      resolvedInlineValues = nextInline;
      resolvedDqlResults = nextDql;
      resolvedDvJsOutputs = nextDvJs;
    }, 120);

    return () => clearTimeout(timer);
  });

  let renderedPreviewHtml = $derived.by(() => {
    if (!note || mode === 'edit') return '';
    return renderMarkdownPreview(draftContent, knownNoteKeys, {
      transclusions: resolvedTransclusions,
      inlineValues: resolvedInlineValues,
      dqlResults: resolvedDqlResults,
      dataviewJsEnabled: enableDataviewJs,
      dataviewJsOutputs: resolvedDvJsOutputs,
      visitedPaths: new Set([note.relative_path]),
      depth: 0,
    });
  });

  function buildThemeExtension(activeTheme: 'light' | 'dark') {
    if (activeTheme === 'dark') {
      return [oneDark];
    }
    return [
      EditorView.theme(
        {
          '&': {
            backgroundColor: 'var(--bg-editor)',
            color: 'var(--text-primary)',
          },
          '.cm-gutters': {
            backgroundColor: 'var(--bg-editor)',
            color: 'var(--text-muted)',
            borderRight: '1px solid var(--border-subtle)',
          },
          '.cm-activeLine': {
            backgroundColor: 'rgba(76, 95, 215, 0.06)',
          },
          '.cm-activeLineGutter': {
            backgroundColor: 'rgba(76, 95, 215, 0.08)',
          },
        },
        { dark: false },
      ),
    ];
  }

  function buildWikilinkDecorations(docText: string): DecorationSet {
    const builder = new RangeSetBuilder<Decoration>();
    for (const match of findWikilinksInText(docText)) {
      const exists = latestKnownKeys.has(normalizeNoteKey(match.target));
      builder.add(
        match.from,
        match.to,
        Decoration.mark({
          class: exists ? 'cm-wikilink-resolved' : 'cm-wikilink-pending',
          attributes: {
            'data-cm-wikilink': match.target,
            title: exists
              ? `Ctrl+Clic para abrir "${match.target}"`
              : `Ctrl+Clic para crear "${match.target}.md"`,
          },
        }),
      );
    }
    return builder.finish();
  }

  function wikilinkCompletionSource(
    context: CompletionContext,
  ): CompletionResult | null {
    const before = context.matchBefore(/\[\[[^\]\n]*/);
    if (!before) return null;
    const typed = before.text.slice(2).toLowerCase();
    const options = latestNoteTitles
      .filter((item) => item.title.toLowerCase().includes(typed))
      .slice(0, 25)
      .map((item) => ({
        label: item.title,
        detail: item.relative_path,
        apply: `${item.title}]]`,
      }));
    return {
      from: before.from + 2,
      options,
      validFor: /^[^\]\n]*$/,
    };
  }

  function buildWikilinkExtension() {
    const plugin = ViewPlugin.fromClass(
      class {
        decorations: DecorationSet;
        constructor(view: EditorView) {
          this.decorations = buildWikilinkDecorations(view.state.doc.toString());
        }
        update(update: ViewUpdate) {
          if (update.docChanged) {
            this.decorations = buildWikilinkDecorations(update.state.doc.toString());
          }
        }
      },
      {
        decorations: (v) => v.decorations,
      },
    );

    return [
      plugin,
      autocompletion({
        override: [wikilinkCompletionSource],
        activateOnTyping: true,
      }),
    ];
  }

  function updateCursorWikilink(text: string, pos: number) {
    const matches = findWikilinksInText(text);
    cursorWikilink = matches.find((m) => pos >= m.from && pos <= m.to) ?? null;
  }

  function scheduleAutosave(relativePath: string, content: string) {
    if (autosaveTimer) clearTimeout(autosaveTimer);
    autosaveTimer = setTimeout(async () => {
      autosaveTimer = null;
      await flushSave(relativePath, content);
    }, 650);
  }

  async function flushSave(relativePath: string, content: string) {
    if (!dirty && note?.content === content) return;
    await onSave(relativePath, content);
    if (loadedPath === relativePath && draftContent === content) {
      dirty = false;
    }
  }

  async function handleManualSave() {
    if (!note) return;
    if (autosaveTimer) {
      clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
    await flushSave(note.relative_path, draftContent);
  }

  function delegateWikilinkClicks(node: HTMLElement) {
    function handleClick(event: MouseEvent) {
      const target = event.target as HTMLElement | null;
      const button = target?.closest<HTMLElement>('[data-wikilink]');
      if (!button) return;
      const linkTarget = button.dataset.wikilink;
      const heading = button.dataset.heading || null;
      if (linkTarget) {
        onOpenWikilink(linkTarget, heading);
      }
    }

    node.addEventListener('click', handleClick);
    return {
      destroy() {
        node.removeEventListener('click', handleClick);
      },
    };
  }

  function mountCodeMirror(hostEl: HTMLDivElement) {
    const initialDoc = note?.content ?? draftContent;
    if (note && loadedPath !== note.relative_path) {
      loadedPath = note.relative_path;
      draftContent = initialDoc;
      dirty = false;
      updateCursorWikilink(initialDoc, 0);
    }

    const saveKeymap = keymap.of([
      {
        key: 'Mod-s',
        preventDefault: true,
        run: () => {
          void handleManualSave();
          return true;
        },
      },
      indentWithTab,
      ...defaultKeymap,
      ...historyKeymap,
    ]);

    const clickHandlers = EditorView.domEventHandlers({
      mousedown: (event, view) => {
        if (!event.ctrlKey && !event.metaKey) return false;
        const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
        if (pos === null) return false;
        const matches = findWikilinksInText(view.state.doc.toString());
        const hit = matches.find((m) => pos >= m.from && pos <= m.to);
        if (hit) {
          event.preventDefault();
          onOpenWikilink(hit.target, hit.heading);
          return true;
        }
        return false;
      },
    });

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged && !ignoreExternalDispatch && loadedPath) {
        const nextText = update.state.doc.toString();
        draftContent = nextText;
        dirty = true;
        scheduleAutosave(loadedPath, nextText);
      }
      if (update.selectionSet || update.docChanged) {
        const anchor = update.state.selection.main.head;
        updateCursorWikilink(update.state.doc.toString(), anchor);
      }
    });

    const state = EditorState.create({
      doc: initialDoc,
      extensions: [
        basicSetup,
        markdown(),
        EditorView.lineWrapping,
        saveKeymap,
        clickHandlers,
        updateListener,
        themeCompartment.of(buildThemeExtension(theme)),
        wikilinkCompartment.of(buildWikilinkExtension()),
      ],
    });

    const view = new EditorView({
      state,
      parent: hostEl,
    });
    editorView = view;

    return {
      destroy() {
        view.destroy();
        if (editorView === view) {
          editorView = null;
        }
      },
    };
  }

  export async function setViewMode(nextMode: ViewMode) {
    await switchMode(nextMode);
  }

  export async function insertText(snippet: string) {
    if (!note || !snippet) return;
    if (editorView) {
      const sel = editorView.state.selection.main;
      editorView.dispatch({
        changes: { from: sel.from, to: sel.to, insert: snippet },
        selection: { anchor: sel.from + snippet.length },
      });
      editorView.focus();
      return;
    }
    const base = draftContent;
    const sep = base.length === 0 || base.endsWith('\n') ? '' : '\n\n';
    const next = `${base}${sep}${snippet}`;
    draftContent = next;
    dirty = true;
    await flushSave(note.relative_path, next);
  }

  async function switchMode(nextMode: ViewMode) {
    mode = nextMode;
    if (nextMode !== 'preview') {
      await tick();
      editorView?.focus();
    }
  }

  onDestroy(() => {
    if (autosaveTimer) clearTimeout(autosaveTimer);
    editorView?.destroy();
    editorView = null;
  });
</script>

<section class="editor-workspace" aria-label="Editor de notas">
  {#if note}
    <header class="editor-toolbar">
      <div class="note-breadcrumb">
        <button
          type="button"
          class="note-title-btn"
          onclick={() => onRenameRequest(note.relative_path)}
          title="Clic para renombrar {note.relative_path}"
        >
          {note.title}
        </button>
        <span class="note-relpath">{note.relative_path}</span>
      </div>

      <div class="toolbar-right">
        {#if cursorWikilink}
          {@const resolved = knownNoteKeys.has(normalizeNoteKey(cursorWikilink.target))}
          <button
            type="button"
            class="wikilink-pill"
            class:pending={!resolved}
            onclick={() =>
              cursorWikilink &&
              onOpenWikilink(cursorWikilink.target, cursorWikilink.heading)}
            title="También puedes usar Ctrl+Clic sobre el enlace"
          >
            {resolved ? '↗ Abrir' : '+ Crear'} [[{cursorWikilink.target}]]
          </button>
        {/if}

        <span class="save-badge" class:dirty>
          {#if saving}
            Guardando...
          {:else if dirty}
            • Sin guardar
          {:else}
            ✓ Guardado
          {/if}
        </span>

        <div class="mode-group" role="group" aria-label="Modo de visualización">
          <button
            type="button"
            class="mode-btn"
            class:active={mode === 'edit'}
            onclick={() => void switchMode('edit')}
          >
            Editar
          </button>
          <button
            type="button"
            class="mode-btn"
            class:active={mode === 'split'}
            onclick={() => void switchMode('split')}
          >
            Dividido
          </button>
          <button
            type="button"
            class="mode-btn"
            class:active={mode === 'preview'}
            onclick={() => void switchMode('preview')}
          >
            Vista previa
          </button>
        </div>

        <button
          type="button"
          class="icon-btn"
          onclick={handleManualSave}
          disabled={!dirty && !saving}
          title="Guardar ahora (Ctrl+S)"
        >
          Guardar
        </button>
      </div>
    </header>

    <div class="panes" class:split={mode === 'split'}>
      <div
        class="editor-pane-wrap"
        class:hidden={mode === 'preview'}
      >
        <div class="cm-host" use:mountCodeMirror></div>
      </div>

      {#if mode !== 'edit'}
        <div
          class="preview-pane-wrap"
          role="region"
          aria-label="Vista previa Markdown"
          use:delegateWikilinkClicks
        >
          <article class="markdown-preview">
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            {@html renderedPreviewHtml}
          </article>

          {#if backlinksLocation === 'document-footer' && onOpenNote && onLinkMention && onUpdateExcludedFiles && onChangeBacklinksLocation}
            <BacklinksCore
              {note}
              {notes}
              location={backlinksLocation}
              excludedFiles={backlinksExcludedFiles}
              embedded={true}
              refreshToken={backlinksRefreshToken}
              {onOpenNote}
              {onLinkMention}
              {onUpdateExcludedFiles}
              onChangeLocation={onChangeBacklinksLocation}
            />
          {/if}
        </div>
      {/if}
    </div>

    {#if mode === 'edit' && backlinksLocation === 'document-footer' && onOpenNote && onLinkMention && onUpdateExcludedFiles && onChangeBacklinksLocation}
      <div class="edit-footer-dock">
        <button
          type="button"
          class="edit-footer-bar"
          onclick={() => (editFooterExpanded = !editFooterExpanded)}
        >
          <span>{editFooterExpanded ? '▾' : '▸'} Retroenlaces al pie del documento ({note.backlinks.length})</span>
        </button>
        {#if editFooterExpanded}
          <div class="edit-footer-body">
            <BacklinksCore
              {note}
              {notes}
              location={backlinksLocation}
              excludedFiles={backlinksExcludedFiles}
              embedded={false}
              refreshToken={backlinksRefreshToken}
              {onOpenNote}
              {onLinkMention}
              {onUpdateExcludedFiles}
              onChangeLocation={onChangeBacklinksLocation}
            />
          </div>
        {/if}
      </div>
    {/if}
  {:else}
    <div class="empty-editor">
      <h2>Ninguna nota abierta</h2>
      <p>
        Selecciona una nota en el árbol izquierdo, crea una nueva con
        <kbd>Ctrl+N</kbd>, busca con <kbd>Ctrl+K</kbd> o abre la paleta de
        comandos con <kbd>Ctrl+P</kbd>.
      </p>
    </div>
  {/if}
</section>

<style>
  .editor-workspace {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    min-height: 0;
    background: var(--bg-editor);
  }

  .editor-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }

  .note-breadcrumb {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .note-title-btn {
    border: none;
    background: transparent;
    font-size: 15px;
    font-weight: 650;
    padding: 2px 4px;
    border-radius: 5px;
    text-align: left;
  }

  .note-title-btn:hover {
    background: var(--bg-surface-hover);
  }

  .note-relpath {
    font-size: 11.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .toolbar-right {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .wikilink-pill {
    border: 1px solid var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    border-radius: 999px;
    padding: 3px 9px;
    font-size: 12px;
    font-weight: 500;
  }

  .wikilink-pill.pending {
    border-color: var(--pending);
    background: var(--pending-soft);
    color: var(--pending);
  }

  .save-badge {
    font-size: 11.5px;
    color: var(--success);
    font-weight: 500;
  }

  .save-badge.dirty {
    color: var(--pending);
  }

  .mode-group {
    display: inline-flex;
    border: 1px solid var(--border-subtle);
    border-radius: 7px;
    overflow: hidden;
  }

  .mode-btn {
    border: none;
    background: transparent;
    padding: 4px 9px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .mode-btn:hover {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
  }

  .mode-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .panes {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 1fr;
  }

  .panes.split {
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }

  .editor-pane-wrap,
  .preview-pane-wrap {
    min-width: 0;
    min-height: 0;
    height: 100%;
    overflow: hidden;
  }

  .panes.split .editor-pane-wrap {
    border-right: 1px solid var(--border-subtle);
  }

  .hidden {
    display: none;
  }

  .empty-editor {
    margin: auto;
    max-width: 420px;
    padding: 24px;
    text-align: center;
    color: var(--text-secondary);
  }

  .empty-editor h2 {
    margin: 0 0 8px;
    color: var(--text-primary);
  }

  kbd {
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    font-size: 12px;
  }

  @media (max-width: 960px) {
    .panes.split {
      grid-template-columns: 1fr;
      grid-template-rows: 1fr 1fr;
    }

    .panes.split .editor-pane-wrap {
      border-right: none;
      border-bottom: 1px solid var(--border-subtle);
    }
  }

  .edit-footer-dock {
    border-top: 1px solid var(--border-subtle);
    background: var(--bg-sidebar);
    display: flex;
    flex-direction: column;
    max-height: 40%;
    min-height: 0;
  }

  .edit-footer-bar {
    border: none;
    background: var(--bg-surface);
    color: var(--text-secondary);
    text-align: left;
    padding: 6px 14px;
    font-size: 12px;
    font-weight: 600;
    border-bottom: 1px solid var(--border-subtle);
  }

  .edit-footer-body {
    overflow-y: auto;
    padding: 10px 14px 14px;
  }
</style>
