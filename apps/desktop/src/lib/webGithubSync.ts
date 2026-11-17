import type {
  GithubRepoInfo,
  LocalVaultEntry,
  SyncConfig,
  SyncReport,
  SyncState,
  SyncStatusDto,
} from './types';

const WEB_STORAGE_KEY = 'romsidian-web-vaults-v1';

export interface WebVaultRecord {
  name: string;
  path: string;
  notes: Record<string, string>;
  canvases: Record<string, string>;
  baseShas: Record<string, string>;
  modifiedPaths: string[];
  deletedPaths: string[];
  conflictFiles: string[];
  lastCommitId: string | null;
  syncConfig: SyncConfig;
  patToken: string;
}

interface WebSessionStore {
  activeVaultPath: string;
  vaults: Record<string, WebVaultRecord>;
}

function defaultSyncConfig(): SyncConfig {
  return {
    remote_url: '',
    branch: 'main',
    device_name: 'web-browser',
    author_name: 'Roma (Web)',
    author_email: 'user@romsidian.local',
  };
}

function encodeBase64Utf8(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = '';
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const chunk = bytes.subarray(i, i + chunkSize);
    binary += String.fromCharCode(...chunk);
  }
  return btoa(binary);
}

function decodeBase64Utf8(base64Raw: string): string {
  const clean = base64Raw.replace(/\s+/g, '');
  const binary = atob(clean);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return new TextDecoder().decode(bytes);
}

