<script lang="ts">
  import { getUnlinkedMentions } from '../api';
  import type {
    BacklinkHit,
    BacklinksLocation,
    NoteDetail,
    NoteSummary,
    UnlinkedMentionHit,
  } from '../types';

  type SectionFilter = 'all' | 'linked' | 'unlinked';
  type SortOrder = 'name-asc' | 'name-desc' | 'date-desc' | 'date-asc' | 'count-desc';

  interface Props {
    note: NoteDetail;
    notes: NoteSummary[];
    location: BacklinksLocation;
    excludedFiles: string[];
    embedded?: boolean;
    isPinned?: boolean;
    activeEditorPath?: string | null;
    refreshToken?: number;
    onOpenNote: (relativePath: string) => void;
    onLinkMention: (hit: UnlinkedMentionHit, targetNotePath: string) => Promise<void>;
    onUpdateExcludedFiles: (patterns: string[]) => void;
    onChangeLocation: (location: BacklinksLocation) => void;
    onTogglePin?: () => void;
  }

  let {
    note,
    notes,
    location,
    excludedFiles,
    embedded = false,
    isPinned = false,
    activeEditorPath = null,
    refreshToken = 0,
    onOpenNote,
    onLinkMention,
    onUpdateExcludedFiles,
    onChangeLocation,
    onTogglePin,
  }: Props = $props();

  let sectionFilter = $state<SectionFilter>('all');
  let searchQuery = $state('');
  let sortOrder = $state<SortOrder>('name-asc');
  let expandFullContext = $state(false);
  let showExcludedEditor = $state(false);
  let newExcludePattern = $state('');

  let linkedCollapsed = $state(false);
  let unlinkedCollapsed = $state(false);
  let collapsedGroups = $state<Set<string>>(new Set());

  let unlinkedMentions = $state<UnlinkedMentionHit[]>([]);
  let loadingUnlinked = $state(false);
  let unlinkedError = $state<string | null>(null);
  let linkingKey = $state<string | null>(null);
  let mentionsSeq = 0;

  let noteModifiedMap = $derived.by(() => {
    const map = new Map<string, number>();
    for (const item of notes) {
      map.set(item.relative_path, item.modified_unix_secs);
    }
    return map;
  });

  function matchesExcludePattern(path: string, patterns: string[]): boolean {
    if (patterns.length === 0) return false;
    const normalized = path.toLowerCase();
    for (const raw of patterns) {
      const pat = raw.trim().toLowerCase().replace(/^\/+/, '');
      if (!pat) continue;
      if (pat.includes('*') || pat.includes('?')) {
        const escaped = pat
          .replace(/[.+^${}()|[\]\\]/g, '\\$&')
          .replace(/\*/g, '.*')
          .replace(/\?/g, '.');
        if (new RegExp(`^${escaped}$`).test(normalized)) return true;
      } else if (pat.endsWith('/')) {
        if (normalized.startsWith(pat)) return true;
      } else if (normalized === pat || normalized.startsWith(`${pat}/`) || normalized.includes(pat)) {
        return true;
      }
    }
    return false;
  }

  async function loadUnlinked(targetPath: string, patterns: string[]) {
    const seq = ++mentionsSeq;
    loadingUnlinked = true;
    unlinkedError = null;
    try {
      const hits = await getUnlinkedMentions(targetPath, patterns);
      if (seq === mentionsSeq) {
        unlinkedMentions = hits;
      }
    } catch (err) {
      if (seq === mentionsSeq) {
        unlinkedError = String(err);
        unlinkedMentions = [];
      }
    } finally {
      if (seq === mentionsSeq) {
        loadingUnlinked = false;
      }
    }
  }

  $effect(() => {
    const targetPath = note.relative_path;
    const patterns = [...excludedFiles];
    void note.modified_unix_secs;
    void refreshToken;
    void loadUnlinked(targetPath, patterns);
  });

  interface HitGroup<T> {
    source_path: string;
    source_title: string;
    modified_unix_secs: number;
    hits: T[];
  }

  function groupAndSortHits<
    T extends { source_path: string; source_title: string; context: string; line: number },
  >(hits: T[], extraMatch?: (hit: T) => string): HitGroup<T>[] {
    const q = searchQuery.trim().toLowerCase();
    const groups = new Map<string, HitGroup<T>>();

    for (const hit of hits) {
      if (matchesExcludePattern(hit.source_path, excludedFiles)) continue;
      if (q) {
        const haystack = `${hit.source_title} ${hit.source_path} ${hit.context} ${extraMatch ? extraMatch(hit) : ''}`.toLowerCase();
        if (!haystack.includes(q)) continue;
      }
      let group = groups.get(hit.source_path);
      if (!group) {
        group = {
          source_path: hit.source_path,
          source_title: hit.source_title,
          modified_unix_secs: noteModifiedMap.get(hit.source_path) ?? 0,
          hits: [],
        };
        groups.set(hit.source_path, group);
      }
      group.hits.push(hit);
    }

    const list = Array.from(groups.values());
    list.sort((a, b) => {
      switch (sortOrder) {
        case 'name-desc':
          return b.source_title.localeCompare(a.source_title, 'es');
        case 'date-desc':
          return b.modified_unix_secs - a.modified_unix_secs || a.source_title.localeCompare(b.source_title, 'es');
        case 'date-asc':
          return a.modified_unix_secs - b.modified_unix_secs || a.source_title.localeCompare(b.source_title, 'es');
        case 'count-desc':
          return b.hits.length - a.hits.length || a.source_title.localeCompare(b.source_title, 'es');
        case 'name-asc':
        default:
          return a.source_title.localeCompare(b.source_title, 'es');
      }
    });
    return list;
  }

  let linkedGroups = $derived.by(() => groupAndSortHits<BacklinkHit>(note.backlinks));
  let unlinkedGroups = $derived.by(() =>
    groupAndSortHits<UnlinkedMentionHit>(unlinkedMentions, (h) => h.matched_term),
  );

  let visibleLinkedCount = $derived.by(() =>
    linkedGroups.reduce((acc, g) => acc + g.hits.length, 0),
  );
  let visibleUnlinkedCount = $derived.by(() =>
    unlinkedGroups.reduce((acc, g) => acc + g.hits.length, 0),
  );

  let allGroupKeys = $derived.by(() => [
    ...linkedGroups.map((g) => `linked:${g.source_path}`),
    ...unlinkedGroups.map((g) => `unlinked:${g.source_path}`),
  ]);

  let areAllCollapsed = $derived.by(
    () =>
      allGroupKeys.length > 0 && allGroupKeys.every((key) => collapsedGroups.has(key)),
  );

  function toggleGroup(key: string) {
    const next = new Set(collapsedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    collapsedGroups = next;
  }

  function toggleCollapseAllGroups() {
    if (areAllCollapsed) {
      collapsedGroups = new Set();
    } else {
      collapsedGroups = new Set(allGroupKeys);
    }
  }

  function splitContextMatch(context: string, term: string): { before: string; match: string; after: string } {
    if (!term) return { before: context, match: '', after: '' };
    const idx = context.toLowerCase().indexOf(term.toLowerCase());
    if (idx < 0) return { before: context, match: '', after: '' };
    return {
      before: context.slice(0, idx),
      match: context.slice(idx, idx + term.length),
      after: context.slice(idx + term.length),
    };
  }

  function addExcludePatterns() {
    const incoming = newExcludePattern
      .split(/[\n,;]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (incoming.length === 0) return;
    const merged = Array.from(new Set([...excludedFiles, ...incoming]));
    onUpdateExcludedFiles(merged);
    newExcludePattern = '';
  }

  function removeExcludePattern(pattern: string) {
    onUpdateExcludedFiles(excludedFiles.filter((p) => p !== pattern));
  }

  async function handleLinkClick(hit: UnlinkedMentionHit, key: string) {
    linkingKey = key;
    try {
      await onLinkMention(hit, note.relative_path);
      await loadUnlinked(note.relative_path, excludedFiles);
    } finally {
      linkingKey = null;
    }
  }

  function formatShortDate(unixSecs: number): string {
    if (!unixSecs) return '';
    try {
      return new Date(unixSecs * 1000).toLocaleDateString('es-AR', {
        day: '2-digit',
        month: '2-digit',
      });
    } catch {
      return '';
    }
  }
</script>

<div class="backlinks-core" class:embedded>
  <div class="core-head">
    <div class="core-title-row">
      <span class="core-title">
        {embedded ? 'Retroenlaces del documento' : 'Retroenlaces'}
        {#if isPinned}
          <span class="pin-badge" title="Mostrando retroenlaces fijados">📌 {note.title}</span>
        {/if}
      </span>

      <div class="location-switch" role="group" aria-label="Ubicación de retroenlaces">
        <button
          type="button"
          class="loc-btn"
          class:active={location === 'right-panel'}
          onclick={() => onChangeLocation('right-panel')}
          title="Mostrar en el panel lateral derecho"
        >
          Panel
        </button>
        <button
          type="button"
          class="loc-btn"
          class:active={location === 'document-footer'}
          onclick={() => onChangeLocation('document-footer')}
          title="Incrustar al pie del documento activo"
        >
          Pie
        </button>
        <button
          type="button"
          class="loc-btn"
          class:active={location === 'pinned-tab'}
          onclick={() => onChangeLocation('pinned-tab')}
          title="Abrir en pestaña fijada independiente"
        >
          Fijada
        </button>
        {#if onTogglePin}
          <button
            type="button"
            class="loc-btn"
            class:active={isPinned}
            onclick={onTogglePin}
            title={isPinned ? 'Seguir nota activa' : 'Fijar retroenlaces en esta nota'}
          >
            📌
          </button>
        {/if}
      </div>
    </div>

    {#if isPinned && activeEditorPath && activeEditorPath !== note.relative_path}
      <p class="pin-subnote">
        Fijado en <strong>{note.relative_path}</strong> mientras editas <code>{activeEditorPath}</code>.
      </p>
    {/if}

    <div class="section-tabs" role="tablist" aria-label="Tipo de menciones">
      <button
        type="button"
        role="tab"
        aria-selected={sectionFilter === 'all'}
        class="tab-pill"
        class:active={sectionFilter === 'all'}
        onclick={() => (sectionFilter = 'all')}
      >
        Todas ({visibleLinkedCount + visibleUnlinkedCount})
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={sectionFilter === 'linked'}
        class="tab-pill"
        class:active={sectionFilter === 'linked'}
        onclick={() => (sectionFilter = 'linked')}
      >
        Enlazadas ({visibleLinkedCount})
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={sectionFilter === 'unlinked'}
        class="tab-pill"
        class:active={sectionFilter === 'unlinked'}
        onclick={() => (sectionFilter = 'unlinked')}
      >
        No enlazadas ({visibleUnlinkedCount})
      </button>
    </div>

    <div class="controls-row">
      <div class="search-wrap">
        <input
          type="search"
          bind:value={searchQuery}
          placeholder="Filtrar menciones..."
          aria-label="Filtrar retroenlaces y menciones"
        />
        {#if searchQuery}
          <button
            type="button"
            class="clear-btn"
            onclick={() => (searchQuery = '')}
            title="Limpiar filtro"
            aria-label="Limpiar filtro"
          >
            ×
          </button>
        {/if}
      </div>

      <select bind:value={sortOrder} aria-label="Ordenar retroenlaces" class="sort-select">
        <option value="name-asc">Nombre A–Z</option>
        <option value="name-desc">Nombre Z–A</option>
        <option value="date-desc">Recientes</option>
        <option value="date-asc">Antiguas</option>
        <option value="count-desc">Más menciones</option>
      </select>
    </div>

    <div class="action-bar">
      <button
        type="button"
        class="mini-tool-btn"
        class:active={areAllCollapsed}
        onclick={toggleCollapseAllGroups}
        disabled={allGroupKeys.length === 0}
        title="Colapsar o expandir todos los archivos"
      >
        {areAllCollapsed ? '▸ Expandir notas' : '▾ Colapsar notas'}
      </button>
      <button
        type="button"
        class="mini-tool-btn"
        class:active={expandFullContext}
        onclick={() => (expandFullContext = !expandFullContext)}
        title="Alternar entre contexto compacto o completo"
      >
        {expandFullContext ? '↕ Contexto compacto' : '↕ Más contexto'}
      </button>
      <button
        type="button"
        class="mini-tool-btn"
        class:active={showExcludedEditor || excludedFiles.length > 0}
        onclick={() => (showExcludedEditor = !showExcludedEditor)}
        title="Configurar Excluded files para retroenlaces"
      >
        ⊘ Excluded ({excludedFiles.length})
      </button>
    </div>

    {#if showExcludedEditor}
      <div class="excluded-box">
        <div class="excluded-head">
          <strong>Excluded files</strong>
          <small>Prefijos o comodines (ej. <code>templates/</code>, <code>daily/*</code>)</small>
        </div>
        <form
          class="excluded-form"
          onsubmit={(e) => {
            e.preventDefault();
            addExcludePatterns();
          }}
        >
          <input
            type="text"
            bind:value={newExcludePattern}
            placeholder="templates/, archivos/*"
            aria-label="Nuevo patrón excluido"
          />
          <button type="submit" class="mini-tool-btn">Agregar</button>
        </form>
        {#if !excludedFiles.includes('templates/')}
          <button
            type="button"
            class="preset-btn"
            onclick={() => onUpdateExcludedFiles([...excludedFiles, 'templates/'])}
          >
            + Excluir carpeta templates/
          </button>
        {/if}
        {#if excludedFiles.length > 0}
          <div class="excluded-chips">
            {#each excludedFiles as pat (pat)}
              <span class="exclude-chip">
                <code>{pat}</code>
                <button
                  type="button"
                  onclick={() => removeExcludePattern(pat)}
                  aria-label={`Quitar ${pat}`}
                >
                  ×
                </button>
              </span>
            {/each}
          </div>
        {:else}
          <p class="empty-note">Sin archivos excluidos actualmente.</p>
        {/if}
      </div>
    {/if}
  </div>

  {#if sectionFilter === 'all' || sectionFilter === 'linked'}
    <section class="mention-section" aria-label="Menciones enlazadas">
      <button
        type="button"
        class="section-toggle"
        onclick={() => (linkedCollapsed = !linkedCollapsed)}
      >
        <span>{linkedCollapsed ? '▸' : '▾'} Menciones enlazadas</span>
        <span class="badge">{visibleLinkedCount}</span>
      </button>

      {#if !linkedCollapsed}
        {#if linkedGroups.length === 0}
          <p class="empty-note">
            {searchQuery
              ? 'No hay menciones enlazadas que coincidan con el filtro.'
              : 'Ninguna otra nota enlaza aquí todavía.'}
          </p>
        {:else}
          <div class="group-list">
            {#each linkedGroups as group (group.source_path)}
              {@const gKey = `linked:${group.source_path}`}
              {@const isCollapsed = collapsedGroups.has(gKey)}
              <div class="source-group">
                <div class="group-head">
                  <button
                    type="button"
                    class="group-chevron"
                    onclick={() => toggleGroup(gKey)}
                    aria-label={isCollapsed ? 'Expandir grupo' : 'Colapsar grupo'}
                  >
                    {isCollapsed ? '▸' : '▾'}
                  </button>
                  <button
                    type="button"
                    class="group-title-btn"
                    onclick={() => onOpenNote(group.source_path)}
                    title={group.source_path}
                  >
                    <strong>{group.source_title}</strong>
                    {#if group.modified_unix_secs}
                      <span class="group-date">{formatShortDate(group.modified_unix_secs)}</span>
                    {/if}
                  </button>
                  <span class="badge">{group.hits.length}</span>
                </div>

                {#if !isCollapsed}
                  <ul class="hit-list">
                    {#each group.hits as hit, idx (`${hit.source_path}:${hit.line}:${idx}`)}
                      <li>
                        <button
                          type="button"
                          class="hit-btn"
                          onclick={() => onOpenNote(hit.source_path)}
                        >
                          <div class="hit-head">
                            <span class="hit-meta">
                              {hit.is_embed ? '![[Transclusión]]' : '[[Enlace]]'}
                              {#if hit.heading}· #{hit.heading}{/if}
                            </span>
                            <span class="line-pill">L{hit.line}</span>
                          </div>
                          <p class="hit-context" class:expanded={expandFullContext}>{hit.context}</p>
                        </button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </section>
  {/if}

  {#if sectionFilter === 'all' || sectionFilter === 'unlinked'}
    <section class="mention-section" aria-label="Menciones no enlazadas">
      <button
        type="button"
        class="section-toggle"
        onclick={() => (unlinkedCollapsed = !unlinkedCollapsed)}
      >
        <span>{unlinkedCollapsed ? '▸' : '▾'} Menciones no enlazadas</span>
        <span class="badge accent">{visibleUnlinkedCount}</span>
      </button>

      {#if !unlinkedCollapsed}
        {#if loadingUnlinked}
          <p class="empty-note">Buscando menciones de texto…</p>
        {:else if unlinkedError}
          <p class="error-note">{unlinkedError}</p>
        {:else if unlinkedGroups.length === 0}
          <p class="empty-note">
            {searchQuery
              ? 'No hay menciones no enlazadas que coincidan con el filtro.'
              : 'Sin menciones no enlazadas pendientes.'}
          </p>
        {:else}
          <div class="group-list">
            {#each unlinkedGroups as group (group.source_path)}
              {@const gKey = `unlinked:${group.source_path}`}
              {@const isCollapsed = collapsedGroups.has(gKey)}
              <div class="source-group">
                <div class="group-head">
                  <button
                    type="button"
                    class="group-chevron"
                    onclick={() => toggleGroup(gKey)}
                    aria-label={isCollapsed ? 'Expandir grupo' : 'Colapsar grupo'}
                  >
                    {isCollapsed ? '▸' : '▾'}
                  </button>
                  <button
                    type="button"
                    class="group-title-btn"
                    onclick={() => onOpenNote(group.source_path)}
                    title={group.source_path}
                  >
                    <strong>{group.source_title}</strong>
                    {#if group.modified_unix_secs}
                      <span class="group-date">{formatShortDate(group.modified_unix_secs)}</span>
                    {/if}
                  </button>
                  <span class="badge">{group.hits.length}</span>
                </div>

                {#if !isCollapsed}
                  <ul class="hit-list">
                    {#each group.hits as hit, idx (`${hit.source_path}:${hit.line}:${hit.column_start}:${idx}`)}
                      {@const itemKey = `${hit.source_path}:${hit.line}:${hit.column_start}:${hit.matched_term}`}
                      {@const parts = splitContextMatch(hit.context, hit.matched_term)}
                      <li class="unlinked-row">
                        <button
                          type="button"
                          class="hit-btn unlinked-hit"
                          onclick={() => onOpenNote(hit.source_path)}
                        >
                          <div class="hit-head">
                            <span class="hit-meta">Coincidencia: <strong>{hit.matched_term}</strong></span>
                            <span class="line-pill">L{hit.line}</span>
                          </div>
                          <p class="hit-context" class:expanded={expandFullContext}>
                            {#if parts.match}
                              {parts.before}<mark class="unlinked-mark">{parts.match}</mark>{parts.after}
                            {:else}
                              {hit.context}
                            {/if}
                          </p>
                        </button>
                        <button
                          type="button"
                          class="link-action-btn"
                          disabled={linkingKey === itemKey}
                          onclick={() => void handleLinkClick(hit, itemKey)}
                          title={`Convertir "${hit.matched_term}" en [[wikilink]] hacia ${note.title}`}
                        >
                          {linkingKey === itemKey ? '…' : 'Enlazar'}
                        </button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </section>
  {/if}
</div>

<style>
  .backlinks-core {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .backlinks-core.embedded {
    margin-top: 28px;
    padding: 14px 16px;
    border-top: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 10px;
  }

  .core-head {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .core-title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 6px;
  }

  .core-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  .pin-badge {
    text-transform: none;
    letter-spacing: 0;
    font-size: 11px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .pin-subnote {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-muted);
  }

  .location-switch {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--border-subtle);
    border-radius: 6px;
    overflow: hidden;
    background: var(--bg-editor);
  }

  .loc-btn {
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    padding: 2px 7px;
  }

  .loc-btn:hover {
    color: var(--text-primary);
    background: var(--bg-surface-hover);
  }

  .loc-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .section-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .tab-pill {
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    color: var(--text-secondary);
    border-radius: 999px;
    font-size: 11px;
    padding: 2px 8px;
  }

  .tab-pill.active {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .controls-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .search-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
  }

  .search-wrap input {
    width: 100%;
    min-width: 0;
    padding: 4px 22px 4px 8px;
    border-radius: 6px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    color: var(--text-primary);
    font-size: 12px;
  }

  .clear-btn {
    position: absolute;
    right: 4px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 14px;
    line-height: 1;
    padding: 0 3px;
  }

  .sort-select {
    max-width: 122px;
    padding: 4px 6px;
    border-radius: 6px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    color: var(--text-primary);
    font-size: 11.5px;
  }

  .action-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .mini-tool-btn,
  .preset-btn {
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    color: var(--text-secondary);
    border-radius: 6px;
    font-size: 11px;
    padding: 3px 7px;
  }

  .mini-tool-btn:hover:not(:disabled),
  .preset-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .mini-tool-btn.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .mini-tool-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .excluded-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border-radius: 7px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
  }

  .excluded-head {
    display: flex;
    flex-direction: column;
    gap: 1px;
    font-size: 11.5px;
  }

  .excluded-head small {
    color: var(--text-muted);
    font-size: 10.5px;
  }

  .excluded-form {
    display: flex;
    gap: 5px;
  }

  .excluded-form input {
    flex: 1;
    min-width: 0;
    padding: 4px 7px;
    border-radius: 5px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-primary);
    font-size: 11.5px;
  }

  .excluded-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .exclude-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--bg-surface-hover);
    border: 1px solid var(--border-subtle);
    font-size: 11px;
  }

  .exclude-chip button {
    border: none;
    background: transparent;
    color: var(--text-muted);
    padding: 0 2px;
    font-size: 12px;
  }

  .mention-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .section-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 4px 2px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    font-size: 11.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    text-align: left;
  }

  .badge {
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--bg-surface-hover);
    color: var(--text-secondary);
    font-size: 11px;
  }

  .badge.accent {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .group-list {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .source-group {
    border: 1px solid var(--border-subtle);
    border-radius: 7px;
    background: var(--bg-editor);
    padding: 5px 7px;
  }

  .group-head {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .group-chevron {
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 12px;
    padding: 2px 4px;
  }

  .group-title-btn {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    text-align: left;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 2px 3px;
    color: var(--text-primary);
    font-size: 12.5px;
  }

  .group-title-btn strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .group-title-btn:hover strong {
    color: var(--accent);
  }

  .group-date {
    font-size: 10.5px;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .hit-list {
    list-style: none;
    margin: 5px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .hit-btn {
    width: 100%;
    text-align: left;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    border-radius: 6px;
    padding: 6px 8px;
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
    font-size: 11px;
    color: var(--text-muted);
  }

  .hit-meta strong {
    color: var(--text-primary);
  }

  .line-pill {
    font-size: 10.5px;
    color: var(--text-muted);
    background: var(--bg-surface-hover);
    padding: 1px 5px;
    border-radius: 4px;
    flex-shrink: 0;
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
    word-break: break-word;
  }

  .hit-context.expanded {
    display: block;
    -webkit-line-clamp: unset;
    line-clamp: unset;
    white-space: pre-wrap;
  }

  .unlinked-row {
    display: flex;
    align-items: stretch;
    gap: 5px;
  }

  .unlinked-hit {
    flex: 1;
    min-width: 0;
  }

  .unlinked-mark {
    background: var(--pending-soft);
    color: var(--pending);
    padding: 0 2px;
    border-radius: 3px;
    font-weight: 600;
  }

  .link-action-btn {
    flex-shrink: 0;
    align-self: center;
    border: 1px solid var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    border-radius: 6px;
    padding: 4px 8px;
    font-size: 11.5px;
    font-weight: 600;
  }

  .link-action-btn:hover:not(:disabled) {
    background: var(--accent);
    color: #fff;
  }

  .link-action-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .empty-note,
  .error-note {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
  }

  .error-note {
    color: var(--danger);
  }
</style>
