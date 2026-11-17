<script lang="ts">
  import type {
    CanvasSummary,
    FileTreeNode,
    NoteSummary,
    VaultSnapshot,
  } from '../types';
  import FileTreeNodeItem from './FileTreeNodeItem.svelte';

  interface Props {
    snapshot: VaultSnapshot | null;
    activePath: string | null;
    activeCanvasPath?: string | null;
    activeTag: string | null;
    tagFilteredNotes: NoteSummary[];
    onSelectNote: (relativePath: string) => void;
    onSelectCanvas?: (relativePath: string) => void;
    onCreateNote: () => void;
    onCreateFolder?: () => void;
    onCreateCanvas?: () => void;
    onMoveItem?: (sourcePath: string, targetFolderPath: string) => void;
    onRefreshVault: () => void;
    onRequestRename: (relativePath: string) => void;
    onRequestDelete: (relativePath: string) => void;
    onRequestDeleteFolder?: (folderPath: string) => void;
    onRequestRenameCanvas?: (relativePath: string) => void;
    onRequestDeleteCanvas?: (relativePath: string) => void;
    onSelectTag: (tag: string | null) => void;
  }

  let {
    snapshot,
    activePath,
    activeCanvasPath = null,
    activeTag,
    tagFilteredNotes,
    onSelectNote,
    onSelectCanvas,
    onCreateNote,
    onCreateFolder,
    onCreateCanvas,
    onMoveItem,
    onRefreshVault,
    onRequestRename,
    onRequestDelete,
    onRequestDeleteFolder,
    onRequestRenameCanvas,
    onRequestDeleteCanvas,
    onSelectTag,
  }: Props = $props();

  let quickFilter = $state('');
  let tagsExpanded = $state(true);
  let canvasesExpanded = $state(true);
  let isRootDragOver = $state(false);

  let filteredNotes = $derived.by(() => {
    if (!snapshot) return [];
    const q = quickFilter.trim().toLowerCase();
    if (!q) return [];
    return snapshot.notes.filter(
      (n) =>
        n.title.toLowerCase().includes(q) || n.relative_path.toLowerCase().includes(q),
    );
  });

  let filteredCanvases = $derived.by<CanvasSummary[]>(() => {
    if (!snapshot) return [];
    const q = quickFilter.trim().toLowerCase();
    if (!q) return [];
    return snapshot.canvases.filter(
      (c) =>
        c.title.toLowerCase().includes(q) || c.relative_path.toLowerCase().includes(q),
    );
  });

  function treeCount(nodes: FileTreeNode[]): number {
    return nodes.length;
  }
</script>

