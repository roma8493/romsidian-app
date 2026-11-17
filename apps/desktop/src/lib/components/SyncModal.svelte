<script lang="ts">
  import { isWebBrowserMode, listGithubRepositoriesForWeb } from '../api';
  import type { GithubRepoInfo, SyncConfig, SyncReport, SyncState, SyncStatusDto } from '../types';

  interface Props {
    open: boolean;
    syncDto: SyncStatusDto | null;
    lastReport: SyncReport | null;
    loading: boolean;
    syncing: boolean;
    errorMsg: string | null;
    onClose: () => void;
    onRefreshStatus: () => Promise<void>;
    onSaveConfig: (
      config: SyncConfig,
      patToken?: string,
      clearToken?: boolean,
    ) => Promise<boolean>;
    onSyncNow: () => Promise<void>;
    onOpenConflictFile?: (relativePath: string) => void;
  }

  let {
    open,
    syncDto,
    lastReport,
    loading,
    syncing,
    errorMsg,
    onClose,
    onRefreshStatus,
    onSaveConfig,
    onSyncNow,
    onOpenConflictFile,
  }: Props = $props();

  let remoteUrl = $state('');
  let branch = $state('main');
  let deviceName = $state('pc-windows');
  let authorName = $state('ROMSIDIAN');
  let authorEmail = $state('romsidian@local.invalid');
  let patToken = $state('');
  let saveFeedback = $state<string | null>(null);
  const isWeb = isWebBrowserMode();
  let discoveredRepos = $state<GithubRepoInfo[]>([]);
  let loadingRepos = $state(false);

  async function handleDiscoverRepos() {
    saveFeedback = null;
    const token = patToken.trim();
    if (!token) {
      saveFeedback = 'Ingresá tu token PAT abajo para listar tus repositorios.';
      return;
    }
    loadingRepos = true;
    try {
      const list = await listGithubRepositoriesForWeb(token);
      discoveredRepos = list;
      if (list.length > 0 && !remoteUrl.trim()) {
        remoteUrl = list[0].clone_url;
        branch = list[0].default_branch || 'main';
      }
      saveFeedback = `${list.length} repositorio(s) de GitHub cargados.`;
    } catch (err) {
      saveFeedback = err instanceof Error ? err.message : String(err);
    } finally {
      loadingRepos = false;
    }
  }

  $effect(() => {
    if (open && syncDto) {
      remoteUrl = syncDto.config.remote_url;
      branch = syncDto.config.branch;
      deviceName = syncDto.config.device_name;
      authorName = syncDto.config.author_name;
      authorEmail = syncDto.config.author_email;
      saveFeedback = null;
    }
  });

  function describeState(state: SyncState | undefined): {
    label: string;
    tone: 'neutral' | 'ok' | 'warn' | 'err';
  } {
    if (!state) return { label: 'Sin inicializar', tone: 'neutral' };
    if (state === 'Idle') return { label: 'Listo', tone: 'neutral' };
    if (state === 'Syncing') return { label: 'Sincronizando...', tone: 'neutral' };
    if (state === 'UpToDate') return { label: 'Actualizado', tone: 'ok' };
    if (typeof state === 'object' && 'ConflictDetected' in state) {
      return {
        label: `Conflicto resuelto (${state.ConflictDetected.conflict_files.length} copia/s)`,
        tone: 'warn',
      };
    }
    if (typeof state === 'object' && 'Error' in state) {
      return { label: `Error: ${state.Error.message}`, tone: 'err' };
    }
    return { label: 'Desconocido', tone: 'neutral' };
  }

  function buildConfig(): SyncConfig {
    return {
      remote_url: remoteUrl.trim(),
      branch: branch.trim() || 'main',
      device_name: deviceName.trim() || 'pc-windows',
      author_name: authorName.trim() || 'ROMSIDIAN',
      author_email: authorEmail.trim() || 'romsidian@local.invalid',
    };
  }

  async function handleSaveSubmit(event: SubmitEvent) {
    event.preventDefault();
    saveFeedback = null;
    const token = patToken.trim();
    const ok = await onSaveConfig(buildConfig(), token ? token : undefined, false);
    if (ok) {
      patToken = '';
      saveFeedback = isWeb
        ? 'Configuración guardada en sessionStorage (se borrará automáticamente al cerrar la pestaña).'
        : 'Configuración guardada (el token PAT se almacena solo en el llavero del SO).';
    }
  }

  async function handleClearToken() {
    saveFeedback = null;
    const ok = await onSaveConfig(buildConfig(), undefined, true);
    if (ok) {
      patToken = '';
      saveFeedback = isWeb
        ? 'Token eliminado de sessionStorage.'
        : 'Token eliminado del llavero del sistema operativo.';
    }
  }

  async function handleSyncClick() {
    saveFeedback = null;
    const token = patToken.trim();
    const cfg = buildConfig();
    const configChanged =
      !syncDto ||
      cfg.remote_url !== syncDto.config.remote_url ||
      cfg.branch !== syncDto.config.branch ||
      cfg.device_name !== syncDto.config.device_name ||
      cfg.author_name !== syncDto.config.author_name ||
      cfg.author_email !== syncDto.config.author_email ||
      token.length > 0;

    if (configChanged) {
      const ok = await onSaveConfig(cfg, token ? token : undefined, false);
      if (!ok) return;
      patToken = '';
    }
    await onSyncNow();
  }

  function handleWindowKeyDown(event: KeyboardEvent) {
    if (open && event.key === 'Escape') {
      event.preventDefault();
      onClose();
    }
  }

  let stateBadge = $derived(describeState(lastReport?.state ?? syncDto?.status.state));
