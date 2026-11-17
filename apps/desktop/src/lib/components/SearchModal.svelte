<script lang="ts">
  import { tick } from 'svelte';
  import { searchNotes } from '../api';
  import { sanitizeSearchSnippet } from '../markdown';
  import type { SearchResult } from '../types';

  interface Props {
    open: boolean;
    onClose: () => void;
    onSelectResult: (relativePath: string) => void;
  }

  let { open, onClose, onSelectResult }: Props = $props();

  let query = $state('');
  let results = $state<SearchResult[]>([]);
  let loading = $state(false);
  let elapsedMs = $state<number | null>(null);
  let selectedIdx = $state(0);
  let errorMsg = $state<string | null>(null);
  let inputEl = $state<HTMLInputElement | null>(null);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (open) {
      selectedIdx = 0;
      tick().then(() => {
        inputEl?.focus();
        inputEl?.select();
      });
    }
  });

  $effect(() => {
    const q = query;
    if (!open) return;

    if (debounceTimer) clearTimeout(debounceTimer);
    if (!q.trim()) {
      results = [];
      elapsedMs = null;
      errorMsg = null;
      return;
    }

    debounceTimer = setTimeout(async () => {
      loading = true;
      errorMsg = null;
      const t0 = performance.now();
      try {
        results = await searchNotes(q, 30);
        elapsedMs = Math.round(performance.now() - t0);
        selectedIdx = 0;
      } catch (err) {
        errorMsg = String(err);
      } finally {
        loading = false;
      }
    }, 110);

    return () => {
      if (debounceTimer) clearTimeout(debounceTimer);
    };
  });

  function handleKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      if (results.length > 0) {
        selectedIdx = (selectedIdx + 1) % results.length;
      }
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      if (results.length > 0) {
        selectedIdx = (selectedIdx - 1 + results.length) % results.length;
      }
      return;
    }
    if (event.key === 'Enter' && results[selectedIdx]) {
      event.preventDefault();
      choose(results[selectedIdx].path);
    }
  }

  function choose(relativePath: string) {
    onSelectResult(relativePath);
    onClose();
  }
</script>

{#if open}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div
      class="search-dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Búsqueda rápida FTS5"
      tabindex="-1"
      onkeydown={handleKeyDown}
    >
      <div class="search-input-row">
        <span class="search-icon">⌕</span>
        <input
          bind:this={inputEl}
          type="search"
          bind:value={query}
          placeholder="Buscar texto, títulos o conceptos (FTS5)..."
          aria-label="Consulta de búsqueda"
        />
        <span class="kbd-hint">ESC</span>
      </div>

      <div class="search-meta">
        {#if loading}
          <span>Buscando en índice SQLite FTS5...</span>
        {:else if errorMsg}
          <span class="error-text">{errorMsg}</span>
        {:else if query.trim()}
          <span>
            {results.length} resultado(s)
            {#if elapsedMs !== null}
              · {elapsedMs} ms
            {/if}
          </span>
        {:else}
          <span>Usa ↑ / ↓ para navegar, Enter para abrir</span>
        {/if}
      </div>

      <div class="results-list">
        {#if query.trim() && !loading && results.length === 0 && !errorMsg}
          <p class="empty-results">No se encontraron notas para "{query}".</p>
        {:else}
          {#each results as item, i (item.path)}
            <button
              type="button"
              class="result-item"
              class:selected={i === selectedIdx}
              onclick={() => choose(item.path)}
              onmouseenter={() => (selectedIdx = i)}
            >
              <div class="result-top">
                <strong>{item.title}</strong>
                <span class="result-path">{item.path}</span>
              </div>
              {#if item.snippet}
                <p class="result-snippet">
                  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                  {@html sanitizeSearchSnippet(item.snippet)}
                </p>
              {/if}
              {#if item.tags.length > 0}
                <div class="result-tags">
                  {#each item.tags as tag}
                    <span>#{tag}</span>
                  {/each}
                </div>
              {/if}
            </button>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--bg-backdrop);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 64px 16px 24px;
  }

  .search-dialog {
    width: min(680px, 100%);
    max-height: min(76vh, 620px);
    display: flex;
    flex-direction: column;
    background: var(--bg-modal);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }

  .search-input-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .search-icon {
    font-size: 18px;
    color: var(--text-muted);
  }

  .search-input-row input {
    flex: 1;
    border: none;
    background: transparent;
    font-size: 15px;
    outline: none;
  }

  .kbd-hint {
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--border-subtle);
    color: var(--text-muted);
  }

  .search-meta {
    padding: 6px 14px;
    font-size: 11.5px;
    color: var(--text-muted);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border-subtle);
  }

  .error-text {
    color: var(--danger);
  }

  .results-list {
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .result-item {
    width: 100%;
    text-align: left;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    padding: 10px 12px;
  }

  .result-item:hover,
  .result-item.selected {
    background: var(--bg-surface-hover);
    border-color: var(--accent);
  }

  .result-top {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
  }

  .result-top strong {
    font-size: 14px;
  }

  .result-path {
    font-size: 11.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .result-snippet {
    margin: 5px 0 0;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .result-tags {
    margin-top: 6px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    font-size: 11px;
    color: var(--accent);
  }

  .empty-results {
    margin: 24px 12px;
    text-align: center;
    color: var(--text-muted);
  }
</style>