<aside class="left-sidebar" aria-label="Explorador de bóveda">
  <div class="sidebar-header">
    <div class="vault-title">
      <span class="vault-badge">BÓVEDA</span>
      <strong>{snapshot?.info.name ?? 'Sin bóveda'}</strong>
    </div>
    <div class="header-actions">
      <button
        type="button"
        class="icon-btn primary"
        onclick={onCreateNote}
        disabled={!snapshot}
        title="Crear nueva nota (Ctrl+N)"
      >
        + Nota
      </button>
      {#if onCreateFolder}
        <button
          type="button"
          class="icon-btn"
          onclick={onCreateFolder}
          disabled={!snapshot}
          title="Crear nueva carpeta en la bóveda"
        >
          + Carpeta
        </button>
      {/if}
      <button
        type="button"
        class="icon-btn"
        onclick={onRefreshVault}
        disabled={!snapshot}
        title="Reindexar bóveda"
      >
        ↻
      </button>
    </div>
  </div>

  <div class="filter-box">
    <input
      type="search"
      bind:value={quickFilter}
      placeholder="Filtrar archivos..."
      aria-label="Filtrar notas en el árbol"
    />
  </div>

  <div class="sidebar-scroll">
    {#if activeTag}
      <div class="tag-filter-banner">
        <span>Filtrado por <strong>#{activeTag}</strong></span>
        <button type="button" class="clear-tag-btn" onclick={() => onSelectTag(null)}>
          Limpiar
        </button>
      </div>
      {#if tagFilteredNotes.length === 0}
        <p class="empty-hint">No hay notas con esta etiqueta.</p>
      {:else}
        <ul class="flat-list">
          {#each tagFilteredNotes as note (note.relative_path)}
            <li>
              <button
                type="button"
                class="flat-note-btn"
                class:active={activePath === note.relative_path}
                onclick={() => onSelectNote(note.relative_path)}
              >
                <span>◇</span>
                <span>{note.title}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if quickFilter.trim()}
      <div class="section-caption">Coincidencias ({filteredNotes.length + filteredCanvases.length})</div>
      {#if filteredNotes.length === 0 && filteredCanvases.length === 0}
        <p class="empty-hint">Sin archivos coincidentes.</p>
      {:else}
        <ul class="flat-list">
          {#each filteredNotes as note (note.relative_path)}
            <li>
              <button
                type="button"
                class="flat-note-btn"
                class:active={activePath === note.relative_path}
                onclick={() => onSelectNote(note.relative_path)}
                title={note.relative_path}
              >
                <span>◇</span>
                <span>{note.title}</span>
              </button>
            </li>
          {/each}
          {#each filteredCanvases as item (item.relative_path)}
            <li>
              <button
                type="button"
                class="flat-note-btn"
                class:active={activeCanvasPath === item.relative_path}
                onclick={() => onSelectCanvas?.(item.relative_path)}
                title={item.relative_path}
              >
                <span>⊞</span>
                <span>{item.title}.canvas</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if snapshot}
      <div
        class="section-caption root-dropzone"
        class:root-drag-over={isRootDragOver}
        role="region"
        aria-label="Zona para soltar archivos en la raíz"
        ondragover={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
          isRootDragOver = true;
        }}
        ondragleave={(e) => {
          if (e.currentTarget && !e.currentTarget.contains(e.relatedTarget as Node)) {
            isRootDragOver = false;
          }
        }}
        ondrop={(e) => {
          e.preventDefault();
          e.stopPropagation();
          isRootDragOver = false;
          const globalSrc = typeof window !== 'undefined' ? (window as any).__romsidian_drag_src : null;
          const src = e.dataTransfer?.getData('text/plain') || globalSrc;
          if (typeof window !== 'undefined') {
            (window as any).__romsidian_drag_src = null;
          }
          if (src) onMoveItem?.(src, '');
        }}
      >
        Archivos ({snapshot.notes.length} notas · {treeCount(snapshot.tree)} raíz)
      </div>
      <div class="tree-list">
        {#each snapshot.tree as node (node.relative_path)}
          <FileTreeNodeItem
            {node}
            {activePath}
            parentFolderPath=""
            {onSelectNote}
            {onRequestRename}
            {onRequestDelete}
            {onRequestDeleteFolder}
            {onMoveItem}
          />
        {/each}
      </div>

      <div class="canvases-section">
        <div class="canvases-header-row">
          <button
            type="button"
            class="tags-toggle"
            onclick={() => (canvasesExpanded = !canvasesExpanded)}
          >
            <span>{canvasesExpanded ? '▾' : '▸'} Lienzos (.canvas)</span>
            <span class="muted-count">{snapshot.canvases.length}</span>
          </button>
          {#if onCreateCanvas}
            <button
              type="button"
              class="mini-add-btn"
              onclick={onCreateCanvas}
              title="Crear nuevo lienzo visual (.canvas)"
            >
              + Lienzo
            </button>
          {/if}
        </div>

        {#if canvasesExpanded}
          {#if snapshot.canvases.length === 0}
            <p class="empty-hint">Sin lienzos `.canvas` todavía.</p>
          {:else}
            <ul class="flat-list">
              {#each snapshot.canvases as item (item.relative_path)}
                <li
                  class="canvas-list-row"
                  draggable="true"
                  ondragstart={(e) => {
                    e.stopPropagation();
                    e.dataTransfer?.setData('text/plain', item.relative_path);
                    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
                  }}
                >
                  <button
                    type="button"
                    class="flat-note-btn canvas-item-btn"
                    class:active={activeCanvasPath === item.relative_path}
                    onclick={() => onSelectCanvas?.(item.relative_path)}
                    title={item.relative_path}
                  >
                    <span>⊞</span>
                    <span class="canvas-item-label">{item.title}</span>
                  </button>
                  {#if onRequestRenameCanvas}
                    <button
                      type="button"
                      class="row-action-btn"
                      onclick={() => onRequestRenameCanvas(item.relative_path)}
                      title="Renombrar lienzo"
                      aria-label={`Renombrar ${item.title}`}
                    >
                      ✎
                    </button>
                  {/if}
                  {#if onRequestDeleteCanvas}
                    <button
                      type="button"
                      class="row-action-btn danger"
                      onclick={() => onRequestDeleteCanvas(item.relative_path)}
                      title="Eliminar lienzo"
                      aria-label={`Eliminar ${item.title}`}
                    >
                      ✕
                    </button>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        {/if}
      </div>
    {:else}
      <p class="empty-hint">Abre una carpeta para explorar tus notas Markdown.</p>
    {/if}

    {#if snapshot}
      <div class="tags-section">
        <button
          type="button"
          class="tags-toggle"
          onclick={() => (tagsExpanded = !tagsExpanded)}
        >
          <span>{tagsExpanded ? '▾' : '▸'} Etiquetas</span>
          <span class="muted-count">{snapshot.tags.length}</span>
        </button>

        {#if tagsExpanded}
          {#if snapshot.tags.length === 0}
            <p class="empty-hint">Sin etiquetas todavía.</p>
          {:else}
            <div class="tag-cloud">
              {#each snapshot.tags as item (item.tag)}
                <button
                  type="button"
                  class="tag-pill"
                  class:active={activeTag === item.tag}
                  onclick={() =>
                    onSelectTag(activeTag === item.tag ? null : item.tag)}
                >
                  <span>#{item.tag}</span>
                  <span class="tag-count">{item.count}</span>
                </button>
              {/each}
            </div>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</aside>

<style>
  .left-sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border-subtle);
  }

  .sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 12px 12px 8px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .vault-title {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .vault-badge {
    font-size: 10px;
    letter-spacing: 0.08em;
    color: var(--text-muted);
    font-weight: 700;
  }

  .vault-title strong {
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .header-actions {
    display: flex;
    gap: 5px;
    flex-shrink: 0;
  }

  .filter-box {
    padding: 8px 10px;
  }

  .filter-box input {
    width: 100%;
    padding: 6px 10px;
    border-radius: 7px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    font-size: 12.5px;
  }

  .filter-box input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .sidebar-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 8px 16px;
  }

  .section-caption {
    padding: 4px 6px 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
    font-weight: 600;
    transition: background 0.15s ease;
  }

  .section-caption.root-drag-over {
    background: var(--accent-soft);
    color: var(--accent);
    outline: 2px dashed var(--accent);
    border-radius: 6px;
  }

  .tree-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .flat-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .flat-note-btn {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 6px 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-secondary);
    text-align: left;
    font-size: 13px;
  }

  .flat-note-btn:hover {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
  }

  .flat-note-btn.active {
    background: var(--bg-active);
    color: var(--text-primary);
    font-weight: 600;
  }

  .tag-filter-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 7px 9px;
    margin-bottom: 8px;
    border-radius: 7px;
    background: var(--accent-soft);
    font-size: 12px;
  }

  .clear-tag-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    font-weight: 600;
    font-size: 12px;
  }

  .tags-section {
    margin-top: 14px;
    padding-top: 10px;
    border-top: 1px solid var(--border-subtle);
  }

  .tags-toggle {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 6px 8px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 600;
  }

  .muted-count {
    font-weight: 500;
  }

  .tag-cloud {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    padding: 0 4px;
  }

  .tag-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 8px;
    border-radius: 999px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: 12px;
  }

  .tag-pill:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .tag-pill.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .tag-count {
    font-size: 11px;
    color: var(--text-muted);
  }

  .empty-hint {
    padding: 8px;
    margin: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }

  .canvases-section {
    margin-top: 14px;
    padding-top: 10px;
    border-top: 1px solid var(--border-subtle);
  }

  .canvases-header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }

  .canvases-header-row .tags-toggle {
    flex: 1;
  }

  .mini-add-btn {
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-secondary);
    border-radius: 5px;
    padding: 2px 7px;
    font-size: 11px;
    cursor: pointer;
    margin-bottom: 6px;
  }

  .mini-add-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .canvas-list-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .canvas-item-btn {
    flex: 1;
    min-width: 0;
  }

  .canvas-item-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row-action-btn {
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 11.5px;
    padding: 4px 5px;
    border-radius: 4px;
    cursor: pointer;
    opacity: 0;
  }

  .canvas-list-row:hover .row-action-btn {
    opacity: 1;
  }

  .row-action-btn:hover {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
  }

  .row-action-btn.danger:hover {
    color: #ef4444;
  }
</style>
