<script lang="ts">
  import {
    askAiCopilot,
    autoConnectNotes,
    createKnowledgeCanvas,
    getAiProviderStatus,
    getVaultKnowledgeMap,
    saveAiProviderConfig,
    suggestNoteConnections,
  } from '../api';
  import { renderMarkdownPreview } from '../markdown';
  import type {
    AiChatMessage,
    AiChatResponse,
    AiEnrichmentPreview,
    AiProviderConfig,
    AiProviderKind,
    AiProviderStatus,
    AutoConnectBatchResponse,
    AutoConnectMode,
    AutoConnectNoteOutcome,
    CandidateConnection,
    ConnectionSignal,
    KnowledgeCanvasResult,
    NoteDetail,
    VaultKnowledgeMap,
  } from '../types';

  type CopilotTab = 'note' | 'batch' | 'canvas' | 'chat' | 'provider';

  interface ChatTurn {
    id: string;
    role: 'user' | 'assistant';
    text: string;
    response?: AiChatResponse;
    savedNotePath?: string;
  }

  interface Props {
    open: boolean;
    initialTab?: CopilotTab;
    vaultPath: string | null;
    activeNote: NoteDetail | null;
    defaultInboxFolder: string;
    knownNoteKeys: Set<string>;
    onClose: () => void;
    onOpenNote: (relativePath: string) => void | Promise<void>;
    onOpenWikilink: (target: string, heading?: string | null) => void | Promise<void>;
    onApplied: (response: AutoConnectBatchResponse) => void | Promise<void>;
    onCanvasCreated: (result: KnowledgeCanvasResult) => void | Promise<void>;
    onCreateNoteFromChat?: (relativePath: string, content: string) => void | Promise<void>;
    onProviderStatusChange?: (status: AiProviderStatus) => void;
  }

  let {
    open,
    initialTab = 'note',
    vaultPath,
    activeNote,
    defaultInboxFolder,
    knownNoteKeys,
    onClose,
    onOpenNote,
    onOpenWikilink,
    onApplied,
    onCanvasCreated,
    onCreateNoteFromChat,
    onProviderStatusChange,
  }: Props = $props();

  const DEFAULT_MODELS: Record<AiProviderKind, string> = {
    deterministic: 'deterministic-rust-v1',
    claude_cli: 'claude-sonnet-4-5',
    gemini_cli: 'gemini-2.5-flash',
    gemini_http: 'gemini-2.5-flash',
    compatible_http: 'gpt-4o-mini',
  };

  let activeTab = $state<CopilotTab>('note');
  let executionMode = $state<AutoConnectMode>('preview');
  let loadingStatus = $state(false);
  let busyMessage = $state<string | null>(null);
  let errorMsg = $state<string | null>(null);
  let successMsg = $state<string | null>(null);

  let providerStatus = $state<AiProviderStatus | null>(null);
  let knowledgeMap = $state<VaultKnowledgeMap | null>(null);
  let candidates = $state<CandidateConnection[]>([]);
  let candidatesLoading = $state(false);
  let candidatesLoadedPath = $state<string | null>(null);

  // Estado pestaña Nota Activa
  let noteMaxConn = $state(8);
  let noteAppendRelated = $state(true);
  let noteOutcome = $state<AutoConnectNoteOutcome | null>(null);

  // Estado pestaña Lote / Carpeta
  let batchFolder = $state('Inbox');
  let batchFolderInitialized = false;
  let batchMaxConn = $state(8);
  let batchAppendRelated = $state(true);
  let batchResponse = $state<AutoConnectBatchResponse | null>(null);
  let expandedBatchPaths = $state<Record<string, boolean>>({});

  // Estado pestaña Mapa .canvas
  let canvasPath = $state('Mapas/Mapa de Conocimiento.canvas');
  let canvasTitle = $state('Mapa de Conocimiento PKM');
  let canvasUseActiveSeed = $state(true);
  let canvasIncludeNeighbors = $state(true);
  let canvasMaxNodes = $state(12);

  // Estado pestaña Chat PKM
  const CHAT_STORAGE_KEY = 'romsidian-copilot-chat';
  function loadSavedChatTurns(): ChatTurn[] {
    try {
      const raw = localStorage.getItem(CHAT_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed)) return parsed;
      }
    } catch {}
    return [];
  }

  let chatInput = $state('');
  let chatIncludeMap = $state(true);
  let chatTurns = $state<ChatTurn[]>(loadSavedChatTurns());
  let chatBusy = $state(false);
  let savingTurnId = $state<string | null>(null);
  let savingTurnPath = $state('');
  let copiedTurnId = $state<string | null>(null);

  $effect(() => {
    const turns = chatTurns;
    try {
      localStorage.setItem(CHAT_STORAGE_KEY, JSON.stringify(turns));
    } catch {}
  });

  function handleClearChat() {
    chatTurns = [];
    savingTurnId = null;
    try {
      localStorage.removeItem(CHAT_STORAGE_KEY);
    } catch {}
  }

  function deriveSmartNotePath(turn: ChatTurn, userTurnText?: string): string {
    const headerMatch = /^#{1,3}\s+(.+)$/m.exec(turn.text);
    let title = '';
    if (headerMatch) {
      title = headerMatch[1].trim();
    } else if (userTurnText) {
      title = userTurnText.trim();
    } else {
      title = 'Nota Copiloto IA';
    }
    title = title.replace(/[\\/:*?"<>|#^\[\]]/g, '').trim();
    if (title.length > 60) {
      title = title.slice(0, 60).trim();
    }
    if (!title) title = 'Respuesta Copiloto IA';
    if (!title.toLowerCase().endsWith('.md')) {
      title += '.md';
    }
    const folder = defaultInboxFolder?.trim() || '';
    return folder ? `${folder}/${title}` : title;
  }

  function handleStartSaveAsNote(turn: ChatTurn, prevTurn?: ChatTurn) {
    savingTurnId = turn.id;
    savingTurnPath = deriveSmartNotePath(turn, prevTurn?.text);
  }

  async function handleConfirmSaveAsNote(turn: ChatTurn) {
    const cleanPath = savingTurnPath.trim();
    if (!cleanPath) return;
    try {
      if (onCreateNoteFromChat) {
        await onCreateNoteFromChat(cleanPath, turn.text);
      }
      turn.savedNotePath = cleanPath;
      chatTurns = [...chatTurns];
      savingTurnId = null;
      successMsg = `Nota creada y abierta: ${cleanPath}`;
    } catch (err) {
      errorMsg = String(err);
    }
  }

  async function handleCopyTurnText(turn: ChatTurn) {
    try {
      await navigator.clipboard.writeText(turn.text);
      copiedTurnId = turn.id;
      setTimeout(() => {
        if (copiedTurnId === turn.id) copiedTurnId = null;
      }, 2000);
    } catch {
      errorMsg = 'No se pudo copiar automáticamente al portapapeles.';
    }
  }

  // Estado pestaña Proveedor IA & MCP
  let cfgProvider = $state<AiProviderKind>('deterministic');
  let cfgModel = $state('deterministic-rust-v1');
  let cfgEndpointUrl = $state('');
  let cfgCliBinary = $state('');
  let cfgMaxConnections = $state(8);
  let cfgAppendRelated = $state(true);
  let cfgFallbackDeterministic = $state(true);
  let geminiApiKeyInput = $state('');
  let compatibleApiKeyInput = $state('');
  let savingConfig = $state(false);
  let copiedMcpNotice = $state<string | null>(null);

  let pendingBatchOutcomes = $derived.by(() =>
    batchResponse
      ? batchResponse.items.filter((item) => item.preview.changed && !item.applied)
      : [],
  );

  let mcpConfigSnippet = $derived.by(() => {
    const cleanVault = vaultPath?.trim() || 'C:/ruta/a/tu/boveda';
    return JSON.stringify(
      {
        mcpServers: {
          romsidian: {
            command: 'romsidian-mcp',
            args: ['--vault', cleanVault],
          },
        },
      },
      null,
      2,
    );
  });

  function providerShortLabel(kind: AiProviderKind | undefined): string {
    switch (kind) {
      case 'claude_cli':
        return 'Claude CLI (Suscripción)';
      case 'gemini_cli':
        return 'Gemini CLI';
      case 'gemini_http':
        return 'Gemini API HTTP';
      case 'compatible_http':
        return 'Compatible HTTP';
      case 'deterministic':
      default:
        return 'Motor Local (Offline)';
    }
  }

  function signalLabel(sig: ConnectionSignal): string {
    switch (sig.kind) {
      case 'outgoing_unlinked_mention':
        return `Menciona "${sig.matched_term}" (L${sig.line})`;
      case 'incoming_unlinked_mention':
        return `Te mencionan "${sig.matched_term}" (L${sig.line})`;
      case 'shared_tags':
        return `Tags: ${sig.tags.map((t) => `#${t}`).join(', ')}`;
      case 'shared_fields':
        return `Campos: ${sig.fields.join(', ')}`;
      case 'lexical_bm25':
        return `Similitud FTS5: ${sig.matched_keywords.slice(0, 4).join(', ')}`;
    }
  }

  function syncFormFromStatus(status: AiProviderStatus) {
    providerStatus = status;
    onProviderStatusChange?.(status);
    const c = status.active_config;
    cfgProvider = c.provider;
    cfgModel = c.model;
    cfgEndpointUrl = c.endpoint_url ?? '';
    cfgCliBinary = c.cli_binary_override ?? '';
    cfgMaxConnections = c.max_connections;
    cfgAppendRelated = c.append_related_section;
    cfgFallbackDeterministic = c.fallback_to_deterministic;
    noteMaxConn = c.max_connections;
    noteAppendRelated = c.append_related_section;
    batchMaxConn = c.max_connections;
    batchAppendRelated = c.append_related_section;
  }

  async function loadInitialContext() {
    errorMsg = null;
    loadingStatus = true;
    try {
      const [status, map] = await Promise.all([
        getAiProviderStatus(),
        getVaultKnowledgeMap(),
      ]);
      syncFormFromStatus(status);
      knowledgeMap = map;
    } catch (err) {
      errorMsg = String(err);
    } finally {
      loadingStatus = false;
    }
  }

  async function refreshCandidatesForActiveNote(force = false) {
    const path = activeNote?.relative_path ?? null;
    if (!path) {
      candidates = [];
      candidatesLoadedPath = null;
      return;
    }
    if (!force && candidatesLoadedPath === path) return;
    candidatesLoading = true;
    try {
      candidates = await suggestNoteConnections(path, 12);
      candidatesLoadedPath = path;
    } catch (err) {
      errorMsg = String(err);
      candidates = [];
    } finally {
      candidatesLoading = false;
    }
  }

  let prevOpen = false;
  let lastHandledNotePath: string | null = null;

  $effect(() => {
    const isOpen = open;
    const requestedTab = initialTab;
    const currentPath = activeNote?.relative_path ?? null;

    if (isOpen && !prevOpen) {
      prevOpen = true;
      activeTab = requestedTab;
      if (!batchFolderInitialized) {
        batchFolder = defaultInboxFolder || 'Inbox';
        batchFolderInitialized = true;
      }
      successMsg = null;
      errorMsg = null;
      lastHandledNotePath = currentPath;
      void loadInitialContext();
      void refreshCandidatesForActiveNote(true);
      return;
    }

    if (!isOpen && prevOpen) {
      prevOpen = false;
      return;
    }

    if (isOpen && currentPath !== lastHandledNotePath) {
      lastHandledNotePath = currentPath;
      noteOutcome = null;
      void refreshCandidatesForActiveNote(true);
    }
  });

  function handleProviderRadioChange(next: AiProviderKind) {
    const prevDefault = DEFAULT_MODELS[cfgProvider];
    cfgProvider = next;
    if (!cfgModel.trim() || cfgModel.trim() === prevDefault) {
      cfgModel = DEFAULT_MODELS[next];
    }
  }

  async function handleRunActiveNoteAutoConnect() {
    if (!activeNote) return;
    errorMsg = null;
    successMsg = null;
    busyMessage =
      executionMode === 'preview'
        ? 'Analizando nota activa y generando Vista Previa...'
        : 'Auto-conectando nota activa e indexando SQLite...';
    try {
      const res = await autoConnectNotes({
        path: activeNote.relative_path,
        mode: executionMode,
        max_connections: noteMaxConn,
        append_related_section: noteAppendRelated,
      });
      noteOutcome = res.items[0] ?? null;
      if (executionMode === 'apply' && res.applied_count > 0) {
        await onApplied(res);
        knowledgeMap = await getVaultKnowledgeMap();
        await refreshCandidatesForActiveNote(true);
        successMsg = `Cambios aplicados en ${activeNote.relative_path} (${providerShortLabel(res.provider_used)}).`;
      } else if (noteOutcome && !noteOutcome.preview.changed) {
        successMsg = 'La nota ya posee las etiquetas, metadatos y enlaces sugeridos.';
      }
    } catch (err) {
      errorMsg = String(err);
    } finally {
      busyMessage = null;
    }
  }

  async function handleApplyPreviewedNote() {
    if (!noteOutcome || !noteOutcome.preview.changed) return;
    errorMsg = null;
    successMsg = null;
    busyMessage = `Aplicando cambios en ${noteOutcome.path}...`;
    try {
      const res = await autoConnectNotes({
        path: noteOutcome.path,
        mode: 'apply',
        max_connections: noteMaxConn,
        append_related_section: noteAppendRelated,
        custom_plan: noteOutcome.plan,
      });
      if (res.items[0]) {
        noteOutcome = res.items[0];
      }
      await onApplied(res);
      knowledgeMap = await getVaultKnowledgeMap();
      await refreshCandidatesForActiveNote(true);
      successMsg = `Nota ${noteOutcome.path} enriquecida y reindexada en SQLite.`;
    } catch (err) {
      errorMsg = String(err);
    } finally {
      busyMessage = null;
    }
  }

  async function handleRunBatchAutoConnect() {
    errorMsg = null;
    successMsg = null;
    const targetFolder = batchFolder.trim();
    busyMessage =
      executionMode === 'preview'
        ? `Analizando ${targetFolder ? `${targetFolder}/` : 'toda la bóveda'} en Vista Previa...`
        : `Auto-conectando ${targetFolder ? `${targetFolder}/` : 'toda la bóveda'} en lote...`;
    try {
      const res = await autoConnectNotes({
        folder: targetFolder,
        mode: executionMode,
        max_connections: batchMaxConn,
        append_related_section: batchAppendRelated,
      });
      batchResponse = res;
      const expanded: Record<string, boolean> = {};
      for (const item of res.items) {
        if (item.preview.changed) expanded[item.path] = res.changed_count <= 4;
      }
      expandedBatchPaths = expanded;
      if (executionMode === 'apply' && res.applied_count > 0) {
        await onApplied(res);
        knowledgeMap = await getVaultKnowledgeMap();
        await refreshCandidatesForActiveNote(true);
        successMsg = `Lote completado: ${res.applied_count} nota(s) modificada(s) y reindexada(s).`;
      } else {
        successMsg = `Análisis terminado: ${res.total_processed} notas revisadas, ${res.changed_count} con mejoras detectadas.`;
      }
    } catch (err) {
      errorMsg = String(err);
    } finally {
      busyMessage = null;
    }
  }

  async function handleApplySingleBatchItem(outcome: AutoConnectNoteOutcome) {
    if (!batchResponse || !outcome.preview.changed || outcome.applied) return;
    errorMsg = null;
    successMsg = null;
    busyMessage = `Aplicando cambios en ${outcome.path}...`;
    try {
      const res = await autoConnectNotes({
        path: outcome.path,
        mode: 'apply',
        max_connections: batchMaxConn,
        append_related_section: batchAppendRelated,
        custom_plan: outcome.plan,
      });
      const updatedItem = res.items[0];
      if (updatedItem) {
        batchResponse = {
          ...batchResponse,
          applied_count: batchResponse.applied_count + (updatedItem.applied ? 1 : 0),
          snapshot: res.snapshot ?? batchResponse.snapshot,
          items: batchResponse.items.map((it) => (it.path === outcome.path ? updatedItem : it)),
        };
      }
      await onApplied(res);
      knowledgeMap = await getVaultKnowledgeMap();
      await refreshCandidatesForActiveNote(true);
      successMsg = `Aplicados los cambios en ${outcome.path}.`;
    } catch (err) {
      errorMsg = String(err);
    } finally {
      busyMessage = null;
    }
  }

  async function handleApplyAllBatchPending() {
    if (!batchResponse || pendingBatchOutcomes.length === 0) return;
    errorMsg = null;
    successMsg = null;
    const toApply = pendingBatchOutcomes;
    busyMessage = `Aplicando cambios en ${toApply.length} nota(s)...`;
    try {
      let lastRes: AutoConnectBatchResponse | null = null;
      const updatedMap = new Map<string, AutoConnectNoteOutcome>();
      for (const item of toApply) {
        const singleRes = await autoConnectNotes({
          path: item.path,
          mode: 'apply',
          max_connections: batchMaxConn,
          append_related_section: batchAppendRelated,
          custom_plan: item.plan,
        });
        lastRes = singleRes;
        if (singleRes.items[0]) {
          updatedMap.set(item.path, singleRes.items[0]);
        }
      }
      const nextItems = batchResponse.items.map((it) => updatedMap.get(it.path) ?? it);
      const totalApplied = nextItems.filter((it) => it.applied).length;
      const mergedResponse: AutoConnectBatchResponse = {
        ...batchResponse,
        mode: 'apply',
        applied_count: totalApplied,
        items: nextItems,
        snapshot: lastRes?.snapshot ?? batchResponse.snapshot,
      };
      batchResponse = mergedResponse;
      await onApplied(mergedResponse);
      knowledgeMap = await getVaultKnowledgeMap();
      await refreshCandidatesForActiveNote(true);
      successMsg = `Lote aplicado: ${updatedMap.size} nota(s) actualizadas y reindexadas en SQLite.`;
    } catch (err) {
      errorMsg = String(err);
    } finally {
      busyMessage = null;
    }
  }

  async function handleCreateKnowledgeCanvas() {
    const cleanPath = canvasPath.trim();
    if (!cleanPath) {
      errorMsg = 'Indica una ruta para el archivo .canvas';
      return;
    }
    errorMsg = null;
    successMsg = null;
    busyMessage = 'Generando lienzo JSON Canvas desde el grafo de conocimiento...';
    try {
      const seedPaths =
        canvasUseActiveSeed && activeNote ? [activeNote.relative_path] : [];
      const result = await createKnowledgeCanvas({
        canvas_path: cleanPath,
        title: canvasTitle.trim() || null,
        seed_note_paths: seedPaths,
        include_neighbors: canvasIncludeNeighbors,
        max_nodes: canvasMaxNodes,
      });
      await onCanvasCreated(result);
    } catch (err) {
      errorMsg = String(err);
    } finally {
      busyMessage = null;
    }
  }

  async function handleSendChat(customPrompt?: string) {
    const promptText = (customPrompt ?? chatInput).trim();
    if (!promptText || chatBusy) return;
    errorMsg = null;
    if (!customPrompt) chatInput = '';

    const historyPayload: AiChatMessage[] = chatTurns.slice(-8).map((t) => ({
      role: t.role,
      content: t.text,
    }));

    const userTurn: ChatTurn = {
      id: `u-${Date.now()}-${Math.random()}`,
      role: 'user',
      text: promptText,
    };
    chatTurns = [...chatTurns, userTurn];
    chatBusy = true;

    try {
      const res = await askAiCopilot({
        prompt: promptText,
        active_note_path: activeNote?.relative_path ?? null,
        include_vault_map: chatIncludeMap,
        history: historyPayload,
      });
      chatTurns = [
        ...chatTurns,
        {
          id: `a-${Date.now()}-${Math.random()}`,
          role: 'assistant',
          text: res.reply_markdown,
          response: res,
        },
      ];
    } catch (err) {
      errorMsg = String(err);
    } finally {
      chatBusy = false;
    }
  }

  function handleSuggestedActionClick(actionText: string) {
    const lower = actionText.toLowerCase();
    if (lower.includes('lote') || lower.includes('inbox') || lower.includes('carpeta')) {
      activeTab = 'batch';
      return;
    }
    if (lower.includes('canvas') || lower.includes('mapa')) {
      activeTab = 'canvas';
      return;
    }
    if (lower.includes('auto-conectar') || lower.includes('nota activa')) {
      activeTab = 'note';
      return;
    }
    if (lower.includes('proveedor') || lower.includes('claude') || lower.includes('gemini')) {
      activeTab = 'provider';
      return;
    }
    void handleSendChat(actionText);
  }

  async function handleSaveProviderConfig(
    clearGemini = false,
    clearCompatible = false,
  ) {
    savingConfig = true;
    errorMsg = null;
    successMsg = null;
    try {
      const config: AiProviderConfig = {
        provider: cfgProvider,
        model: cfgModel.trim() || DEFAULT_MODELS[cfgProvider],
        endpoint_url: cfgEndpointUrl.trim() || null,
        cli_binary_override: cfgCliBinary.trim() || null,
        max_connections: Math.max(1, Math.min(25, Number(cfgMaxConnections) || 8)),
        append_related_section: cfgAppendRelated,
        fallback_to_deterministic: cfgFallbackDeterministic,
      };
      const updated = await saveAiProviderConfig(config, {
        geminiApiKey: geminiApiKeyInput.trim() || undefined,
        compatibleApiKey: compatibleApiKeyInput.trim() || undefined,
        clearGeminiApiKey: clearGemini,
        clearCompatibleApiKey: clearCompatible,
      });
      syncFormFromStatus(updated);
      geminiApiKeyInput = '';
      compatibleApiKeyInput = '';
      successMsg =
        clearGemini || clearCompatible
          ? 'Credencial eliminada del llavero seguro del SO y configuración actualizada.'
          : 'Configuración de IA guardada (.romsidian/ai_config.json + llavero seguro del SO).';
    } catch (err) {
      errorMsg = String(err);
    } finally {
      savingConfig = false;
    }
  }

  async function handleCopyMcpConfig() {
    copiedMcpNotice = null;
    try {
      await navigator.clipboard.writeText(mcpConfigSnippet);
      copiedMcpNotice = 'JSON MCP copiado al portapapeles';
      setTimeout(() => {
        copiedMcpNotice = null;
      }, 2500);
    } catch {
      copiedMcpNotice = 'No se pudo copiar automáticamente; selecciona el bloque manualmente.';
    }
  }

  function delegateChatLinks(node: HTMLElement) {
    function handleClick(event: MouseEvent) {
      const btn = (event.target as HTMLElement | null)?.closest<HTMLButtonElement>(
        'button[data-wikilink]',
      );
      if (!btn) return;
      const wl = btn.dataset.wikilink;
      const heading = btn.dataset.heading || null;
      if (wl) {
        event.preventDefault();
        void onOpenWikilink(wl, heading);
      }
    }
    node.addEventListener('click', handleClick);
    return {
      destroy() {
        node.removeEventListener('click', handleClick);
      },
    };
  }
</script>

{#snippet previewDiffBlock(preview: AiEnrichmentPreview)}
  <div class="diff-summary">
    <div class="diff-badges">
      {#if preview.added_frontmatter_keys.length > 0}
        <span class="chip ok">
          + Frontmatter: {preview.added_frontmatter_keys.join(', ')}
        </span>
      {/if}
      {#if preview.added_tags.length > 0}
        <span class="chip accent">
          + Tags: {preview.added_tags.map((t) => `#${t}`).join(' ')}
        </span>
      {/if}
      {#if preview.injected_wikilinks.length > 0}
        <span class="chip accent">
          + {preview.injected_wikilinks.length} [[wikilink(s)]] en texto
        </span>
      {/if}
      {#if preview.appended_related_links.length > 0}
        <span class="chip warn">
          + {preview.appended_related_links.length} conexión(es) al pie
        </span>
      {/if}
      {#if !preview.changed}
        <span class="chip muted">Sin modificaciones pendientes</span>
      {/if}
    </div>

    {#if preview.unified_diff_lines.length > 0}
      <div class="diff-code" role="region" aria-label="Diff unificado de {preview.relative_path}">
        {#each preview.unified_diff_lines as line, idx (idx)}
          <div
            class="diff-line"
            class:add={line.startsWith('+') && !line.startsWith('+++')}
            class:del={line.startsWith('-') && !line.startsWith('---')}
            class:hunk={line.startsWith('@@') || line.startsWith('---') || line.startsWith('+++')}
          >
            {line || ' '}
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/snippet}

{#if open}
  <div
    class="copilot-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div
      class="copilot-modal"
      role="dialog"
      aria-modal="true"
      aria-label="Copiloto PKM y Auto-Conexión IA"
    >
      <header class="copilot-header">
        <div class="header-title-row">
          <div>
            <h2>✦ Copiloto PKM & Auto-Conexión IA</h2>
            <p class="subtitle">
              Analiza fuentes, genera <code>frontmatter</code>, <code>#tags</code> y teje
              <code>[[wikilinks]]</code> entre tus notas.
            </p>
          </div>
          <div class="header-right-controls">
            <span class="provider-pill" title="Modelo: {providerStatus?.active_config.model ?? ''}">
              {providerShortLabel(providerStatus?.active_config.provider)}
              {#if providerStatus}
                <small>· {providerStatus.active_config.model}</small>
              {/if}
            </span>
            <button type="button" class="btn" onclick={onClose} aria-label="Cerrar panel">
              Cerrar
            </button>
          </div>
        </div>

        <div class="toolbar-strip">
          <div class="tabs-bar" role="tablist" aria-label="Modos del Copiloto PKM">
            <button
              type="button"
              role="tab"
              aria-selected={activeTab === 'note'}
              class="tab-btn"
              class:active={activeTab === 'note'}
              onclick={() => (activeTab = 'note')}
            >
              Nota Activa
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={activeTab === 'batch'}
              class="tab-btn"
              class:active={activeTab === 'batch'}
              onclick={() => (activeTab = 'batch')}
            >
              Lote / Carpeta
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={activeTab === 'canvas'}
              class="tab-btn"
              class:active={activeTab === 'canvas'}
              onclick={() => (activeTab = 'canvas')}
            >
              Mapa .canvas
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={activeTab === 'chat'}
              class="tab-btn"
              class:active={activeTab === 'chat'}
              onclick={() => (activeTab = 'chat')}
            >
              Chat PKM
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={activeTab === 'provider'}
              class="tab-btn"
              class:active={activeTab === 'provider'}
              onclick={() => (activeTab = 'provider')}
            >
              Proveedor IA & MCP
            </button>
          </div>

          <div class="safety-toggle" role="group" aria-label="Modo de escritura">
            <button
              type="button"
              class="safety-btn"
              class:active={executionMode === 'preview'}
              onclick={() => (executionMode = 'preview')}
              title="Muestra el diff antes de modificar archivos"
            >
              👁 Vista Previa
            </button>
            <button
              type="button"
              class="safety-btn auto"
              class:active={executionMode === 'apply'}
              onclick={() => (executionMode = 'apply')}
              title="Aplica los cambios directo en .md y reindexa SQLite"
            >
              ⚡ Automático
            </button>
          </div>
        </div>
      </header>

      {#if errorMsg}
        <div class="alert error" role="alert">
          <span>{errorMsg}</span>
          <button type="button" onclick={() => (errorMsg = null)}>✕</button>
        </div>
      {/if}
      {#if successMsg}
        <div class="alert ok" role="status">
          <span>{successMsg}</span>
          <button type="button" onclick={() => (successMsg = null)}>✕</button>
        </div>
      {/if}
      {#if busyMessage}
        <div class="alert info" role="status">{busyMessage}</div>
      {/if}

      <div class="copilot-body">
        {#if knowledgeMap}
          <div class="vault-map-bar">
            <span><strong>{knowledgeMap.total_notes}</strong> notas</span>
            <span><strong>{knowledgeMap.total_links}</strong> enlaces</span>
            <span><strong>{knowledgeMap.total_tags}</strong> #tags</span>
            <span>
              <strong>{knowledgeMap.orphan_paths.length}</strong> huérfana(s)
            </span>
            {#if knowledgeMap.folders.length > 0}
              <span>{knowledgeMap.folders.length} carpeta(s)</span>
            {/if}
          </div>
        {/if}

        {#if activeTab === 'note'}
          {#if !activeNote}
            <div class="empty-box">
              Abre o crea una nota Markdown en el editor para analizar sus menciones no enlazadas,
              sugerir <code>#tags</code> y generar su <code>frontmatter</code>.
            </div>
          {:else}
            <section class="panel-card">
              <div class="card-head">
                <div>
                  <h3>Nota activa: <code>{activeNote.relative_path}</code></h3>
                  <small>
                    {activeNote.wikilinks.length} enlace(s) saliente(s) ·
                    {activeNote.backlinks.length} backlink(s) ·
                    {activeNote.tags.length} #tag(s)
                  </small>
                </div>
                <button
                  type="button"
                  class="btn primary"
                  disabled={Boolean(busyMessage)}
                  onclick={() => void handleRunActiveNoteAutoConnect()}
                >
                  {executionMode === 'preview'
                    ? '🔍 Analizar y Previsualizar'
                    : '⚡ Auto-Conectar Nota Ahora'}
                </button>
              </div>

              <div class="inline-options">
                <label>
                  Máx. conexiones:
                  <input type="number" min="1" max="20" bind:value={noteMaxConn} />
                </label>
                <label class="checkbox-inline">
                  <input type="checkbox" bind:checked={noteAppendRelated} />
                  <span>Añadir sección <code>## Conexiones Relacionadas</code> si hay nuevas</span>
                </label>
              </div>
            </section>

            {#if noteOutcome}
              <section class="panel-card highlight">
                <div class="card-head">
                  <div>
                    <h4>
                      Resultado ({providerShortLabel(noteOutcome.provider_used)}) ·
                      {noteOutcome.applied
                        ? 'Aplicado en disco ✓'
                        : noteOutcome.preview.changed
                          ? 'Vista previa lista para confirmar'
                          : 'Sin cambios pendientes'}
                    </h4>
                    {#if noteOutcome.warning}
                      <p class="warn-text">{noteOutcome.warning}</p>
                    {/if}
                  </div>
                  {#if !noteOutcome.applied && noteOutcome.preview.changed}
                    <button
                      type="button"
                      class="btn primary"
                      disabled={Boolean(busyMessage)}
                      onclick={() => void handleApplyPreviewedNote()}
                    >
                      ✓ Aplicar Cambios en {activeNote.title}
                    </button>
                  {/if}
                </div>

                {@render previewDiffBlock(noteOutcome.preview)}
              </section>
            {/if}

            <section class="panel-card">
              <div class="card-head">
                <h4>Conexiones candidatas detectadas (SQLite FTS5 + Menciones + #Tags)</h4>
                <button
                  type="button"
                  class="btn"
                  disabled={candidatesLoading}
                  onclick={() => void refreshCandidatesForActiveNote(true)}
                >
                  {candidatesLoading ? 'Buscando...' : '↻ Recalcular'}
                </button>
              </div>

              {#if candidatesLoading}
                <p class="muted-note">Calculando conexiones candidatas en la bóveda...</p>
              {:else if candidates.length === 0}
                <p class="muted-note">
                  No se encontraron candidatos adicionales para esta nota todavía.
                </p>
              {:else}
                <div class="candidate-list">
                  {#each candidates as cand (cand.target_path)}
                    <article class="candidate-item">
                      <div class="candidate-top">
                        <div>
                          <strong>{cand.suggested_wikilink}</strong>
                          <span class="path-label">{cand.target_path}</span>
                        </div>
                        <div class="candidate-actions">
                          <span class="chip" class:ok={cand.already_linked}>
                            {cand.already_linked ? 'Ya enlazada' : `Score ${Math.round(cand.score)}`}
                          </span>
                          <button
                            type="button"
                            class="btn small"
                            onclick={() => void onOpenNote(cand.target_path)}
                          >
                            Abrir
                          </button>
                        </div>
                      </div>
                      <p class="reason-line">{cand.reason_summary}</p>
                      {#if cand.signals.length > 0 || cand.target_tags.length > 0}
                        <div class="signal-pills">
                          {#each cand.signals as sig, idx (idx)}
                            <span class="signal-pill">{signalLabel(sig)}</span>
                          {/each}
                          {#each cand.target_tags as tg (tg)}
                            <span class="signal-pill tag">#{tg}</span>
                          {/each}
                        </div>
                      {/if}
                    </article>
                  {/each}
                </div>
              {/if}
            </section>
          {/if}
        {:else if activeTab === 'batch'}
          <section class="panel-card">
            <h3>Analizar y Conectar Fuentes en Lote</h3>
            <p class="muted-note">
              Ideal para procesar notas capturadas desde YouTube o lecturas en <code>Inbox/</code> y
              tejer enlaces bidireccionales, <code>#tags</code> y <code>frontmatter</code>.
            </p>

            <div class="batch-controls">
              <label class="field">
                <span>Carpeta objetivo (deja vacío para analizar toda la bóveda):</span>
                <input
                  type="text"
                  bind:value={batchFolder}
                  placeholder="ej. Inbox (vacío = toda la bóveda)"
                />
              </label>

              {#if knowledgeMap}
                <div class="folder-pills">
                  <button
                    type="button"
                    class="pill-btn"
                    class:active={batchFolder === ''}
                    onclick={() => (batchFolder = '')}
                  >
                    Toda la bóveda ({knowledgeMap.total_notes})
                  </button>
                  <button
                    type="button"
                    class="pill-btn"
                    class:active={batchFolder === (defaultInboxFolder || 'Inbox')}
                    onclick={() => (batchFolder = defaultInboxFolder || 'Inbox')}
                  >
                    {defaultInboxFolder || 'Inbox'}/
                  </button>
                  {#each knowledgeMap.folders as folder (folder)}
                    {#if folder !== (defaultInboxFolder || 'Inbox')}
                      <button
                        type="button"
                        class="pill-btn"
                        class:active={batchFolder === folder}
                        onclick={() => (batchFolder = folder)}
                      >
                        {folder}/
                      </button>
                    {/if}
                  {/each}
                </div>
              {/if}

              <div class="inline-options">
                <label>
                  Máx. conexiones por nota:
                  <input type="number" min="1" max="25" bind:value={batchMaxConn} />
                </label>
                <label class="checkbox-inline">
                  <input type="checkbox" bind:checked={batchAppendRelated} />
                  <span>Incluir <code>## Conexiones Relacionadas</code></span>
                </label>
                <button
                  type="button"
                  class="btn primary"
                  disabled={Boolean(busyMessage)}
                  onclick={() => void handleRunBatchAutoConnect()}
                >
                  {executionMode === 'preview'
                    ? '🔍 Analizar Lote (Vista Previa)'
                    : '⚡ Ejecutar Lote Automático'}
                </button>
              </div>
            </div>
          </section>

          {#if batchResponse}
            <section class="panel-card">
              <div class="card-head">
                <div>
                  <h4>
                    Resultado del lote · {batchResponse.total_processed} notas procesadas ·
                    {batchResponse.changed_count} con mejoras ·
                    {batchResponse.applied_count} aplicadas
                  </h4>
                  <small>Proveedor usado: {providerShortLabel(batchResponse.provider_used)}</small>
                </div>
                {#if pendingBatchOutcomes.length > 0}
                  <button
                    type="button"
                    class="btn primary"
                    disabled={Boolean(busyMessage)}
                    onclick={() => void handleApplyAllBatchPending()}
                  >
                    ✓ Aplicar Todos ({pendingBatchOutcomes.length} notas)
                  </button>
                {/if}
              </div>

              {#if batchResponse.items.length === 0}
                <p class="muted-note">
                  No se encontraron notas Markdown dentro de esa carpeta (excluyendo plantillas).
                </p>
              {:else}
                <div class="batch-items">
                  {#each batchResponse.items as item (item.path)}
                    <article class="batch-item" class:changed={item.preview.changed}>
                      <div class="candidate-top">
                        <div>
                          <button
                            type="button"
                            class="link-btn"
                            onclick={() => void onOpenNote(item.path)}
                          >
                            {item.path}
                          </button>
                          {#if item.warning}
                            <small class="warn-text"> · {item.warning}</small>
                          {/if}
                        </div>
                        <div class="candidate-actions">
                          {#if item.applied}
                            <span class="chip ok">Aplicado ✓</span>
                          {:else if item.preview.changed}
                            <span class="chip warn">Cambios listos</span>
                            <button
                              type="button"
                              class="btn small primary"
                              disabled={Boolean(busyMessage)}
                              onclick={() => void handleApplySingleBatchItem(item)}
                            >
                              ✓ Aplicar
                            </button>
                          {:else}
                            <span class="chip muted">Sin cambios</span>
                          {/if}
                          {#if item.preview.changed}
                            <button
                              type="button"
                              class="btn small"
                              onclick={() =>
                                (expandedBatchPaths = {
                                  ...expandedBatchPaths,
                                  [item.path]: !expandedBatchPaths[item.path],
                                })}
                            >
                              {expandedBatchPaths[item.path] ? 'Ocultar diff' : 'Ver diff'}
                            </button>
                          {/if}
                        </div>
                      </div>

                      {#if item.preview.changed && expandedBatchPaths[item.path]}
                        {@render previewDiffBlock(item.preview)}
                      {/if}
                    </article>
                  {/each}
                </div>
              {/if}
            </section>
          {/if}
        {:else if activeTab === 'canvas'}
          <section class="panel-card">
            <h3>Generar Lienzo Visual <code>.canvas</code> desde Fuentes y Conexiones</h3>
            <p class="muted-note">
              Crea un archivo compatible con JSON Canvas 1.0 organizando tus notas clave y sus
              conexiones en red para explorarlo visualmente o exportarlo como PNG.
            </p>

            <div class="form-grid">
              <label class="field">
                <span>Ruta del archivo <code>.canvas</code>:</span>
                <input
                  type="text"
                  bind:value={canvasPath}
                  placeholder="Mapas/Mapa de Conocimiento.canvas"
                />
              </label>

              <label class="field">
                <span>Título del grupo raíz (opcional):</span>
                <input type="text" bind:value={canvasTitle} placeholder="Mapa de Conocimiento PKM" />
              </label>

              <label class="field">
                <span>Máximo de nodos en el lienzo (4 a 40):</span>
                <input type="number" min="4" max="40" bind:value={canvasMaxNodes} />
              </label>
            </div>

            <div class="inline-options">
              <label class="checkbox-inline">
                <input
                  type="checkbox"
                  bind:checked={canvasUseActiveSeed}
                  disabled={!activeNote}
                />
                <span>
                  Usar nota activa como semilla
                  {#if activeNote}
                    (<code>{activeNote.relative_path}</code>)
                  {:else}
                    (sin nota activa)
                  {/if}
                </span>
              </label>

              <label class="checkbox-inline">
                <input type="checkbox" bind:checked={canvasIncludeNeighbors} />
                <span>Incluir notas vecinas conectadas</span>
              </label>
            </div>

            <div class="actions-end">
              <button
                type="button"
                class="btn primary"
                disabled={Boolean(busyMessage)}
                onclick={() => void handleCreateKnowledgeCanvas()}
              >
                ⊞ Generar y Abrir Mapa .canvas
              </button>
            </div>
          </section>
        {:else if activeTab === 'chat'}
          <section class="chat-panel">
            <div class="chat-context-bar">
              <label class="checkbox-inline">
                <input type="checkbox" bind:checked={chatIncludeMap} />
                <span>Incluir mapa compacto de la bóveda</span>
              </label>
              {#if activeNote}
                <span class="chip accent">Nota activa: {activeNote.title}</span>
              {/if}
              {#if chatTurns.length > 0}
                <button
                  type="button"
                  class="pill-btn"
                  title="Borrar historial guardado del chat"
                  onclick={handleClearChat}
                >
                  🗑 Limpiar chat
                </button>
              {/if}
            </div>

            <div class="quick-prompts">
              <button
                type="button"
                class="pill-btn"
                onclick={() =>
                  void handleSendChat(
                    activeNote
                      ? `¿Qué conexiones [[wikilink]], #tags y claves de frontmatter faltan en ${activeNote.relative_path}?`
                      : '¿Qué conexiones faltan entre mis notas principales?',
                  )}
              >
                ¿Qué conexiones faltan en la nota activa?
              </button>
              <button
                type="button"
                class="pill-btn"
                onclick={() =>
                  void handleSendChat('Analiza las notas huérfanas de mi bóveda y sugiere enlaces.')}
              >
                Analizar notas huérfanas
              </button>
              <button
                type="button"
                class="pill-btn"
                onclick={() =>
                  void handleSendChat(
                    'Propón una taxonomía coherente de #tags y campos de frontmatter para mis fuentes.',
                  )}
              >
                Proponer #tags y frontmatter
              </button>
            </div>

            <div class="chat-messages">
              {#if chatTurns.length === 0}
                <div class="empty-box">
                  Pregunta al Copiloto PKM sobre tu bóveda, pide un análisis cruzado de tus notas o
                  solicita ideas de <code>[[wikilinks]]</code>.
                </div>
              {:else}
                {#each chatTurns as turn, idx (turn.id)}
                  <div class="chat-bubble" class:user={turn.role === 'user'}>
                    <div class="chat-meta">
                      <strong>{turn.role === 'user' ? 'Tú' : '✦ Copiloto PKM'}</strong>
                      {#if turn.response}
                        <span class="chip muted">
                          {providerShortLabel(turn.response.provider_used)} · {turn.response.model_used}
                        </span>
                      {/if}
                    </div>

                    {#if turn.role === 'user'}
                      <p class="user-text">{turn.text}</p>
                    {:else}
                      {#if turn.response?.warning}
                        <p class="warn-text">{turn.response.warning}</p>
                      {/if}
                      <div class="chat-md" use:delegateChatLinks>
                        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                        {@html renderMarkdownPreview(turn.text, knownNoteKeys)}
                      </div>

                      {#if turn.response && turn.response.referenced_notes.length > 0}
                        <div class="ref-chips">
                          <span>Notas citadas:</span>
                          {#each turn.response.referenced_notes as refPath (refPath)}
                            <button
                              type="button"
                              class="pill-btn"
                              onclick={() => void onOpenNote(refPath)}
                            >
                              ↗ {refPath}
                            </button>
                          {/each}
                        </div>
                      {/if}

                      {#if turn.response && turn.response.suggested_actions.length > 0}
                        <div class="ref-chips">
                          <span>Acciones sugeridas:</span>
                          {#each turn.response.suggested_actions as actionText (actionText)}
                            <button
                              type="button"
                              class="pill-btn accent"
                              onclick={() => handleSuggestedActionClick(actionText)}
                            >
                              ✦ {actionText}
                            </button>
                          {/each}
                        </div>
                      {/if}

                      {#if turn.savedNotePath}
                        <div class="ref-chips">
                          <span class="chip ok">
                            ✓ Guardado como nota:
                            <button
                              type="button"
                              class="link-btn inline-link"
                              onclick={() => void onOpenNote(turn.savedNotePath!)}
                            >
                              ↗ {turn.savedNotePath}
                            </button>
                          </span>
                        </div>
                      {:else if savingTurnId === turn.id}
                        <div class="turn-save-form">
                          <input
                            type="text"
                            bind:value={savingTurnPath}
                            placeholder="Ruta de la nota (.md)..."
                            onkeydown={(e) => {
                              if (e.key === 'Enter') {
                                e.preventDefault();
                                void handleConfirmSaveAsNote(turn);
                              } else if (e.key === 'Escape') {
                                savingTurnId = null;
                              }
                            }}
                          />
                          <button
                            type="button"
                            class="btn small primary"
                            onclick={() => void handleConfirmSaveAsNote(turn)}
                          >
                            ✓ Crear y abrir
                          </button>
                          <button
                            type="button"
                            class="btn small"
                            onclick={() => (savingTurnId = null)}
                          >
                            Cancelar
                          </button>
                        </div>
                      {/if}

                      <div class="bubble-actions">
                        <button
                          type="button"
                          class="btn small"
                          title="Copiar respuesta al portapapeles"
                          onclick={() => void handleCopyTurnText(turn)}
                        >
                          {copiedTurnId === turn.id ? '✓ Copiado' : '📋 Copiar'}
                        </button>
                        {#if !turn.savedNotePath && savingTurnId !== turn.id}
                          <button
                            type="button"
                            class="btn small primary"
                            title="Crear una nueva nota en la bóveda con esta respuesta"
                            onclick={() => handleStartSaveAsNote(turn, chatTurns[idx - 1])}
                          >
                            📄 Guardar como nota
                          </button>
                        {/if}
                      </div>
                    {/if}
                  </div>
                {/each}
              {/if}
            </div>

            <form
              class="chat-composer"
              onsubmit={(e) => {
                e.preventDefault();
                void handleSendChat();
              }}
            >
              <textarea
                rows="2"
                bind:value={chatInput}
                placeholder="Escribe tu consulta para el Copiloto PKM (Enter para enviar, Shift+Enter nueva línea)..."
                onkeydown={(e) => {
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    void handleSendChat();
                  }
                }}
              ></textarea>
              <button
                type="submit"
                class="btn primary"
                disabled={chatBusy || !chatInput.trim()}
              >
                {chatBusy ? 'Pensando...' : 'Enviar'}
              </button>
            </form>
          </section>
        {:else if activeTab === 'provider'}
          <section class="panel-card">
            <div class="card-head">
              <div>
                <h3>Conectores IA y Diagnóstico del Sistema</h3>
                <small>Plataforma detectada: <strong>{providerStatus?.platform ?? 'cargando'}</strong></small>
              </div>
              <button
                type="button"
                class="btn"
                disabled={loadingStatus}
                onclick={() => void loadInitialContext()}
              >
                {loadingStatus ? 'Verificando...' : '↻ Re-detectar CLI'}
              </button>
            </div>

            {#if providerStatus}
              <div class="status-grid">
                <div class="status-box">
                  <strong>Claude Code CLI</strong>
                  <span class="chip" class:ok={providerStatus.claude_cli_available}>
                    {providerStatus.claude_cli_available ? 'Detectado ✓' : 'No detectado'}
                  </span>
                  {#if providerStatus.claude_cli_path}
                    <code>{providerStatus.claude_cli_path}</code>
                  {:else}
                    <small>Instala y autentica <code>claude</code> para usar tu suscripción de USD 20 sin costo extra.</small>
                  {/if}
                </div>

                <div class="status-box">
                  <strong>Gemini CLI</strong>
                  <span class="chip" class:ok={providerStatus.gemini_cli_available}>
                    {providerStatus.gemini_cli_available ? 'Detectado ✓' : 'No detectado'}
                  </span>
                  {#if providerStatus.gemini_cli_path}
                    <code>{providerStatus.gemini_cli_path}</code>
                  {:else}
                    <small>Opcional en PC mediante <code>gemini</code> CLI autenticado.</small>
                  {/if}
                </div>

                <div class="status-box">
                  <strong>Gemini API (Llavero SO)</strong>
                  <span class="chip" class:ok={providerStatus.has_gemini_api_key}>
                    {providerStatus.has_gemini_api_key ? 'Clave guardada ✓' : 'Sin clave'}
                  </span>
                  <small>Compatible con Windows y Android (Google AI Studio Free Tier).</small>
                </div>

                <div class="status-box">
                  <strong>Compatible HTTP (Llavero SO)</strong>
                  <span class="chip" class:ok={providerStatus.has_compatible_api_key}>
                    {providerStatus.has_compatible_api_key ? 'Clave guardada ✓' : 'Sin clave'}
                  </span>
                  <small>OpenAI, OpenRouter u Ollama local (<code>localhost</code> sin clave).</small>
                </div>
              </div>
            {/if}

            <div class="provider-options">
              <label class="provider-radio" class:selected={cfgProvider === 'deterministic'}>
                <input
                  type="radio"
                  name="ai-provider"
                  checked={cfgProvider === 'deterministic'}
                  onchange={() => handleProviderRadioChange('deterministic')}
                />
                <div>
                  <strong>Motor Determinista Local (Rust + SQLite FTS5)</strong>
                  <small>100% offline, 0 tokens y latencia &lt; 50 ms en Windows y Android.</small>
                </div>
              </label>

              <label class="provider-radio" class:selected={cfgProvider === 'claude_cli'}>
                <input
                  type="radio"
                  name="ai-provider"
                  checked={cfgProvider === 'claude_cli'}
                  onchange={() => handleProviderRadioChange('claude_cli')}
                />
                <div>
                  <strong>Claude Code CLI (<code>claude -p</code>)</strong>
                  <small>Usa tu suscripción de Claude (USD 20) directamente en PC Windows sin facturación por token.</small>
                </div>
              </label>

              <label class="provider-radio" class:selected={cfgProvider === 'gemini_cli'}>
                <input
                  type="radio"
                  name="ai-provider"
                  checked={cfgProvider === 'gemini_cli'}
                  onchange={() => handleProviderRadioChange('gemini_cli')}
                />
                <div>
                  <strong>Gemini CLI (<code>gemini -p</code>)</strong>
                  <small>Invoca el CLI oficial de Google Gemini desde tu cuenta local en PC.</small>
                </div>
              </label>

              <label class="provider-radio" class:selected={cfgProvider === 'gemini_http'}>
                <input
                  type="radio"
                  name="ai-provider"
                  checked={cfgProvider === 'gemini_http'}
                  onchange={() => handleProviderRadioChange('gemini_http')}
                />
                <div>
                  <strong>Google Gemini API HTTP (PC + Android)</strong>
                  <small>Funciona en Samsung A34 y Windows usando HTTPS <code>rustls</code> + clave de AI Studio.</small>
                </div>
              </label>

              <label class="provider-radio" class:selected={cfgProvider === 'compatible_http'}>
                <input
                  type="radio"
                  name="ai-provider"
                  checked={cfgProvider === 'compatible_http'}
                  onchange={() => handleProviderRadioChange('compatible_http')}
                />
                <div>
                  <strong>Endpoint Compatible OpenAI / Ollama HTTP</strong>
                  <small>Para Ollama local o cualquier endpoint <code>/v1/chat/completions</code>.</small>
                </div>
              </label>
            </div>

            <div class="form-grid">
              <label class="field">
                <span>Modelo activo:</span>
                <input type="text" bind:value={cfgModel} placeholder={DEFAULT_MODELS[cfgProvider]} />
              </label>

              <label class="field">
                <span>Máx. conexiones por nota (1-25):</span>
                <input type="number" min="1" max="25" bind:value={cfgMaxConnections} />
              </label>

              {#if cfgProvider === 'gemini_http' || cfgProvider === 'compatible_http'}
                <label class="field span-2">
                  <span>URL de endpoint personalizado (opcional):</span>
                  <input
                    type="text"
                    bind:value={cfgEndpointUrl}
                    placeholder={cfgProvider === 'gemini_http'
                      ? 'Por defecto: https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent'
                      : 'Por defecto: http://127.0.0.1:11434/v1/chat/completions'}
                  />
                </label>
              {/if}

              {#if cfgProvider === 'claude_cli' || cfgProvider === 'gemini_cli'}
                <label class="field span-2">
                  <span>Ruta manual del binario CLI (opcional):</span>
                  <input
                    type="text"
                    bind:value={cfgCliBinary}
                    placeholder="Déjalo vacío para autodetección en PATH / npm / .local"
                  />
                </label>
              {/if}
            </div>

            <div class="model-presets">
              <span>Modelos rápidos:</span>
              {#each ['claude-sonnet-4-5', 'gemini-2.5-flash', 'gemini-2.5-pro', 'gpt-4o-mini', 'llama3.2'] as preset (preset)}
                <button type="button" class="pill-btn" onclick={() => (cfgModel = preset)}>
                  {preset}
                </button>
              {/each}
            </div>

            <div class="inline-options">
              <label class="checkbox-inline">
                <input type="checkbox" bind:checked={cfgAppendRelated} />
                <span>Añadir sección <code>## Conexiones Relacionadas</code> por defecto</span>
              </label>
              <label class="checkbox-inline">
                <input type="checkbox" bind:checked={cfgFallbackDeterministic} />
                <span>Degradar al Motor Local si el CLI/HTTP falla</span>
              </label>
            </div>

            <div class="credentials-box">
              <h4>Claves API en Almacén Seguro del SO (nunca se guardan en la bóveda)</h4>
              <div class="form-grid">
                <div class="field">
                  <label>
                    <span>API Key Google Gemini ({providerStatus?.has_gemini_api_key ? 'guardada ✓' : 'vacía'}):</span>
                    <input
                      type="password"
                      autocomplete="off"
                      bind:value={geminiApiKeyInput}
                      placeholder={providerStatus?.has_gemini_api_key
                        ? '•••••••• (deja vacío para conservar)'
                        : 'AIza...'}
                    />
                  </label>
                  {#if providerStatus?.has_gemini_api_key}
                    <button
                      type="button"
                      class="btn small"
                      disabled={savingConfig}
                      onclick={() => void handleSaveProviderConfig(true, false)}
                    >
                      Borrar clave Gemini del SO
                    </button>
                  {/if}
                </div>

                <div class="field">
                  <label>
                    <span>API Key Compatible / OpenAI ({providerStatus?.has_compatible_api_key ? 'guardada ✓' : 'vacía'}):</span>
                    <input
                      type="password"
                      autocomplete="off"
                      bind:value={compatibleApiKeyInput}
                      placeholder={providerStatus?.has_compatible_api_key
                        ? '•••••••• (deja vacío para conservar)'
                        : 'sk-...'}
                    />
                  </label>
                  {#if providerStatus?.has_compatible_api_key}
                    <button
                      type="button"
                      class="btn small"
                      disabled={savingConfig}
                      onclick={() => void handleSaveProviderConfig(false, true)}
                    >
                      Borrar clave Compatible del SO
                    </button>
                  {/if}
                </div>
              </div>
            </div>

            <div class="actions-end">
              <button
                type="button"
                class="btn primary"
                disabled={savingConfig}
                onclick={() => void handleSaveProviderConfig(false, false)}
              >
                {savingConfig ? 'Guardando...' : '💾 Guardar Configuración de IA'}
              </button>
            </div>
          </section>

          <section class="panel-card">
            <div class="card-head">
              <div>
                <h4>Servidor MCP Nativo (<code>romsidian-mcp</code>)</h4>
                <small>
                  Conecta tu bóveda por <code>stdio</code> JSON-RPC 2.0 a Claude Desktop, Claude
                  Code o Antigravity sin gastar tokens enviando notas completas.
                </small>
              </div>
              <button type="button" class="btn small" onclick={() => void handleCopyMcpConfig()}>
                Copiar JSON MCP
              </button>
            </div>
            {#if copiedMcpNotice}
              <p class="ok-text">{copiedMcpNotice}</p>
            {/if}
            <pre class="mcp-code">{mcpConfigSnippet}</pre>
          </section>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .copilot-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    background: var(--bg-backdrop);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 14px;
  }

  .copilot-modal {
    width: min(980px, 100%);
    max-height: min(92vh, 880px);
    display: flex;
    flex-direction: column;
    background: var(--bg-modal);
    color: var(--text-primary);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }

  .copilot-header {
    padding: 14px 16px 10px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-sidebar);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .header-title-row {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }

  .header-title-row h2 {
    margin: 0;
    font-size: 16px;
  }

  .subtitle {
    margin: 2px 0 0;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .header-right-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .provider-pill {
    padding: 4px 10px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent);
    border: 1px solid var(--border-strong);
    font-size: 12px;
    font-weight: 600;
  }

  .provider-pill small {
    font-weight: 400;
    color: var(--text-secondary);
  }

  .toolbar-strip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .tabs-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .tab-btn {
    padding: 6px 11px;
    border-radius: 7px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: 12.5px;
    cursor: pointer;
  }

  .tab-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: var(--accent);
    font-weight: 600;
  }

  .safety-toggle {
    display: inline-flex;
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    overflow: hidden;
    background: var(--bg-surface);
  }

  .safety-btn {
    padding: 5px 10px;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
  }

  .safety-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .safety-btn.auto.active {
    background: var(--pending-soft);
    color: var(--pending);
  }

  .alert {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 16px;
    font-size: 12.5px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .alert.error {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .alert.ok {
    background: var(--success-soft);
    color: var(--success);
  }

  .alert.info {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .alert button {
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-weight: 700;
  }

  .copilot-body {
    padding: 14px 16px 16px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
    flex: 1;
    min-height: 0;
  }

  .vault-map-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .panel-card {
    padding: 12px 14px;
    border-radius: 9px;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .panel-card.highlight {
    border-color: var(--accent);
  }

  .panel-card h3,
  .panel-card h4 {
    margin: 0;
    font-size: 14px;
  }

  .card-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .card-head small {
    color: var(--text-muted);
    font-size: 12px;
  }

  .muted-note {
    margin: 0;
    color: var(--text-secondary);
    font-size: 12.5px;
  }

  .warn-text {
    margin: 4px 0 0;
    color: var(--pending);
    font-size: 12px;
  }

  .ok-text {
    margin: 0;
    color: var(--success);
    font-size: 12px;
  }

  .empty-box {
    padding: 20px;
    border-radius: 9px;
    border: 1px dashed var(--border-strong);
    color: var(--text-secondary);
    text-align: center;
    font-size: 13px;
  }

  .inline-options {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 14px;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .inline-options input[type='number'] {
    width: 68px;
    padding: 4px 7px;
    margin-left: 6px;
    border-radius: 6px;
    border: 1px solid var(--border-strong);
    background: var(--bg-editor);
    color: var(--text-primary);
  }

  .checkbox-inline {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    user-select: none;
  }

  .btn {
    padding: 6px 12px;
    border-radius: 7px;
    border: 1px solid var(--border-strong);
    background: var(--bg-surface-hover);
    color: var(--text-primary);
    font-size: 12.5px;
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .btn:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .btn.primary {
    background: var(--accent);
    color: #ffffff;
    border-color: var(--accent);
    font-weight: 600;
  }

  .btn.small {
    padding: 4px 9px;
    font-size: 11.5px;
  }

  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    font-weight: 600;
    padding: 0;
    cursor: pointer;
    text-align: left;
    font-size: 13px;
  }

  .link-btn:hover {
    text-decoration: underline;
  }

  .candidate-list,
  .batch-items {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .candidate-item,
  .batch-item {
    padding: 9px 11px;
    border-radius: 8px;
    background: var(--bg-editor);
    border: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .batch-item.changed {
    border-left: 3px solid var(--accent);
  }

  .candidate-top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .path-label {
    margin-left: 6px;
    font-size: 11.5px;
    color: var(--text-muted);
  }

  .candidate-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .reason-line {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .signal-pills,
  .diff-badges,
  .folder-pills,
  .quick-prompts,
  .model-presets,
  .ref-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .signal-pill {
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 5px;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    color: var(--text-secondary);
  }

  .signal-pill.tag {
    color: var(--accent);
    background: var(--accent-soft);
    border-color: transparent;
  }

  .chip {
    font-size: 11.5px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--bg-surface-hover);
    color: var(--text-secondary);
  }

  .chip.ok {
    background: var(--success-soft);
    color: var(--success);
  }

  .chip.warn {
    background: var(--pending-soft);
    color: var(--pending);
  }

  .chip.accent {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .diff-summary {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .diff-code,
  .mcp-code {
    margin: 0;
    padding: 9px 11px;
    border-radius: 7px;
    background: var(--code-bg);
    border: 1px solid var(--border-subtle);
    font-family: 'Cascadia Code', 'Fira Code', ui-monospace, monospace;
    font-size: 11.5px;
    line-height: 1.45;
    max-height: 240px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .diff-line.add {
    background: var(--success-soft);
    color: var(--success);
  }

  .diff-line.del {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .diff-line.hunk {
    color: var(--accent);
    font-weight: 600;
  }

  .batch-controls {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .field,
  .field label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .field input,
  .chat-composer textarea {
    width: 100%;
    padding: 7px 10px;
    border-radius: 7px;
    border: 1px solid var(--border-strong);
    background: var(--bg-editor);
    color: var(--text-primary);
    font: inherit;
  }

  .pill-btn {
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    color: var(--text-secondary);
    font-size: 11.5px;
    cursor: pointer;
  }

  .pill-btn.active,
  .pill-btn.accent {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .form-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }

  .span-2 {
    grid-column: span 2;
  }

  .actions-end {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .chat-panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex: 1;
    min-height: 360px;
  }

  .chat-context-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .chat-messages {
    flex: 1;
    min-height: 200px;
    max-height: 360px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 4px 2px;
  }

  .chat-bubble {
    padding: 10px 12px;
    border-radius: 9px;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .chat-bubble.user {
    background: var(--accent-soft);
    border-color: var(--accent);
  }

  .chat-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    font-size: 12px;
  }

  .user-text {
    margin: 0;
    white-space: pre-wrap;
    font-size: 13px;
  }

  .chat-md {
    font-size: 13px;
    line-height: 1.55;
  }

  .chat-md :global(p) {
    margin: 0 0 8px;
  }

  .chat-md :global(p:last-child) {
    margin-bottom: 0;
  }

  .chat-md :global(button[data-wikilink]) {
    border: none;
    background: var(--accent-soft);
    color: var(--accent);
    padding: 1px 6px;
    border-radius: 4px;
    cursor: pointer;
    font: inherit;
    font-weight: 600;
  }

  .chat-composer {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }

  .chat-composer textarea {
    flex: 1;
    resize: vertical;
  }

  .bubble-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border-subtle);
  }

  .turn-save-form {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
    padding: 8px;
    background: var(--bg-surface-hover);
    border: 1px solid var(--border-subtle);
    border-radius: 6px;
  }

  .turn-save-form input {
    flex: 1;
    font-size: 12px;
    padding: 5px 8px;
    border-radius: 4px;
    background: var(--bg-editor);
    color: var(--text-primary);
    border: 1px solid var(--border-subtle);
  }

  .inline-link {
    display: inline;
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
    font-weight: 500;
  }

  .status-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }

  .status-box {
    padding: 9px 11px;
    border-radius: 8px;
    background: var(--bg-editor);
    border: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }

  .status-box code {
    word-break: break-all;
    font-size: 11px;
    color: var(--text-muted);
  }

  .provider-options {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .provider-radio {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 9px 11px;
    border-radius: 8px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    cursor: pointer;
    font-size: 12.5px;
  }

  .provider-radio.selected {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .provider-radio small {
    display: block;
    color: var(--text-secondary);
    margin-top: 2px;
  }

  .credentials-box {
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .credentials-box h4 {
    margin: 0;
    font-size: 13px;
  }

  .credentials-box .btn.small {
    align-self: flex-start;
    margin-top: 4px;
  }

  @media (max-width: 720px) {
    .copilot-backdrop {
      padding: 6px;
    }

    .form-grid,
    .status-grid {
      grid-template-columns: 1fr;
    }

    .span-2 {
      grid-column: span 1;
    }

    .chat-composer {
      flex-direction: column;
      align-items: stretch;
    }
  }
</style>
