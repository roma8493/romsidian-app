<script lang="ts">
  import { tick } from 'svelte';
  import type { CommandAction } from '../types';

  interface Props {
    open: boolean;
    commands: CommandAction[];
    onClose: () => void;
  }

  let { open, commands, onClose }: Props = $props();

  let query = $state('');
  let selectedIdx = $state(0);
  let inputEl = $state<HTMLInputElement | null>(null);

  let filteredCommands = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter((cmd) => {
      const hay = `${cmd.category} ${cmd.label} ${cmd.keywords ?? ''} ${cmd.shortcut ?? ''}`.toLowerCase();
      return q.split(/\s+/).every((part) => hay.includes(part));
    });
  });

  $effect(() => {
    if (open) {
      query = '';
      selectedIdx = 0;
      tick().then(() => {
        inputEl?.focus();
      });
    }
  });

  $effect(() => {
    if (selectedIdx >= filteredCommands.length) {
      selectedIdx = Math.max(0, filteredCommands.length - 1);
    }
  });

  async function executeCommand(cmd: CommandAction | undefined) {
    if (!cmd || cmd.disabled) return;
    onClose();
    await cmd.run();
  }

  function handleKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      if (filteredCommands.length > 0) {
        selectedIdx = (selectedIdx + 1) % filteredCommands.length;
      }
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      if (filteredCommands.length > 0) {
        selectedIdx = (selectedIdx - 1 + filteredCommands.length) % filteredCommands.length;
      }
      return;
    }
    if (event.key === 'Enter' && filteredCommands[selectedIdx]) {
      event.preventDefault();
      void executeCommand(filteredCommands[selectedIdx]);
    }
  }
</script>

{#if open}
  <div
    class="palette-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div class="palette-box" role="dialog" aria-modal="true" aria-label="Paleta de comandos">
      <div class="palette-input-row">
        <span class="palette-prefix">⌘</span>
        <input
          bind:this={inputEl}
          type="text"
          bind:value={query}
          onkeydown={handleKeyDown}
          placeholder="Escribe un comando (ej. diaria, plantilla, tema, vista, git)..."
          aria-label="Filtrar comandos"
        />
        <button type="button" class="close-btn" onclick={onClose}>Esc</button>
      </div>

      <div class="palette-list" role="listbox">
        {#if filteredCommands.length === 0}
          <div class="palette-empty">No hay comandos que coincidan con "{query}".</div>
        {:else}
          {#each filteredCommands as cmd, idx (cmd.id)}
            <button
              type="button"
              class="palette-item"
              class:selected={idx === selectedIdx}
              class:disabled={cmd.disabled}
              disabled={cmd.disabled}
              onmouseenter={() => (selectedIdx = idx)}
              onclick={() => void executeCommand(cmd)}
            >
              <span class="cmd-main">
                <span class="cmd-category">{cmd.category}:</span>
                <span class="cmd-label">{cmd.label}</span>
              </span>
              {#if cmd.shortcut}
                <kbd class="cmd-shortcut">{cmd.shortcut}</kbd>
              {/if}
            </button>
          {/each}
        {/if}
      </div>

      <footer class="palette-footer">
        <span>↑/↓ navegar · Enter ejecutar · Esc cerrar</span>
        <span>{filteredCommands.length} comando/s</span>
      </footer>
    </div>
  </div>
{/if}

<style>
  .palette-backdrop {
    position: fixed;
    inset: 0;
    z-index: 65;
    background: var(--bg-backdrop);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 68px 16px 24px;
  }

  .palette-box {
    width: min(640px, 100%);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-modal);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }

  .palette-input-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .palette-prefix {
    font-size: 15px;
    color: var(--accent);
    font-weight: 700;
  }

  .palette-input-row input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text-primary);
    font-size: 14.5px;
    outline: none;
  }

  .close-btn {
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-secondary);
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 11.5px;
  }

  .palette-list {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .palette-empty {
    padding: 20px 14px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }

  .palette-item {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    text-align: left;
    border: 1px solid transparent;
    background: transparent;
    border-radius: 8px;
    padding: 9px 11px;
    color: var(--text-primary);
  }

  .palette-item:hover:not(:disabled),
  .palette-item.selected:not(:disabled) {
    background: var(--bg-active);
    border-color: var(--accent);
  }

  .palette-item.disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .cmd-main {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }

  .cmd-category {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--accent);
    white-space: nowrap;
  }

  .cmd-label {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cmd-shortcut {
    font-size: 11px;
    color: var(--text-muted);
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    white-space: nowrap;
  }

  .palette-footer {
    display: flex;
    justify-content: space-between;
    padding: 7px 14px;
    font-size: 11.5px;
    color: var(--text-muted);
    border-top: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }
</style>
