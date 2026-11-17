<script lang="ts">
  import type {
    BacklinksLocation,
    NoteDetail,
    NoteSummary,
    UnlinkedMentionHit,
  } from '../types';
  import { normalizeNoteKey } from '../markdown';
  import BacklinksCore from './BacklinksCore.svelte';

  interface Props {
    note: NoteDetail | null;
    pinnedNote?: NoteDetail | null;
    notes: NoteSummary[];
    knownNoteKeys: Set<string>;
    backlinksLocation: BacklinksLocation;
    excludedFiles: string[];
    isPinned?: boolean;
    refreshToken?: number;
    onOpenNote: (relativePath: string) => void;
    onOpenWikilink: (target: string, heading: string | null) => void;
    onSelectTag: (tag: string) => void;
    onLinkMention: (hit: UnlinkedMentionHit, targetNotePath: string) => Promise<void>;
    onUpdateExcludedFiles: (patterns: string[]) => void;
    onChangeLocation: (location: BacklinksLocation) => void;
    onTogglePin: () => void;
    onPinCurrentNote?: () => void;
  }

  let {
    note,
    pinnedNote = null,
    notes,
    knownNoteKeys,
    backlinksLocation,
    excludedFiles,
    isPinned = false,
    refreshToken = 0,
    onOpenNote,
    onOpenWikilink,
    onSelectTag,
    onLinkMention,
    onUpdateExcludedFiles,
    onChangeLocation,
    onTogglePin,
    onPinCurrentNote,
  }: Props = $props();

  let pinnedSubTab = $state<'backlinks' | 'inspector'>('backlinks');

  $effect(() => {
    if (backlinksLocation === 'pinned-tab') {
      pinnedSubTab = 'backlinks';
    }
  });

  let effectiveBacklinkNote = $derived(isPinned && pinnedNote ? pinnedNote : note);

  let frontmatterEntries = $derived.by(() => {
    if (!note) return [];
    return Object.entries(note.frontmatter);
  });

  function formatTimestamp(unixSecs: number): string {
    if (!unixSecs) return '—';
    try {
      return new Date(unixSecs * 1000).toLocaleString('es-AR', {
        dateStyle: 'short',
        timeStyle: 'short',
      });
    } catch {
      return String(unixSecs);
    }
  }
</script>