export function parseGithubRepoFullName(remoteUrl: string): string | null {
  const trimmed = remoteUrl.trim();
  if (!trimmed) return null;

  // Shorthand: owner/repo
  const shorthandMatch = trimmed.match(/^([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+?)(?:\.git)?$/);
  if (shorthandMatch) {
    return `${shorthandMatch[1]}/${shorthandMatch[2]}`;
  }

  // HTTPS: https://github.com/owner/repo(.git)
  const httpsMatch = trimmed.match(
    /^https?:\/\/(?:[^@/\s]+@)?github\.com\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+?)(?:\.git)?\/?$/i,
  );
  if (httpsMatch) {
    return `${httpsMatch[1]}/${httpsMatch[2]}`;
  }

  // SSH: git@github.com:owner/repo(.git)
  const sshMatch = trimmed.match(
    /^git@github\.com:([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+?)(?:\.git)?$/i,
  );
  if (sshMatch) {
    return `${sshMatch[1]}/${sshMatch[2]}`;
  }

  return null;
}

let cachedStore: WebSessionStore | null = null;

function createEmptyVaultRecord(
  name: string,
  initialNotes: Record<string, string> = {},
  initialCanvases: Record<string, string> = {},
): WebVaultRecord {
  const cleanName = name.trim() || 'boveda-web';
  return {
    name: cleanName,
    path: cleanName,
    notes: { ...initialNotes },
    canvases: { ...initialCanvases },
    baseShas: {},
    modifiedPaths: [],
    deletedPaths: [],
    conflictFiles: [],
    lastCommitId: null,
    syncConfig: defaultSyncConfig(),
    patToken: '',
  };
}

export function initWebSessionStore(
  defaultNotes: Map<string, string>,
  defaultCanvases: Map<string, string>,
): WebSessionStore {
  if (cachedStore) return cachedStore;

  if (typeof window !== 'undefined' && window.sessionStorage) {
    try {
      const raw = window.sessionStorage.getItem(WEB_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as WebSessionStore;
        if (parsed && parsed.vaults && Object.keys(parsed.vaults).length > 0) {
          cachedStore = parsed;
          return cachedStore;
        }
      }
    } catch {
      // Ignore malformed sessionStorage and initialize default store
    }
  }

  const notesObj: Record<string, string> = {};
  for (const [k, v] of defaultNotes.entries()) notesObj[k] = v;
  const canvasesObj: Record<string, string> = {};
  for (const [k, v] of defaultCanvases.entries()) canvasesObj[k] = v;

  const defaultVault = createEmptyVaultRecord('boveda-demo', notesObj, canvasesObj);
  cachedStore = {
    activeVaultPath: defaultVault.path,
    vaults: {
      [defaultVault.path]: defaultVault,
    },
  };
  saveWebSessionStore();
  return cachedStore;
}

export function saveWebSessionStore(): void {
  if (!cachedStore) return;
  if (typeof window !== 'undefined' && window.sessionStorage) {
    try {
      window.sessionStorage.setItem(WEB_STORAGE_KEY, JSON.stringify(cachedStore));
    } catch (err) {
      console.warn('No se pudo persistir en sessionStorage:', err);
    }
  }
}

export function clearAllWebSessionData(
  defaultNotes: Map<string, string>,
  defaultCanvases: Map<string, string>,
): void {
  if (typeof window !== 'undefined' && window.sessionStorage) {
    window.sessionStorage.removeItem(WEB_STORAGE_KEY);
    window.sessionStorage.clear();
  }
  cachedStore = null;
  initWebSessionStore(defaultNotes, defaultCanvases);
}

export function getActiveWebVault(
  defaultNotes: Map<string, string>,
  defaultCanvases: Map<string, string>,
): WebVaultRecord {
  const store = initWebSessionStore(defaultNotes, defaultCanvases);
  const current = store.vaults[store.activeVaultPath];
  if (current) return current;
  const firstKey = Object.keys(store.vaults)[0];
  store.activeVaultPath = firstKey;
  saveWebSessionStore();
  return store.vaults[firstKey];
}

export function listWebVaults(
  defaultNotes: Map<string, string>,
  defaultCanvases: Map<string, string>,
): LocalVaultEntry[] {
  const store = initWebSessionStore(defaultNotes, defaultCanvases);
  return Object.values(store.vaults).map((v) => ({
    name: v.name,
    path: v.path,
    is_default: v.path === store.activeVaultPath,
  }));
}

export function createWebVault(
  name: string,
  defaultNotes: Map<string, string>,
  defaultCanvases: Map<string, string>,
): LocalVaultEntry {
  const store = initWebSessionStore(defaultNotes, defaultCanvases);
  const cleanName =
    name
      .trim()
      .replace(/[\\/:*?"<>|]+/g, '-')
      .replace(/\s+/g, '-') || 'nueva-boveda';
  if (!store.vaults[cleanName]) {
    const initialNote = `# ${cleanName}\n\nBóveda web en \`sessionStorage\`. Configurá **Sync** para conectarla con tu repositorio privado de GitHub.\n`;
    store.vaults[cleanName] = createEmptyVaultRecord(cleanName, {
      'Bienvenida.md': initialNote,
    });
  }
  store.activeVaultPath = cleanName;
  saveWebSessionStore();
  return {
    name: cleanName,
    path: cleanName,
    is_default: true,
  };
}

export function switchWebVault(
  path: string,
  defaultNotes: Map<string, string>,
  defaultCanvases: Map<string, string>,
): WebVaultRecord {
  const store = initWebSessionStore(defaultNotes, defaultCanvases);
  const clean = path.split(/[\\/]/).filter(Boolean).pop() || path || 'boveda-demo';
  if (!store.vaults[clean]) {
    store.vaults[clean] = createEmptyVaultRecord(clean, {
      'Bienvenida.md': `# ${clean}\n\nBóveda lista para sincronizar con GitHub.\n`,
    });
  }
  store.activeVaultPath = clean;
  saveWebSessionStore();
  return store.vaults[clean];
}

export function markWebPathModified(vault: WebVaultRecord, relPath: string): void {
  if (!vault.modifiedPaths.includes(relPath)) {
    vault.modifiedPaths.push(relPath);
  }
  vault.deletedPaths = vault.deletedPaths.filter((p) => p !== relPath);
  saveWebSessionStore();
}

export function markWebPathDeleted(vault: WebVaultRecord, relPath: string): void {
  vault.modifiedPaths = vault.modifiedPaths.filter((p) => p !== relPath);
  if (vault.baseShas[relPath] && !vault.deletedPaths.includes(relPath)) {
    vault.deletedPaths.push(relPath);
  }
  saveWebSessionStore();
}

export function getWebVaultSyncStatusDto(vault: WebVaultRecord): SyncStatusDto {
  const pendingChanges = Array.from(new Set([...vault.modifiedPaths, ...vault.deletedPaths]));
  let state: SyncState = 'Idle';
  if (vault.conflictFiles.length > 0) {
    state = { ConflictDetected: { conflict_files: [...vault.conflictFiles] } };
  } else if (vault.lastCommitId && pendingChanges.length === 0) {
    state = 'UpToDate';
  }
  return {
    status: {
      state,
      branch: vault.syncConfig.branch || 'main',
      remote_url: vault.syncConfig.remote_url || null,
      modified_files: pendingChanges,
      conflict_files: [...vault.conflictFiles],
      last_commit_id: vault.lastCommitId,
    },
    config: { ...vault.syncConfig },
    has_pat_token: Boolean(vault.patToken && vault.patToken.trim().length > 0),
  };
}

export function saveWebVaultSyncConfig(
  vault: WebVaultRecord,
  config: SyncConfig,
  patToken?: string | null,
): SyncStatusDto {
  vault.syncConfig = {
    remote_url: config.remote_url.trim(),
    branch: config.branch.trim() || 'main',
    device_name: config.device_name.trim() || 'web-browser',
    author_name: config.author_name.trim() || 'Roma (Web)',
    author_email: config.author_email.trim() || 'user@romsidian.local',
  };
  if (typeof patToken === 'string' && patToken.trim().length > 0) {
    vault.patToken = patToken.trim();
  }
  saveWebSessionStore();
  return getWebVaultSyncStatusDto(vault);
}

async function githubFetch<T>(
  endpoint: string,
  patToken: string,
  options: RequestInit = {},
): Promise<T> {
  const url = endpoint.startsWith('http') ? endpoint : `https://api.github.com${endpoint}`;
  const headers: Record<string, string> = {
    Accept: 'application/vnd.github+json',
    Authorization: `Bearer ${patToken.trim()}`,
    'X-GitHub-Api-Version': '2022-11-28',
  };
  if (options.body) {
    headers['Content-Type'] = 'application/json';
  }
  const response = await fetch(url, {
    ...options,
    headers: {
      ...headers,
      ...(options.headers as Record<string, string> | undefined),
    },
  });

  if (!response.ok) {
    let detail = response.statusText;
    try {
      const errBody = await response.json();
      if (errBody && typeof errBody.message === 'string') {
        detail = errBody.message;
      }
    } catch {
      // Ignore JSON parse failure
    }
    const err = new Error(`GitHub API (${response.status}): ${detail}`) as Error & {
      status?: number;
    };
    err.status = response.status;
    throw err;
  }

  return (await response.json()) as T;
}

export async function listUserGithubRepos(patToken: string): Promise<GithubRepoInfo[]> {
  const cleanToken = patToken.trim();
  if (!cleanToken) {
    throw new Error('Ingresá un Personal Access Token (PAT) de GitHub válido.');
  }
  interface RawRepo {
    full_name: string;
    name: string;
    owner?: { login?: string };
    private: boolean;
    html_url: string;
    clone_url: string;
    default_branch?: string;
    description: string | null;
    updated_at?: string;
  }
  const repos = await githubFetch<RawRepo[]>(
    '/user/repos?per_page=100&sort=updated&affiliation=owner,collaborator',
    cleanToken,
  );
  return repos.map((r) => ({
    full_name: r.full_name,
    name: r.name,
    owner: r.owner?.login ?? r.full_name.split('/')[0] ?? '',
    private: Boolean(r.private),
    html_url: r.html_url,
    clone_url: r.clone_url,
    default_branch: r.default_branch || 'main',
    description: r.description ?? null,
    updated_at: r.updated_at || '',
  }));
}

interface GitRefResponse {
  object: {
    sha: string;
    type: string;
  };
}

interface GitCommitResponse {
  sha: string;
  tree: {
    sha: string;
  };
}

interface GitTreeEntry {
  path: string;
  mode: string;
  type: 'blob' | 'tree' | 'commit';
  sha: string;
  size?: number;
}

interface GitTreeResponse {
  sha: string;
  tree: GitTreeEntry[];
  truncated?: boolean;
}

interface GitBlobResponse {
  sha: string;
  content: string;
  encoding: string;
}

function isVaultSyncablePath(relPath: string): boolean {
  if (!relPath || relPath.startsWith('.romsidian/') || relPath.startsWith('.git/')) {
    return false;
  }
  const lower = relPath.toLowerCase();
  return lower.endsWith('.md') || lower.endsWith('.canvas');
}

function buildConflictCopyPath(relPath: string): string {
  const now = new Date();
  const ts = `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, '0')}${String(
    now.getDate(),
  ).padStart(2, '0')}-${String(now.getHours()).padStart(2, '0')}${String(
    now.getMinutes(),
  ).padStart(2, '0')}`;
  if (relPath.toLowerCase().endsWith('.canvas')) {
    return relPath.replace(/\.canvas$/i, ` (conflicto web ${ts}).canvas`);
  }
  return relPath.replace(/\.md$/i, ` (conflicto web ${ts}).md`);
}

export async function syncWebVaultWithGithub(vault: WebVaultRecord): Promise<SyncReport> {
  const fullName = parseGithubRepoFullName(vault.syncConfig.remote_url);
  if (!fullName) {
    throw new Error(
      'URL remota inválida. Usá https://github.com/usuario/repo.git o usuario/repo.',
    );
  }
  const pat = vault.patToken.trim();
  if (!pat) {
    throw new Error(
      'Falta el Personal Access Token (PAT) de GitHub. Guardalo en Configurar Sync antes de sincronizar.',
    );
  }

  // 1. Verify repository access and inspect default_branch
  const repoMeta = await githubFetch<{ default_branch?: string }>(
    `/repos/${fullName}`,
    pat,
  );
  let branch = vault.syncConfig.branch.trim() || repoMeta.default_branch || 'main';

  // 2. Try to fetch branch reference
  let remoteCommitSha: string | null = null;
  try {
    const refData = await githubFetch<GitRefResponse>(
      `/repos/${fullName}/git/ref/heads/${encodeURIComponent(branch)}`,
      pat,
    );
    remoteCommitSha = refData.object.sha;
  } catch (err: unknown) {
    const status = (err as { status?: number })?.status;
    if (status === 404 && repoMeta.default_branch && repoMeta.default_branch !== branch) {
      try {
        const fallbackRef = await githubFetch<GitRefResponse>(
          `/repos/${fullName}/git/ref/heads/${encodeURIComponent(repoMeta.default_branch)}`,
          pat,
        );
        branch = repoMeta.default_branch;
        vault.syncConfig.branch = branch;
        remoteCommitSha = fallbackRef.object.sha;
      } catch {
        remoteCommitSha = null;
      }
    } else if (status === 404 || status === 409) {
      remoteCommitSha = null;
    } else {
      throw err;
    }
  }

  const mergedCleanly: string[] = [];
  const conflictCopiesCreated: string[] = [];
  const isFirstPullOnEmptyVault = Object.keys(vault.baseShas).length === 0;

  // If the remote repository has no commits yet, bootstrap the initial commit using Contents API
  if (!remoteCommitSha) {
    const allLocalEntries: Array<[string, string]> = [
      ...Object.entries(vault.notes),
      ...Object.entries(vault.canvases),
    ];
    if (allLocalEntries.length === 0) {
      allLocalEntries.push([
        'Bienvenida.md',
        `# ${vault.name}\n\nBóveda inicializada desde ROMSIDIAN Web.\n`,
      ]);
      vault.notes['Bienvenida.md'] = allLocalEntries[0][1];
    }

    let lastCommitSha: string | null = null;
    for (const [relPath, content] of allLocalEntries) {
      const encodedPath = relPath
        .split('/')
        .map((seg) => encodeURIComponent(seg))
        .join('/');
      const putRes = await githubFetch<{
        content?: { sha?: string };
        commit?: { sha?: string };
      }>(`/repos/${fullName}/contents/${encodedPath}`, pat, {
        method: 'PUT',
        body: JSON.stringify({
          message: `sync(${vault.syncConfig.device_name || 'web'}): inicializar ${relPath}`,
          content: encodeBase64Utf8(content),
          branch,
        }),
      });
      if (putRes.content?.sha) {
        vault.baseShas[relPath] = putRes.content.sha;
      }
      if (putRes.commit?.sha) {
        lastCommitSha = putRes.commit.sha;
      }
    }

    vault.modifiedPaths = [];
    vault.deletedPaths = [];
    vault.conflictFiles = [];
    vault.lastCommitId = lastCommitSha;
    saveWebSessionStore();

    return {
      state: 'UpToDate',
      committed_files: allLocalEntries.length,
      merged_cleanly: [],
      conflict_copies_created: [],
      pushed: true,
      head_commit: lastCommitSha,
    };
  }

  // 3. Fetch remote commit & recursive tree
  const commitData = await githubFetch<GitCommitResponse>(
    `/repos/${fullName}/git/commits/${remoteCommitSha}`,
    pat,
  );
  const remoteTreeSha = commitData.tree.sha;
  const treeData = await githubFetch<GitTreeResponse>(
    `/repos/${fullName}/git/trees/${remoteTreeSha}?recursive=1`,
    pat,
  );

  const remoteBlobEntries = treeData.tree.filter(
    (entry) => entry.type === 'blob' && isVaultSyncablePath(entry.path),
  );

  // If this vault was just created with the default demo notes and never synced before,
  // clear unedited demo notes before importing the real GitHub repository.
  if (isFirstPullOnEmptyVault && remoteBlobEntries.length > 0 && vault.modifiedPaths.length === 0) {
    vault.notes = {};
    vault.canvases = {};
  }

  const remotePathMap = new Map<string, string>();
  for (const entry of remoteBlobEntries) {
    remotePathMap.set(entry.path, entry.sha);
  }

  // 4. Pull new or updated files from GitHub
  for (const entry of remoteBlobEntries) {
    const relPath = entry.path;
    const remoteSha = entry.sha;
    const prevBaseSha = vault.baseShas[relPath];
    const isCanvas = relPath.toLowerCase().endsWith('.canvas');
    const localMap = isCanvas ? vault.canvases : vault.notes;
    const hasLocalFile = Object.prototype.hasOwnProperty.call(localMap, relPath);
    const wasModifiedLocally = vault.modifiedPaths.includes(relPath);
    const wasDeletedLocally = vault.deletedPaths.includes(relPath);

    // If unchanged on remote since last sync, keep local state
    if (prevBaseSha === remoteSha && (hasLocalFile || wasDeletedLocally)) {
      continue;
    }

    // Download blob content from GitHub
    const blobData = await githubFetch<GitBlobResponse>(
      `/repos/${fullName}/git/blobs/${remoteSha}`,
      pat,
    );
    const remoteContent = decodeBase64Utf8(blobData.content);

    if (wasModifiedLocally && hasLocalFile && localMap[relPath] !== remoteContent) {
      // Conflict between local edit and remote edit -> create conflict copy of local edit
      const conflictPath = buildConflictCopyPath(relPath);
      localMap[conflictPath] = localMap[relPath];
      if (!vault.modifiedPaths.includes(conflictPath)) {
        vault.modifiedPaths.push(conflictPath);
      }
      conflictCopiesCreated.push(conflictPath);
      localMap[relPath] = remoteContent;
      vault.modifiedPaths = vault.modifiedPaths.filter((p) => p !== relPath);
    } else {
      localMap[relPath] = remoteContent;
      vault.deletedPaths = vault.deletedPaths.filter((p) => p !== relPath);
      mergedCleanly.push(relPath);
    }

    vault.baseShas[relPath] = remoteSha;
  }

  // 5. Handle files that were deleted on remote since last sync
  for (const prevPath of Object.keys(vault.baseShas)) {
    if (!remotePathMap.has(prevPath)) {
      const wasModifiedLocally = vault.modifiedPaths.includes(prevPath);
      if (!wasModifiedLocally) {
        delete vault.notes[prevPath];
        delete vault.canvases[prevPath];
        delete vault.baseShas[prevPath];
        vault.deletedPaths = vault.deletedPaths.filter((p) => p !== prevPath);
      }
    }
  }

  // 6. Push local modifications and deletions if any
  const pathsToUpload = Array.from(new Set(vault.modifiedPaths)).filter((relPath) => {
    const isCanvas = relPath.toLowerCase().endsWith('.canvas');
    const localMap = isCanvas ? vault.canvases : vault.notes;
    return Object.prototype.hasOwnProperty.call(localMap, relPath);
  });
  const pathsToDelete = Array.from(new Set(vault.deletedPaths)).filter((relPath) =>
    remotePathMap.has(relPath),
  );

  let finalCommitSha = remoteCommitSha;
  let pushed = false;
  let committedFilesCount = 0;

  if (pathsToUpload.length > 0 || pathsToDelete.length > 0) {
    const newTreeItems: Array<{
      path: string;
      mode: '100644';
      type: 'blob';
      sha: string | null;
    }> = [];

    for (const relPath of pathsToUpload) {
      const isCanvas = relPath.toLowerCase().endsWith('.canvas');
      const content = isCanvas ? vault.canvases[relPath] : vault.notes[relPath];
      const createdBlob = await githubFetch<{ sha: string }>(
        `/repos/${fullName}/git/blobs`,
        pat,
        {
          method: 'POST',
          body: JSON.stringify({
            content: encodeBase64Utf8(content ?? ''),
            encoding: 'base64',
          }),
        },
      );
      newTreeItems.push({
        path: relPath,
        mode: '100644',
        type: 'blob',
        sha: createdBlob.sha,
      });
      vault.baseShas[relPath] = createdBlob.sha;
    }

    for (const delPath of pathsToDelete) {
      newTreeItems.push({
        path: delPath,
        mode: '100644',
        type: 'blob',
        sha: null,
      });
      delete vault.baseShas[delPath];
    }

    const createdTree = await githubFetch<{ sha: string }>(
      `/repos/${fullName}/git/trees`,
      pat,
      {
        method: 'POST',
        body: JSON.stringify({
          base_tree: remoteTreeSha,
          tree: newTreeItems,
        }),
      },
    );

    committedFilesCount = pathsToUpload.length + pathsToDelete.length;
    const createdCommit = await githubFetch<{ sha: string }>(
      `/repos/${fullName}/git/commits`,
      pat,
      {
        method: 'POST',
        body: JSON.stringify({
          message: `sync(${vault.syncConfig.device_name || 'web'}): ${committedFilesCount} archivo(s) actualizados`,
          tree: createdTree.sha,
          parents: [remoteCommitSha],
        }),
      },
    );

    await githubFetch<GitRefResponse>(
      `/repos/${fullName}/git/refs/heads/${encodeURIComponent(branch)}`,
      pat,
      {
        method: 'PATCH',
        body: JSON.stringify({
          sha: createdCommit.sha,
          force: false,
        }),
      },
    );

    finalCommitSha = createdCommit.sha;
    pushed = true;
    vault.modifiedPaths = [];
    vault.deletedPaths = [];
  }

  vault.conflictFiles = conflictCopiesCreated;
  vault.lastCommitId = finalCommitSha;
  saveWebSessionStore();

  const finalState: SyncState =
    conflictCopiesCreated.length > 0
      ? { ConflictDetected: { conflict_files: conflictCopiesCreated } }
      : 'UpToDate';

  return {
    state: finalState,
    committed_files: committedFilesCount,
    merged_cleanly: mergedCleanly,
    conflict_copies_created: conflictCopiesCreated,
    pushed,
    head_commit: finalCommitSha,
  };
}