</script>

<svelte:window onkeydown={handleWindowKeyDown} />

{#if open}
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}
  >
    <div
      class="sync-dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Sincronización Git de la bóveda"
      tabindex="-1"
    >
      <header class="sync-header">
        <div>
          <h2>Sincronización Git (PC ↔ Android)</h2>
          <p class="sync-subtitle">
            Los archivos <code>.md</code> se sincronizan contra GitHub privado sin guardar
            credenciales dentro de la bóveda.
          </p>
        </div>
        <button type="button" class="close-btn" onclick={onClose} aria-label="Cerrar modal">
          ✕
        </button>
      </header>

      <div class="sync-body">
        <section class="status-card">
          <div class="status-top">
            <span class={`state-pill tone-${stateBadge.tone}`}>{stateBadge.label}</span>
            <button
              type="button"
              class="mini-btn"
              onclick={() => void onRefreshStatus()}
              disabled={loading || syncing}
            >
              {loading ? 'Consultando...' : '↻ Actualizar estado'}
            </button>
          </div>

          {#if syncDto}
            <div class="status-grid">
              <div>
                <span class="meta-label">Rama activa</span>
                <strong>{syncDto.status.branch}</strong>
              </div>
              <div>
                <span class="meta-label">Último commit</span>
                <strong>
                  {syncDto.status.last_commit_id
                    ? syncDto.status.last_commit_id.slice(0, 8)
                    : 'Sin commits aún'}
                </strong>
              </div>
              <div>
                <span class="meta-label">Cambios pendientes</span>
                <strong>{syncDto.status.modified_files.length} archivo(s)</strong>
              </div>
              <div>
                <span class="meta-label">
                  {isWeb ? 'Credencial PAT (sessionStorage)' : 'Credencial PAT (SO)'}
                </span>
                <strong class={syncDto.has_pat_token ? 'text-ok' : 'text-muted'}>
                  {syncDto.has_pat_token
                    ? isWeb
                      ? 'Activo en esta pestaña'
                      : 'Configurado en llavero'
                    : 'No configurado'}
                </strong>
              </div>
            </div>

            {#if syncDto.status.modified_files.length > 0}
              <div class="file-chips">
                <span class="meta-label">Modificados sin sincronizar:</span>
                <ul>
                  {#each syncDto.status.modified_files as file}
                    <li><code>{file}</code></li>
                  {/each}
                </ul>
              </div>
            {/if}
          {/if}
        </section>

        {#if errorMsg}
          <div class="alert error" role="alert">{errorMsg}</div>
        {/if}

        {#if saveFeedback}
          <div class="alert ok" role="status">{saveFeedback}</div>
        {/if}

        {#if lastReport}
          <section class="report-card">
            <h3>Última sincronización</h3>
            <p class="report-summary">
              Confirmados localmente: <strong>{lastReport.committed_files}</strong> · Fusión limpia:
              <strong>{lastReport.merged_cleanly.length}</strong> · Push remoto:
              <strong>{lastReport.pushed ? 'Sí' : 'No / solo local'}</strong>
            </p>

            {#if lastReport.conflict_copies_created.length > 0}
              <div class="alert warn" role="alert">
                <strong>
                  Se detectó colisión en las mismas líneas. Se crearon {lastReport
                    .conflict_copies_created.length} copia(s) de conflicto sin pérdida de datos:
                </strong>
                <ul class="conflict-list">
                  {#each lastReport.conflict_copies_created as copyPath}
                    <li>
                      <code>{copyPath}</code>
                      {#if onOpenConflictFile}
                        <button
                          type="button"
                          class="mini-btn"
                          onclick={() => {
                            onOpenConflictFile(copyPath);
                            onClose();
                          }}
                        >
                          Abrir copia
                        </button>
                      {/if}
                    </li>
                  {/each}
                </ul>
              </div>
            {/if}
          </section>
        {/if}

        <form class="sync-form" onsubmit={handleSaveSubmit}>
          <label>
            <span>URL del repositorio remoto (HTTPS GitHub privado)</span>
            <input
              type="text"
              bind:value={remoteUrl}
              placeholder="https://github.com/usuario/mi-boveda-notas.git o usuario/repo"
            />
          </label>

          {#if discoveredRepos.length > 0}
            <label>
              <span>Seleccionar de mis repositorios de GitHub ({discoveredRepos.length})</span>
              <select
                value={remoteUrl}
                onchange={(e) => {
                  const val = (e.currentTarget as HTMLSelectElement).value;
                  const found = discoveredRepos.find((r) => r.clone_url === val);
                  if (found) {
                    remoteUrl = found.clone_url;
                    branch = found.default_branch || 'main';
                  }
                }}
              >
                {#each discoveredRepos as repo (repo.full_name)}
                  <option value={repo.clone_url}>
                    {repo.private ? '🔒' : '🌐'} {repo.full_name} ({repo.default_branch})
                  </option>
                {/each}
              </select>
            </label>
          {/if}

          <div class="form-row">
            <label>
              <span>Rama</span>
              <input type="text" bind:value={branch} placeholder="main" required />
            </label>
            <label>
              <span>Nombre de este dispositivo</span>
              <input type="text" bind:value={deviceName} placeholder="pc-windows" required />
            </label>
          </div>

          <div class="form-row">
            <label>
              <span>Autor Git</span>
              <input type="text" bind:value={authorName} placeholder="Tu Nombre" required />
            </label>
            <label>
              <span>Correo de commit</span>
              <input type="email" bind:value={authorEmail} placeholder="tu@correo.com" required />
            </label>
          </div>

          <label>
            <span>
              {isWeb
                ? 'Token PAT de GitHub (se guarda solo en sessionStorage y se borra al cerrar la pestaña)'
                : 'Token PAT de GitHub (opcional al editar; se guarda en Credential Manager del SO)'}
            </span>
            <div class="token-row">
              <input
                type="password"
                autocomplete="off"
                bind:value={patToken}
                placeholder={syncDto?.has_pat_token
                  ? '•••••••••••• (dejar vacío para conservar el token actual)'
                  : 'github_pat_... o ghp_...'}
              />
              <button
                type="button"
                class="mini-btn"
                onclick={() => void handleDiscoverRepos()}
                disabled={loading || syncing || loadingRepos || !patToken.trim()}
                title="Consultar tus repositorios de GitHub con este PAT"
              >
                {loadingRepos ? 'Buscando...' : '🔍 Mis repos'}
              </button>
              {#if syncDto?.has_pat_token}
                <button
                  type="button"
                  class="mini-btn danger"
                  onclick={() => void handleClearToken()}
                  disabled={loading || syncing}
                >
                  Borrar token
                </button>
              {/if}
            </div>
          </label>

          <footer class="sync-actions">
            <button
              type="submit"
              class="btn secondary"
              disabled={loading || syncing}
            >
              Guardar configuración
            </button>
            <button
              type="button"
              class="btn primary"
              onclick={() => void handleSyncClick()}
              disabled={loading || syncing}
            >
              {syncing ? 'Sincronizando...' : '⟳ Sincronizar ahora'}
            </button>
          </footer>
        </form>
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 55;
    background: var(--bg-backdrop);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 5vh 16px;
    overflow-y: auto;
  }

  .sync-dialog {
    width: min(680px, 100%);
    background: var(--bg-modal);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }

  .sync-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .sync-header h2 {
    margin: 0;
    font-size: 16px;
  }

  .sync-subtitle {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .close-btn {
    border: 1px solid var(--border-subtle);
    background: transparent;
    color: var(--text-secondary);
    border-radius: 6px;
    padding: 4px 8px;
    cursor: pointer;
  }

  .close-btn:hover {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
  }

  .sync-body {
    padding: 18px 20px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .status-card,
  .report-card {
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
    padding: 12px 14px;
  }

  .status-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    margin-bottom: 10px;
  }

  .state-pill {
    font-size: 12px;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 999px;
    border: 1px solid var(--border-subtle);
  }

  .state-pill.tone-neutral {
    background: var(--bg-app);
    color: var(--text-secondary);
  }

  .state-pill.tone-ok {
    background: var(--success-soft);
    color: var(--success);
    border-color: var(--success);
  }

  .state-pill.tone-warn {
    background: var(--pending-soft);
    color: var(--pending);
    border-color: var(--pending);
  }

  .state-pill.tone-err {
    background: var(--danger-soft);
    color: var(--danger);
    border-color: var(--danger);
  }

  .status-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(135px, 1fr));
    gap: 10px;
    font-size: 12px;
  }

  .meta-label {
    display: block;
    font-size: 11px;
    color: var(--text-muted);
    margin-bottom: 2px;
  }

  .text-ok {
    color: var(--success);
  }

  .text-muted {
    color: var(--text-muted);
  }

  .file-chips {
    margin-top: 10px;
    padding-top: 8px;
    border-top: 1px solid var(--border-subtle);
  }

  .file-chips ul {
    margin: 4px 0 0;
    padding-left: 18px;
    font-size: 12px;
    max-height: 92px;
    overflow-y: auto;
  }

  .report-card h3 {
    margin: 0 0 6px;
    font-size: 13px;
  }

  .report-summary {
    margin: 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .alert {
    padding: 10px 12px;
    border-radius: 8px;
    font-size: 12px;
    border: 1px solid transparent;
  }

  .alert.error {
    background: var(--danger-soft);
    border-color: var(--danger);
    color: var(--danger);
  }

  .alert.ok {
    background: var(--success-soft);
    border-color: var(--success);
    color: var(--success);
  }

  .alert.warn {
    margin-top: 8px;
    background: var(--pending-soft);
    border-color: var(--pending);
    color: var(--pending);
  }

  .conflict-list {
    margin: 6px 0 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .conflict-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .sync-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .sync-form label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .sync-form input {
    background: var(--bg-app);
    border: 1px solid var(--border-subtle);
    color: var(--text-primary);
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 13px;
  }

  .sync-form input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .form-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  .token-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .token-row input {
    flex: 1;
  }

  .mini-btn {
    border: 1px solid var(--border-subtle);
    background: var(--bg-app);
    color: var(--text-secondary);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }

  .mini-btn:hover:not(:disabled) {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
  }

  .mini-btn.danger:hover:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }

  .sync-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding-top: 6px;
  }

  .btn {
    border: 1px solid var(--border-subtle);
    border-radius: 7px;
    padding: 8px 14px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }

  .btn.secondary {
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .btn.secondary:hover:not(:disabled) {
    background: var(--bg-surface-hover);
  }

  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .btn.primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn:disabled,
  .mini-btn:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  @media (max-width: 600px) {
    .form-row {
      grid-template-columns: 1fr;
    }
  }
</style>
