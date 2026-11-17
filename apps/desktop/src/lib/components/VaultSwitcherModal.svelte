<script lang="ts">
  import {
    createLocalVault,
    isWebBrowserMode,
    listGithubRepositoriesForWeb,
    logoutAndClearWebSession,
    openVault,
    saveSyncConfig,
    syncVaultNow,
  } from '../api';
  import type { GithubRepoInfo, LocalVaultEntry } from '../types';

  interface Props {
    open: boolean;
    activeVaultPath: string | null;
    activeVaultName: string | null;
    vaults: LocalVaultEntry[];
    loading: boolean;
    errorMsg: string | null;
    onClose: () => void;
    onSelectVault: (path: string) => Promise<void>;
    onCreateVault: (name: string) => Promise<void>;
    onPickFolder: () => Promise<void>;
    onRemoveRecent: (path: string) => void;
    onOpenSync: () => void;
  }

  let {
    open,
    activeVaultPath,
    activeVaultName,
    vaults,
    loading,
    errorMsg,
    onClose,
    onSelectVault,
    onCreateVault,
    onPickFolder,
    onRemoveRecent,
    onOpenSync,
  }: Props = $props();

  let newVaultName = $state('');
  let manualPath = $state('');
  let showManualPath = $state(false);
  let creating = $state(false);

  const isWeb = isWebBrowserMode();
  const isAndroid =
    typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent);

  let webPatInput = $state('');
  let webRepos = $state<GithubRepoInfo[]>([]);
  let selectedRepoFullName = $state('');
  let loadingRepos = $state(false);
  let connectingRepo = $state(false);
  let webNotice = $state<string | null>(null);
  let webError = $state<string | null>(null);

  function normalizePath(p: string | null | undefined): string {
    return (p ?? '').trim().replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  }

  async function handleFetchGithubRepos() {
    webError = null;
    webNotice = null;
    const token = webPatInput.trim();
    if (!token) {
      webError = 'Pegá tu Personal Access Token (PAT) de GitHub primero.';
      return;
    }
    loadingRepos = true;
    try {
      const list = await listGithubRepositoriesForWeb(token);
      webRepos = list;
      if (list.length > 0) {
        selectedRepoFullName = list[0].full_name;
        webNotice = `${list.length} repositorio(s) encontrados en tu cuenta de GitHub.`;
      } else {
        webError = 'No se encontraron repositorios accesibles con este token.';
      }
    } catch (err) {
      webError = err instanceof Error ? err.message : String(err);
    } finally {
      loadingRepos = false;
    }
  }

  async function handleConnectGithubRepo() {
    webError = null;
    webNotice = null;
    const token = webPatInput.trim();
    const repo = webRepos.find((r) => r.full_name === selectedRepoFullName);
    if (!token || !repo) {
      webError = 'Seleccioná un repositorio y verificá tu token PAT.';
      return;
    }
    connectingRepo = true;
    try {
      const entry = await createLocalVault(repo.name);
      await openVault(entry.path);
      await saveSyncConfig(
        {
          remote_url: repo.clone_url,
          branch: repo.default_branch || 'main',
          device_name: 'web-browser',
          author_name: repo.owner || 'Roma (Web)',
          author_email: 'user@romsidian.local',
        },
        token,
        false,
      );
      await syncVaultNow();
      await onSelectVault(entry.path);
      webPatInput = '';
    } catch (err) {
      webError = err instanceof Error ? err.message : String(err);
    } finally {
      connectingRepo = false;
    }
  }

  async function handleWebLogout() {
    const snap = await logoutAndClearWebSession();
    webPatInput = '';
    webRepos = [];
    selectedRepoFullName = '';
    webNotice = 'Sesión web cerrada y sessionStorage vaciado.';
    await onSelectVault(snap.info.root_path);
  }

  async function handleCreateSubmit(e: SubmitEvent) {
    e.preventDefault();
    const trimmed = newVaultName.trim();
    if (!trimmed || creating) return;
    creating = true;
    try {
      await onCreateVault(trimmed);
      newVaultName = '';
    } finally {
      creating = false;
    }
  }

  async function handleManualSubmit(e: SubmitEvent) {
    e.preventDefault();
    const trimmed = manualPath.trim();
    if (!trimmed) return;
    await onSelectVault(trimmed);
    manualPath = '';
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="modal-backdrop"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div class="vault-modal" role="dialog" aria-modal="true" aria-label="Cambiar de bóveda">
      <div class="modal-head">
        <div>
          <h3>🗂 Gestor de Bóvedas</h3>
          <p class="subtitle">
            Cambiá entre tus bóvedas o creá una nueva para sincronizarla con otro repositorio de GitHub.
          </p>
        </div>
        <button type="button" class="close-btn" onclick={onClose} aria-label="Cerrar">✕</button>
      </div>

      {#if errorMsg}
        <div class="modal-error">{errorMsg}</div>
      {/if}

      {#if isWeb}
        <div class="web-session-banner">
          <div>
            <strong>🌐 Modo Web (sessionStorage)</strong>
            <p>
              Tus notas, lienzos y token PAT viven únicamente en esta pestaña y se borran solos al cerrarla.
            </p>
          </div>
          <button
            type="button"
            class="btn-danger-outline"
            onclick={() => void handleWebLogout()}
            title="Vaciar sessionStorage y cerrar sesión web"
          >
            🔒 Cerrar sesión web
          </button>
        </div>
      {/if}

      {#if activeVaultPath}
        <div class="active-card">
          <div class="active-meta">
            <span class="badge-active">Bóveda activa</span>
            <strong>{activeVaultName || 'Bóveda'}</strong>
            <span class="vault-path">{activeVaultPath}</span>
          </div>
          <button
            type="button"
            class="btn-secondary"
            onclick={() => {
              onClose();
              onOpenSync();
            }}
          >
            🔄 Sync Git
          </button>
        </div>
      {/if}

      {#if isWeb}
        <section class="section web-github-box">
          <h4>☁️ Conectar repositorio de GitHub (1 clic)</h4>
          <p class="hint">
            Pegá tu Personal Access Token (PAT) de GitHub para listar tus repositorios privados y abrir cualquier bóveda al instante.
          </p>
          <div class="inline-form">
            <input
              type="password"
              autocomplete="off"
              bind:value={webPatInput}
              placeholder="github_pat_... o ghp_..."
              aria-label="Token PAT de GitHub"
              disabled={loadingRepos || connectingRepo}
            />
            <button
              type="button"
              class="btn-secondary"
              disabled={!webPatInput.trim() || loadingRepos || connectingRepo}
              onclick={() => void handleFetchGithubRepos()}
            >
              {loadingRepos ? 'Buscando...' : 'Listar mis repos'}
            </button>
          </div>

          {#if webError}
            <div class="modal-error">{webError}</div>
          {/if}
          {#if webNotice}
            <div class="web-notice">{webNotice}</div>
          {/if}

          {#if webRepos.length > 0}
            <div class="inline-form">
              <select
                class="repo-select"
                bind:value={selectedRepoFullName}
                aria-label="Seleccionar repositorio de GitHub"
                disabled={connectingRepo}
              >
                {#each webRepos as repo (repo.full_name)}
                  <option value={repo.full_name}>
                    {repo.private ? '🔒' : '🌐'} {repo.full_name} ({repo.default_branch})
                  </option>
                {/each}
              </select>
              <button
                type="button"
                class="btn-primary"
                disabled={!selectedRepoFullName || connectingRepo}
                onclick={() => void handleConnectGithubRepo()}
              >
                {connectingRepo ? 'Descargando...' : 'Abrir y sincronizar'}
              </button>
            </div>
          {/if}
        </section>
      {/if}

      <section class="section">
        <div class="section-head">
          <h4>Bóvedas disponibles ({vaults.length})</h4>
          {#if loading}
            <span class="muted">Cargando...</span>
          {/if}
        </div>

        {#if vaults.length === 0}
          <p class="empty-msg">Todavía no hay bóvedas registradas.</p>
        {:else}
          <ul class="vault-list">
            {#each vaults as vault (vault.path)}
              {@const isCurrent = normalizePath(vault.path) === normalizePath(activeVaultPath)}
              <li class="vault-item" class:current={isCurrent}>
                <button
                  type="button"
                  class="vault-select-btn"
                  disabled={isCurrent || loading}
                  onclick={() => void onSelectVault(vault.path)}
                >
                  <span class="vault-title-row">
                    <span class="vault-name">{vault.name}</span>
                    {#if vault.is_default}
                      <span class="tag-pill">Inicial</span>
                    {/if}
                    {#if isCurrent}
                      <span class="tag-pill accent">Abierta</span>
                    {/if}
                  </span>
                  <span class="vault-path">{vault.path}</span>
                </button>
                {#if !isCurrent && !vault.is_default}
                  <button
                    type="button"
                    class="remove-recent-btn"
                    title="Quitar de la lista reciente (no borra archivos)"
                    onclick={() => onRemoveRecent(vault.path)}
                  >
                    ✕
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="section">
        <h4>+ Crear nueva bóveda</h4>
        <form class="inline-form" onsubmit={handleCreateSubmit}>
          <input
            type="text"
            bind:value={newVaultName}
            placeholder="Nombre (ej. Trabajo, Facultad...)"
            aria-label="Nombre de la nueva bóveda"
            disabled={creating}
          />
          <button type="submit" class="btn-primary" disabled={!newVaultName.trim() || creating}>
            {creating ? 'Creando...' : 'Crear y abrir'}
          </button>
        </form>
        <p class="hint">
          Después de crearla y abrirla, entrá a <strong>🔄 Sync</strong> para conectar su repositorio y Token PAT de GitHub y descargar tus notas.
        </p>
      </section>

      {#if !isWeb}
        <div class="footer-actions">
          {#if !isAndroid}
            <button type="button" class="btn-secondary" onclick={() => void onPickFolder()}>
              📁 Explorar carpeta en PC...
            </button>
          {/if}
          <button
            type="button"
            class="btn-link"
            onclick={() => (showManualPath = !showManualPath)}
          >
            {showManualPath ? 'Ocultar ruta manual' : 'Abrir por ruta manual'}
          </button>
        </div>

        {#if showManualPath}
          <form class="inline-form manual-form" onsubmit={handleManualSubmit}>
            <input
              type="text"
              bind:value={manualPath}
              placeholder="Ruta completa de la carpeta..."
              aria-label="Ruta completa de la bóveda"
            />
            <button type="submit" class="btn-secondary" disabled={!manualPath.trim()}>
              Abrir
            </button>
          </form>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(15, 23, 42, 0.55);
    backdrop-filter: blur(2px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 14px;
    z-index: 90;
  }

  .vault-modal {
    width: min(520px, 100%);
    max-height: 90vh;
    overflow-y: auto;
    background: var(--bg-panel);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px;
    box-shadow: var(--shadow-lg);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .modal-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .modal-head h3 {
    margin: 0;
    font-size: 1.05rem;
  }

  .subtitle {
    margin: 4px 0 0;
    font-size: 0.82rem;
    color: var(--text-secondary);
  }

  .close-btn {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-secondary);
    border-radius: 6px;
    width: 30px;
    height: 30px;
    cursor: pointer;
    flex-shrink: 0;
  }

  .modal-error {
    background: rgba(220, 38, 38, 0.12);
    border: 1px solid rgba(220, 38, 38, 0.35);
    color: var(--danger);
    padding: 8px 10px;
    border-radius: 8px;
    font-size: 0.83rem;
  }

  .active-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--bg-app);
    border: 1px solid var(--accent);
  }

  .active-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .badge-active {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--accent);
    font-weight: 700;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .section h4 {
    margin: 0;
    font-size: 0.86rem;
    color: var(--text-secondary);
  }

  .muted,
  .empty-msg,
  .hint {
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-secondary);
  }

  .vault-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 220px;
    overflow-y: auto;
  }

  .vault-item {
    display: flex;
    align-items: stretch;
    gap: 6px;
  }

  .vault-select-btn {
    flex: 1;
    min-width: 0;
    text-align: left;
    padding: 9px 11px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg-app);
    color: var(--text-primary);
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .vault-select-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .vault-item.current .vault-select-btn {
    border-color: var(--accent);
    opacity: 0.9;
    cursor: default;
  }

  .vault-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .vault-name {
    font-weight: 600;
    font-size: 0.9rem;
  }

  .tag-pill {
    font-size: 0.68rem;
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-secondary);
  }

  .tag-pill.accent {
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .vault-path {
    font-size: 0.74rem;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .remove-recent-btn {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-secondary);
    border-radius: 8px;
    padding: 0 10px;
    cursor: pointer;
  }

  .inline-form {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .inline-form input {
    flex: 1;
    min-width: 170px;
    padding: 8px 10px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--bg-app);
    color: var(--text-primary);
    font-size: 0.88rem;
  }

  .btn-primary,
  .btn-secondary {
    padding: 8px 12px;
    border-radius: 7px;
    font-size: 0.84rem;
    cursor: pointer;
    border: 1px solid var(--border);
  }

  .btn-primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
    font-weight: 600;
  }

  .btn-primary:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .btn-secondary {
    background: var(--bg-app);
    color: var(--text-primary);
  }

  .footer-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
    padding-top: 6px;
    border-top: 1px solid var(--border);
  }

  .btn-link {
    border: none;
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    font-size: 0.8rem;
    padding: 4px 0;
  }

  .web-session-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 8px;
    background: rgba(59, 130, 246, 0.1);
    border: 1px solid rgba(59, 130, 246, 0.35);
    font-size: 0.8rem;
  }

  .web-session-banner p {
    margin: 2px 0 0;
    color: var(--text-secondary);
    font-size: 0.76rem;
  }

  .btn-danger-outline {
    padding: 6px 10px;
    border-radius: 7px;
    font-size: 0.78rem;
    cursor: pointer;
    border: 1px solid rgba(220, 38, 38, 0.45);
    background: rgba(220, 38, 38, 0.1);
    color: var(--danger);
    white-space: nowrap;
    flex-shrink: 0;
  }

  .web-github-box {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 12px;
    background: var(--bg-app);
  }

  .repo-select {
    flex: 1;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: var(--bg-panel);
    color: var(--text-primary);
    font-size: 0.85rem;
    min-width: 0;
  }

  .web-notice {
    font-size: 0.78rem;
    color: var(--accent);
    font-weight: 500;
  }
</style>