<aside class="right-sidebar" aria-label="Panel de backlinks y propiedades">
  {#if backlinksLocation === 'pinned-tab'}
    <div class="pinned-tab-strip" role="tablist" aria-label="Pestañas del panel lateral">
      <button
        type="button"
        role="tab"
        aria-selected={pinnedSubTab === 'backlinks'}
        class="pinned-tab-btn"
        class:active={pinnedSubTab === 'backlinks'}
        onclick={() => (pinnedSubTab = 'backlinks')}
      >
        📌 Retroenlaces
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={pinnedSubTab === 'inspector'}
        class="pinned-tab-btn"
        class:active={pinnedSubTab === 'inspector'}
        onclick={() => (pinnedSubTab = 'inspector')}
      >
        Propiedades
      </button>
    </div>
  {/if}

  {#if backlinksLocation === 'pinned-tab' && pinnedSubTab === 'backlinks'}
    {#if effectiveBacklinkNote}
      <div class="panel-scroll">
        {#if note && effectiveBacklinkNote.relative_path !== note.relative_path && onPinCurrentNote}
          <div class="pinned-banner">
            <span>Mostrando nota fijada: <strong>{effectiveBacklinkNote.title}</strong></span>
            <button type="button" class="repin-btn" onclick={onPinCurrentNote}>
              Fijar actual ({note.title})
            </button>
          </div>
        {/if}

        <section class="panel-card">
          <BacklinksCore
            note={effectiveBacklinkNote}
            {notes}
            location={backlinksLocation}
            {excludedFiles}
            {isPinned}
            activeEditorPath={note?.relative_path ?? null}
            {refreshToken}
            {onOpenNote}
            {onLinkMention}
            {onUpdateExcludedFiles}
            {onChangeLocation}
            {onTogglePin}
          />
        </section>
      </div>
    {:else}
      <div class="empty-panel">Selecciona una nota para fijar sus retroenlaces aquí.</div>
    {/if}
  {:else if note}
    <div class="panel-scroll">
      <section class="panel-card">
        <header class="card-title">
          <span>Propiedades (Frontmatter)</span>
          <span class="badge">{frontmatterEntries.length}</span>
        </header>

        <div class="meta-line">
          <span class="meta-key">Modificada</span>
          <span class="meta-val">{formatTimestamp(note.modified_unix_secs)}</span>
        </div>

        {#if frontmatterEntries.length === 0}
          <p class="empty-note">Sin campos YAML en esta nota.</p>
        {:else}
          <dl class="fm-list">
            {#each frontmatterEntries as [key, value] (key)}
              <div class="fm-row">
                <dt>{key}</dt>
                <dd>
                  {#if Array.isArray(value)}
                    <div class="fm-array">
                      {#each value as item}
                        <span class="fm-chip">{item}</span>
                      {/each}
                    </div>
                  {:else}
                    <span>{value}</span>
                  {/if}
                </dd>
              </div>
            {/each}
          </dl>
        {/if}
      </section>

      <section class="panel-card">
        <header class="card-title">
          <span>Etiquetas</span>
          <span class="badge">{note.tags.length}</span>
        </header>
        {#if note.tags.length === 0}
          <p class="empty-note">Sin etiquetas detectadas.</p>
        {:else}
          <div class="chip-wrap">
            {#each note.tags as tag (tag)}
              <button
                type="button"
                class="tag-btn"
                onclick={() => onSelectTag(tag)}
              >
                #{tag}
              </button>
            {/each}
          </div>
        {/if}
      </section>

      {#if backlinksLocation === 'document-footer'}
        <section class="panel-card">
          <header class="card-title">
            <span>Retroenlaces</span>
            <span class="badge">{note.backlinks.length}</span>
          </header>
          <p class="empty-note">
            Los retroenlaces y menciones no enlazadas se muestran incrustados al pie del documento activo.
          </p>
          <div class="footer-hint-actions">
            <button
              type="button"
              class="hint-btn"
              onclick={() => onChangeLocation('right-panel')}
            >
              Mover al panel derecho
            </button>
            <button
              type="button"
              class="hint-btn"
              onclick={() => onChangeLocation('pinned-tab')}
            >
              Abrir pestaña fijada
            </button>
          </div>
        </section>
      {:else}
        <section class="panel-card">
          <BacklinksCore
            note={effectiveBacklinkNote ?? note}
            {notes}
            location={backlinksLocation}
            {excludedFiles}
            {isPinned}
            activeEditorPath={note.relative_path}
            {refreshToken}
            {onOpenNote}
            {onLinkMention}
            {onUpdateExcludedFiles}
            {onChangeLocation}
            {onTogglePin}
          />
        </section>
      {/if}

      <section class="panel-card">
        <header class="card-title">
          <span>Enlaces salientes</span>
          <span class="badge">{note.wikilinks.length}</span>
        </header>
        {#if note.wikilinks.length === 0}
          <p class="empty-note">Sin [[wikilinks]] salientes.</p>
        {:else}
          <ul class="link-list">
            {#each note.wikilinks as link, idx (`${link.target}:${link.line}:${idx}`)}
              {@const exists = knownNoteKeys.has(normalizeNoteKey(link.target))}
              <li>
                <button
                  type="button"
                  class="hit-btn"
                  onclick={() => onOpenWikilink(link.target, link.heading)}
                >
                  <div class="hit-head">
                    <span class="out-target" class:pending={!exists}>
                      [[{link.target}{link.heading ? `#${link.heading}` : ''}]]
                    </span>
                    <span class="status-pill" class:pending={!exists}>
                      {exists ? 'Existe' : 'Crear'}
                    </span>
                  </div>
                  {#if link.alias}
                    <p class="hit-context">Alias: {link.alias}</p>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </div>
  {:else}
    <div class="empty-panel">Selecciona una nota para ver sus propiedades y backlinks.</div>
  {/if}
</aside>

<style>
  .right-sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg-sidebar);
    border-left: 1px solid var(--border-subtle);
  }

  .panel-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .panel-card {
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 9px;
    padding: 10px 11px;
  }

  .card-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 11.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
    margin-bottom: 8px;
  }

  .badge {
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--bg-surface-hover);
    color: var(--text-secondary);
    font-size: 11px;
  }

  .meta-line {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
    color: var(--text-muted);
    padding-bottom: 6px;
    margin-bottom: 6px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .fm-list {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .fm-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .fm-row dt {
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 600;
  }

  .fm-row dd {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-primary);
    word-break: break-word;
  }

  .fm-array {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .fm-chip {
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--bg-surface-hover);
    font-size: 11.5px;
  }

  .chip-wrap {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .tag-btn {
    border: 1px solid var(--border-subtle);
    background: var(--accent-soft);
    color: var(--accent);
    border-radius: 999px;
    padding: 2px 9px;
    font-size: 12px;
    font-weight: 500;
  }

  .tag-btn:hover {
    border-color: var(--accent);
  }

  .link-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .hit-btn {
    width: 100%;
    text-align: left;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    border-radius: 7px;
    padding: 7px 9px;
    transition: border-color 0.12s ease, background 0.12s ease;
  }

  .hit-btn:hover {
    border-color: var(--accent);
    background: var(--bg-surface-hover);
  }

  .hit-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    font-size: 12.5px;
  }

  .hit-context {
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--text-secondary);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .out-target {
    color: var(--accent);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .out-target.pending {
    color: var(--pending);
  }

  .status-pill {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    flex-shrink: 0;
  }

  .status-pill.pending {
    background: var(--pending-soft);
    color: var(--pending);
  }

  .empty-note,
  .empty-panel {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
  }

  .empty-panel {
    padding: 18px 14px;
  }

  .pinned-tab-strip {
    display: flex;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }

  .pinned-tab-btn {
    flex: 1;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-muted);
    padding: 8px 6px;
    font-size: 12px;
    font-weight: 600;
  }

  .pinned-tab-btn.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    background: var(--bg-editor);
  }

  .pinned-banner {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid var(--accent);
    background: var(--accent-soft);
    font-size: 11.5px;
    color: var(--text-primary);
  }

  .repin-btn,
  .hint-btn {
    align-self: flex-start;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-primary);
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 11.5px;
  }

  .repin-btn:hover,
  .hint-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .footer-hint-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 8px;
  }
</style>
