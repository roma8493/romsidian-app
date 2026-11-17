<script lang="ts">
  import type { FileTreeNode } from '../types';
  import FileTreeNodeItem from './FileTreeNodeItem.svelte';

  interface Props {
    node: FileTreeNode;
    activePath: string | null;
    depth?: number;
    parentFolderPath?: string;
    onSelectNote: (relativePath: string) => void;
    onRequestRename: (relativePath: string) => void;
    onRequestDelete: (relativePath: string) => void;
    onRequestDeleteFolder?: (folderPath: string) => void;
    onMoveItem?: (sourcePath: string, targetFolderPath: string) => void;
  }

  let {
    node,
    activePath,
    depth = 0,
    parentFolderPath = '',
    onSelectNote,
    onRequestRename,
    onRequestDelete,
    onRequestDeleteFolder,
    onMoveItem,
  }: Props = $props();

  let expanded = $state(true);
  let isDragOver = $state(false);
</script>

{#if node.kind === 'folder'}
  <div class="tree-folder">
    <div
      role="button"
      tabindex="0"
      class="tree-row folder-row"
      class:drag-over={isDragOver}
      style="padding-left: {10 + depth * 14}px"
      draggable="true"
      ondragstart={(e) => {
        e.stopPropagation();
        e.dataTransfer?.setData('text/plain', node.relative_path);
        if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
        if (typeof window !== 'undefined') {
          (window as any).__romsidian_drag_src = node.relative_path;
        }
      }}
      ondragend={() => {
        if (typeof window !== 'undefined') {
          (window as any).__romsidian_drag_src = null;
        }
      }}
      ondragover={(e) => {
        e.preventDefault();
        e.stopPropagation();
        if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
        isDragOver = true;
      }}
      ondragleave={(e) => {
        if (e.currentTarget && !e.currentTarget.contains(e.relatedTarget as Node)) {
          isDragOver = false;
        }
      }}
      ondrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        isDragOver = false;
        const globalSrc = typeof window !== 'undefined' ? (window as any).__romsidian_drag_src : null;
        const src = e.dataTransfer?.getData('text/plain') || globalSrc;
        if (typeof window !== 'undefined') {
          (window as any).__romsidian_drag_src = null;
        }
        if (src && src !== node.relative_path) {
          onMoveItem?.(src, node.relative_path);
        }
      }}
      onclick={() => (expanded = !expanded)}
      onkeydown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          expanded = !expanded;
        }
      }}
    >
      <span class="chevron">{expanded ? '▾' : '▸'}</span>
      <span class="node-label folder-label">{node.name}</span>
      <span class="node-count">{node.children.length}</span>
      <div class="row-actions">
        <button
          type="button"
          class="mini-action danger"
          title="Eliminar carpeta"
          aria-label="Eliminar carpeta {node.name}"
          onclick={(e) => {
            e.stopPropagation();
            onRequestDeleteFolder?.(node.relative_path);
          }}
        >
          ✕
        </button>
      </div>
    </div>

    {#if expanded}
      <div class="tree-children">
        {#each node.children as child (child.relative_path)}
          <FileTreeNodeItem
            node={child}
            {activePath}
            depth={depth + 1}
            parentFolderPath={node.relative_path}
            {onSelectNote}
            {onRequestRename}
            {onRequestDelete}
            {onRequestDeleteFolder}
            {onMoveItem}
          />
        {/each}
      </div>
    {/if}
  </div>
{:else}
  <div
    role="listitem"
    class="tree-note-row"
    class:active={activePath === node.relative_path}
    class:drag-over={isDragOver}
    style="padding-left: {12 + depth * 14}px"
    draggable="true"
    ondragstart={(e) => {
      e.stopPropagation();
      e.dataTransfer?.setData('text/plain', node.relative_path);
      if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
      if (typeof window !== 'undefined') {
        (window as any).__romsidian_drag_src = node.relative_path;
      }
    }}
    ondragend={() => {
      if (typeof window !== 'undefined') {
        (window as any).__romsidian_drag_src = null;
      }
    }}
    ondragover={(e) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
      isDragOver = true;
    }}
    ondragleave={(e) => {
      if (e.currentTarget && !e.currentTarget.contains(e.relatedTarget as Node)) {
        isDragOver = false;
      }
    }}
    ondrop={(e) => {
      e.preventDefault();
      e.stopPropagation();
      isDragOver = false;
      const globalSrc = typeof window !== 'undefined' ? (window as any).__romsidian_drag_src : null;
      const src = e.dataTransfer?.getData('text/plain') || globalSrc;
      if (typeof window !== 'undefined') {
        (window as any).__romsidian_drag_src = null;
      }
      if (src && src !== node.relative_path) {
        onMoveItem?.(src, parentFolderPath);
      }
    }}
  >
    <button
      type="button"
      class="note-select-btn"
      onclick={() => onSelectNote(node.relative_path)}
      title={node.relative_path}
    >
      <span class="note-icon">◇</span>
      <span class="node-label">{node.title}</span>
    </button>

    <div class="row-actions">
      <button
        type="button"
        class="mini-action"
        title="Renombrar nota"
        aria-label="Renombrar {node.title}"
        onclick={(e) => {
          e.stopPropagation();
          onRequestRename(node.relative_path);
        }}
      >
        ✎
      </button>
      <button
        type="button"
        class="mini-action danger"
        title="Eliminar nota"
        aria-label="Eliminar {node.title}"
        onclick={(e) => {
          e.stopPropagation();
          onRequestDelete(node.relative_path);
        }}
      >
        ✕
      </button>
    </div>
  </div>
{/if}

<style>
  .tree-row,
  .tree-note-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    border-radius: 6px;
    min-height: 28px;
    padding-right: 6px;
    text-align: left;
    cursor: grab;
    user-select: none;
    transition: background 0.12s ease, color 0.12s ease;
  }

  .tree-row:active,
  .tree-note-row:active {
    cursor: grabbing;
  }

  .tree-row:hover,
  .tree-note-row:hover {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
  }

  .folder-row.drag-over,
  .tree-note-row.drag-over {
    outline: 2px dashed var(--accent) !important;
    background: var(--accent-soft) !important;
  }

  .tree-note-row.active {
    background: var(--bg-active);
    color: var(--text-primary);
    font-weight: 600;
  }

  .note-select-btn {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    padding: 4px 2px;
    text-align: left;
    cursor: pointer;
  }

  .chevron,
  .note-icon,
  .node-label,
  .node-count {
    pointer-events: none;
  }

  .chevron {
    width: 12px;
    font-size: 11px;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .note-icon {
    font-size: 10px;
    color: var(--accent);
    flex-shrink: 0;
  }

  .node-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .node-count {
    font-size: 11px;
    color: var(--text-muted);
  }

  .tree-row:hover .node-count,
  .tree-row:focus-within .node-count {
    display: none;
  }

  .row-actions {
    display: none;
    align-items: center;
    gap: 2px;
  }

  .tree-row:hover .row-actions,
  .tree-row:focus-within .row-actions,
  .tree-note-row:hover .row-actions,
  .tree-note-row:focus-within .row-actions,
  .tree-note-row.active .row-actions {
    display: flex;
  }

  .mini-action {
    border: none;
    background: transparent;
    color: var(--text-muted);
    padding: 2px 5px;
    border-radius: 4px;
    font-size: 11px;
    line-height: 1;
  }

  .mini-action:hover {
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .mini-action.danger:hover {
    background: var(--danger-soft);
    color: var(--danger);
  }
</style>
