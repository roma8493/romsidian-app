<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    applyTemplateContent,
    createCanvasFile,
    createFolder,
    createLocalVault,
    deleteFolder,
    createNote,
    deleteCanvasFile,
    deleteNote,
    getAiProviderStatus,
    getDefaultVaultPath,
    getNotesByTag,
    getOrCreateDailyNote,
    getSyncStatus,
    getVaultSnapshot,
    listLocalVaults,
    listTemplates,
    linkUnlinkedMention,
    moveVaultItem,
    openVault,
    pickVaultFolder,
    readCanvasFile,
    readNote,
    renameCanvasFile,
    renameNote,
    resolveWikilink,
    saveAiProviderConfig,
    saveCanvasFile,
    saveNote,
    saveSyncConfig,
    syncVaultNow,
  } from './lib/api';
  import AiCopilotPanel from './lib/components/AiCopilotPanel.svelte';
  import BacklinksPanel from './lib/components/BacklinksPanel.svelte';
  import CanvasEditor from './lib/components/CanvasEditor.svelte';
  import CommandPaletteModal from './lib/components/CommandPaletteModal.svelte';
  import FileTreeSidebar from './lib/components/FileTreeSidebar.svelte';
  import MarkdownEditor from './lib/components/MarkdownEditor.svelte';
  import GraphView from './lib/components/GraphView.svelte';
  import SearchModal from './lib/components/SearchModal.svelte';
  import SyncModal from './lib/components/SyncModal.svelte';
  import VaultSwitcherModal from './lib/components/VaultSwitcherModal.svelte';
  import { normalizeNoteKey } from './lib/markdown';
  import {
    buildUniqueInboxNotePath,
    buildYouTubeNoteContent,
    parseSharedYouTubePayload,
    type AndroidSharePayload,
  } from './lib/youtubeShare';
  import type {
    AiProviderKind,
    AiProviderStatus,
    AutoConnectBatchResponse,
    BacklinksLocation,
    CanvasDocument,
    CommandAction,
    KnowledgeCanvasResult,
    LocalVaultEntry,
    NoteDetail,
    NoteSummary,
    SyncConfig,
    SyncReport,
    SyncStatusDto,
    TemplateSummary,
    UnlinkedMentionHit,
    VaultSnapshot,
    WorkspaceSettings,
  } from './lib/types';

  type ThemeMode = 'light' | 'dark';
  type PromptDialog =
    | { kind: 'create'; value: string; withFrontmatter: boolean }
    | { kind: 'create-folder'; value: string }
    | { kind: 'delete-folder'; path: string }
    | { kind: 'rename'; oldPath: string; value: string }
    | { kind: 'delete'; path: string }
    | { kind: 'create-canvas'; value: string }
    | { kind: 'rename-canvas'; oldPath: string; value: string }
    | { kind: 'delete-canvas'; path: string }
    | null;

  const THEME_STORAGE_KEY = 'romsidian-theme';
  const SETTINGS_STORAGE_KEY = 'romsidian-settings';
  const LAST_VAULT_STORAGE_KEY = 'romsidian-last-vault';
  const RECENT_VAULTS_STORAGE_KEY = 'romsidian-recent-vaults';
  const DEFAULT_SETTINGS: WorkspaceSettings = {
    dailyFolder: 'daily',
    templatesFolder: 'templates',
    youtubeInboxFolder: 'Inbox',
    dailyTemplatePath: '',
    enableDataviewJs: false,
    backlinksLocation: 'right-panel',
    backlinksExcludedFiles: [],
  };

  let theme = $state<ThemeMode>('dark');
  let settings = $state<WorkspaceSettings>({ ...DEFAULT_SETTINGS });
  let vaultPathInput = $state('');
  let vaultSwitcherOpen = $state(false);
  let vaultSwitcherLoading = $state(false);
  let vaultSwitcherError = $state<string | null>(null);
  let availableVaults = $state<LocalVaultEntry[]>([]);
  let snapshot = $state<VaultSnapshot | null>(null);
  let activeNote = $state<NoteDetail | null>(null);
  let activeCanvas = $state<CanvasDocument | null>(null);
  let pinnedBacklinkNote = $state<NoteDetail | null>(null);
  let isBacklinksPinned = $state(false);
  let backlinksRefreshToken = $state(0);
  let centerView = $state<'editor' | 'canvas' | 'graph-global' | 'graph-local'>('editor');

  function handleOpenNoteFromGraph(relativePath: string) {
    centerView = 'editor';
    void handleSelectNote(relativePath);
  }

  function handleOpenWikilinkFromGraph(target: string) {
    centerView = 'editor';
    void handleOpenWikilink(target);
  }
  let activeTag = $state<string | null>(null);
  let tagFilteredNotes = $state<NoteSummary[]>([]);

  let showLeftSidebar = $state(true);
  let showRightSidebar = $state(true);
  let searchOpen = $state(false);
  let paletteOpen = $state(false);
  let syncOpen = $state(false);
  let settingsOpen = $state(false);
  let templateModalOpen = $state(false);
  let aiCopilotOpen = $state(false);
  let aiCopilotTab = $state<'note' | 'batch' | 'canvas' | 'chat' | 'provider'>('note');
  let aiProviderStatus = $state<AiProviderStatus | null>(null);
  let aiQuickSaving = $state(false);
  let templateModalMode = $state<'insert' | 'create'>('insert');
  let availableTemplates = $state<TemplateSummary[]>([]);
  let templateNewNotePath = $state('');
  let templatesLoading = $state(false);
  let editorRef = $state<{
    setViewMode: (m: 'split' | 'edit' | 'preview') => Promise<void>;
    insertText: (snippet: string) => Promise<void>;
  } | null>(null);

  let syncDto = $state<SyncStatusDto | null>(null);
  let syncReport = $state<SyncReport | null>(null);
  let syncLoading = $state(false);
  let syncing = $state(false);
  let syncError = $state<string | null>(null);
  let conflictAlertCopies = $state<string[]>([]);
  let promptDialog = $state<PromptDialog>(null);
  let dialogInputEl = $state<HTMLInputElement | null>(null);

  let saving = $state(false);
  let errorMessage = $state<string | null>(null);
  let statusNotice = $state<string | null>(null);
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;

  let knownNoteKeys = $derived.by(() => {
    const keys = new Set<string>();
    if (!snapshot) return keys;
    for (const n of snapshot.notes) {
      keys.add(normalizeNoteKey(n.title));
      keys.add(normalizeNoteKey(n.relative_path));
    }
    return keys;
  });

  $effect(() => {
    if (typeof document !== 'undefined') {
      document.documentElement.dataset.theme = theme;
    }
  });

  function showNotice(message: string) {
    statusNotice = message;
    if (noticeTimer) clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => {
      statusNotice = null;
    }, 3200);
  }

  function ensureMarkdownExtension(pathInput: string): string {
    const trimmed = pathInput.trim().replace(/^[/\\]+/, '');
    if (!trimmed) return '';
    return trimmed.toLowerCase().endsWith('.md') ? trimmed : `${trimmed}.md`;
  }

  function ensureCanvasExtension(pathInput: string): string {
    const trimmed = pathInput.trim().replace(/^[/\\]+/, '');
    if (!trimmed) return '';
    return trimmed.toLowerCase().endsWith('.canvas') ? trimmed : `${trimmed}.canvas`;
  }

  function toggleTheme() {
    theme = theme === 'dark' ? 'light' : 'dark';
    try {
      localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      // Ignorar si almacenamiento no está disponible
    }
  }

  async function refreshSnapshotAndTag() {
    snapshot = await getVaultSnapshot();
    if (activeTag) {
      tagFilteredNotes = await getNotesByTag(activeTag);
    }
  }

  function normalizeVaultKey(p: string): string {
    return p.trim().replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  }

  function loadRecentVaults(): LocalVaultEntry[] {
    try {
      const raw = localStorage.getItem(RECENT_VAULTS_STORAGE_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      if (!Array.isArray(parsed)) return [];
      return parsed.filter(
        (v): v is LocalVaultEntry =>
          v && typeof v.name === 'string' && typeof v.path === 'string',
      );
    } catch {
      return [];
    }
  }

  function saveRecentVault(entry: LocalVaultEntry) {
    try {
      localStorage.setItem(LAST_VAULT_STORAGE_KEY, entry.path);
      const existing = loadRecentVaults();
      const key = normalizeVaultKey(entry.path);
      const next = [entry, ...existing.filter((item) => normalizeVaultKey(item.path) !== key)];
      localStorage.setItem(RECENT_VAULTS_STORAGE_KEY, JSON.stringify(next.slice(0, 20)));
    } catch {
      // Ignorar si localStorage no está disponible
    }
  }

  function removeRecentVault(path: string) {
    const key = normalizeVaultKey(path);
    const next = loadRecentVaults().filter((item) => normalizeVaultKey(item.path) !== key);
    try {
      localStorage.setItem(RECENT_VAULTS_STORAGE_KEY, JSON.stringify(next));
    } catch {}
    void refreshVaultList();
  }

  async function refreshVaultList() {
    vaultSwitcherLoading = true;
    vaultSwitcherError = null;
    try {
      const localList = await listLocalVaults();
      const recentList = loadRecentVaults();
      const merged: LocalVaultEntry[] = [];
      const seen = new Set<string>();
      for (const item of [...localList, ...recentList]) {
        const key = normalizeVaultKey(item.path);
        if (!key || seen.has(key)) continue;
        seen.add(key);
        merged.push(item);
      }
      availableVaults = merged;
    } catch (err) {
      vaultSwitcherError = String(err);
    } finally {
      vaultSwitcherLoading = false;
    }
  }

  async function openVaultSwitcher() {
    vaultSwitcherOpen = true;
    await refreshVaultList();
  }

  async function handleCreateLocalVault(name: string) {
    vaultSwitcherError = null;
    try {
      const created = await createLocalVault(name);
      await handleOpenVault(created.path);
      await refreshVaultList();
      vaultSwitcherOpen = false;
      showNotice(`Bóveda "${created.name}" creada. Abrí Sincronizar para conectar su repo GitHub.`);
    } catch (err) {
      vaultSwitcherError = String(err);
    }
  }

  async function handleOpenVault(targetPath: string) {
    const cleaned = targetPath.trim();
    if (!cleaned) return;
    errorMessage = null;
    activeTag = null;
    tagFilteredNotes = [];
    syncReport = null;
    syncError = null;
    conflictAlertCopies = [];
    pinnedBacklinkNote = null;
    isBacklinksPinned = false;
    try {
      const snap = await openVault(cleaned);
      snapshot = snap;
      vaultPathInput = snap.info.root_path;
      saveRecentVault({
        name: snap.info.name,
        path: snap.info.root_path,
        is_default: false,
      });
      if (snap.notes.length > 0) {
        const preferred =
          snap.notes.find((n) => normalizeNoteKey(n.title) === 'índice general') ??
          snap.notes[0];
        activeNote = await readNote(preferred.relative_path);
      } else {
        activeNote = null;
      }
      showNotice(`Bóveda abierta (${snap.notes.length} notas)`);
      void refreshSyncStatus();
      void refreshAiProviderStatus();
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function handleRefreshVault() {
    if (!snapshot) return;
    errorMessage = null;
    try {
      await handleOpenVault(snapshot.info.root_path);
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function handleMoveItem(sourcePath: string, targetFolderPath: string) {
    errorMessage = null;
    try {
      const newPath = await moveVaultItem(sourcePath, targetFolderPath);
      if (activeNote) {
        if (activeNote.relative_path === sourcePath) {
          activeNote = await readNote(newPath);
        } else if (activeNote.relative_path.startsWith(`${sourcePath}/`)) {
          const suffix = activeNote.relative_path.slice(sourcePath.length);
          const nextNotePath = `${newPath}${suffix}`;
          try {
            activeNote = await readNote(nextNotePath);
          } catch {}
        }
      }
      if (activeCanvas) {
        if (activeCanvas.relative_path === sourcePath) {
          activeCanvas = await readCanvasFile(newPath);
        } else if (activeCanvas.relative_path.startsWith(`${sourcePath}/`)) {
          const suffix = activeCanvas.relative_path.slice(sourcePath.length);
          const nextCanvasPath = `${newPath}${suffix}`;
          try {
            activeCanvas = await readCanvasFile(nextCanvasPath);
          } catch {}
        }
      }
      await refreshSnapshotAndTag();
      showNotice(`Movido con éxito a: ${newPath || 'raíz'}`);
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function handlePickFolder() {
    errorMessage = null;
    try {
      const selected = await pickVaultFolder();
      if (selected) {
        vaultPathInput = selected;
        await handleOpenVault(selected);
      }
    } catch (err) {
      errorMessage = String(err);
    }
  }

  function closeSidebarsOnMobile() {
    if (typeof window !== 'undefined' && window.innerWidth <= 900) {
      showLeftSidebar = false;
      showRightSidebar = false;
    }
  }

  let queuedAndroidShare: AndroidSharePayload | null = null;
  let processingAndroidShare = false;

  async function createNoteFromYouTubeShare(payload: AndroidSharePayload) {
    const parsed = parseSharedYouTubePayload(payload);
    if (!parsed.url) return;

    const existingPaths = snapshot?.notes.map((n) => n.relative_path) ?? [];
    const dateStr = localDateString();
    const timeStr = localTimeString();
    const relativePath = buildUniqueInboxNotePath(
      settings.youtubeInboxFolder,
      parsed,
      existingPaths,
      dateStr,
      timeStr,
    );
    const content = buildYouTubeNoteContent(parsed, dateStr);
    const created = await createNote(relativePath, content);
    activeNote = created;
    centerView = 'editor';
    closeSidebarsOnMobile();
    await refreshSnapshotAndTag();
    await tick();
    await editorRef?.setViewMode('edit');
    showNotice(`Video guardado en ${settings.youtubeInboxFolder}: ${created.title}`);
  }

  async function handlePendingAndroidShare() {
    if (processingAndroidShare) return;
    processingAndroidShare = true;
    try {
      const bridge = (
        window as Window & {
          RomsidianAndroidShare?: { consumePendingShare?: () => string | null };
        }
      ).RomsidianAndroidShare;
      const rawJson = bridge?.consumePendingShare?.();
      if (rawJson) {
        const parsed = JSON.parse(rawJson) as AndroidSharePayload;
        if (parsed?.text?.trim()) {
          queuedAndroidShare = parsed;
        }
      }

      if (!queuedAndroidShare || !snapshot) return;
      const toProcess = queuedAndroidShare;
      queuedAndroidShare = null;
      await createNoteFromYouTubeShare(toProcess);
    } catch (err) {
      errorMessage = String(err);
    } finally {
      processingAndroidShare = false;
    }
  }

  async function handleSelectNote(relativePath: string) {
    errorMessage = null;
    try {
      activeNote = await readNote(relativePath);
      centerView = 'editor';
      closeSidebarsOnMobile();
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function handleSelectCanvas(relativePath: string) {
    errorMessage = null;
    try {
      activeCanvas = await readCanvasFile(relativePath);
      centerView = 'canvas';
      closeSidebarsOnMobile();
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function handleSaveCanvas(relativePath: string, content: string) {
    saving = true;
    errorMessage = null;
    try {
      const updated = await saveCanvasFile(relativePath, content);
      if (activeCanvas?.relative_path === relativePath) {
        activeCanvas = updated;
      }
      await refreshSnapshotAndTag();
    } catch (err) {
      errorMessage = String(err);
    } finally {
      saving = false;
    }
  }

  async function handleConvertCanvasTextToNote(
    suggestedTitle: string,
    text: string,
  ): Promise<string | null> {
    if (!snapshot) return null;
    errorMessage = null;
    const sanitized = suggestedTitle.replace(/[\\/:*?"<>|]/g, '-').trim() || 'Nota Canvas';
    let candidate = `${sanitized}.md`;
    let suffix = 2;
    const existing = new Set(snapshot.notes.map((n) => n.relative_path.toLowerCase()));
    while (existing.has(candidate.toLowerCase())) {
      candidate = `${sanitized} (${suffix}).md`;
      suffix += 1;
    }
    try {
      const created = await createNote(candidate, text || `# ${sanitized}\n\n`);
      await refreshSnapshotAndTag();
      showNotice(`Tarjeta convertida a nota: ${created.relative_path}`);
      return created.relative_path;
    } catch (err) {
      errorMessage = String(err);
      return null;
    }
  }

  function handleOpenNoteFromCanvas(relativePath: string) {
    void handleSelectNote(relativePath);
  }

  async function handleSaveNote(relativePath: string, content: string) {
    saving = true;
    errorMessage = null;
    try {
      const updated = await saveNote(relativePath, content);
      if (activeNote?.relative_path === relativePath) {
        activeNote = {
          ...activeNote,
          content: updated.content,
          modified_unix_secs: updated.modified_unix_secs,
          frontmatter: updated.frontmatter,
          wikilinks: updated.wikilinks,
          tags: updated.tags,
          backlinks: updated.backlinks,
        };
      }
      await refreshSnapshotAndTag();
      if (isBacklinksPinned && pinnedBacklinkNote?.relative_path === relativePath) {
        pinnedBacklinkNote = activeNote?.relative_path === relativePath ? activeNote : updated;
      }
      backlinksRefreshToken += 1;
    } catch (err) {
      errorMessage = String(err);
    } finally {
      saving = false;
    }
  }

  async function handleOpenWikilink(target: string, _heading: string | null) {
    errorMessage = null;
    try {
      const resolved = await resolveWikilink(target);
      if (resolved) {
        activeNote = await readNote(resolved);
        return;
      }
      const newPath = ensureMarkdownExtension(target);
      if (!newPath) return;
      const starter = `# ${target.trim()}\n\n`;
      const created = await createNote(newPath, starter);
      await refreshSnapshotAndTag();
      activeNote = created;
      showNotice(`Nota creada desde wikilink: ${created.relative_path}`);
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function handleCreateNoteFromChat(relativePath: string, content: string) {
    errorMessage = null;
    try {
      const cleanPath = ensureMarkdownExtension(relativePath);
      if (!cleanPath) return;
      const created = await createNote(cleanPath, content);
      await refreshSnapshotAndTag();
      activeNote = created;
      centerView = 'editor';
      closeSidebarsOnMobile();
      showNotice(`Nota creada desde el Copiloto: ${created.relative_path}`);
    } catch (err) {
      errorMessage = String(err);
      throw err;
    }
  }

  async function handleSelectTag(tag: string | null) {
    errorMessage = null;
    activeTag = tag;
    if (!tag) {
      tagFilteredNotes = [];
      return;
    }
    try {
      showLeftSidebar = true;
      tagFilteredNotes = await getNotesByTag(tag);
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function openCreateDialog() {
    let savedAuto = false;
    try {
      savedAuto = localStorage.getItem('romsidian-auto-frontmatter') === 'true';
    } catch {}
    promptDialog = { kind: 'create', value: '', withFrontmatter: savedAuto };
    await tick();
    dialogInputEl?.focus();
  }

  async function openCreateCanvasDialog() {
    promptDialog = { kind: 'create-canvas', value: '' };
    await tick();
    dialogInputEl?.focus();
  }

  async function openRenameDialog(relativePath: string) {
    promptDialog = {
      kind: 'rename',
      oldPath: relativePath,
      value: relativePath,
    };
    await tick();
    dialogInputEl?.focus();
    dialogInputEl?.select();
  }

  async function openRenameCanvasDialog(relativePath: string) {
    promptDialog = {
      kind: 'rename-canvas',
      oldPath: relativePath,
      value: relativePath,
    };
    await tick();
    dialogInputEl?.focus();
    dialogInputEl?.select();
  }

  function openDeleteDialog(relativePath: string) {
    promptDialog = { kind: 'delete', path: relativePath };
  }

  function openDeleteFolderDialog(folderPath: string) {
    promptDialog = { kind: 'delete-folder', path: folderPath };
  }

  function openDeleteCanvasDialog(relativePath: string) {
    promptDialog = { kind: 'delete-canvas', path: relativePath };
  }

  async function submitPromptDialog(event: SubmitEvent) {
    event.preventDefault();
    const dlg = promptDialog;
    if (!dlg) return;

    errorMessage = null;
    try {
      if (dlg.kind === 'create') {
        const relPath = ensureMarkdownExtension(dlg.value);
        if (!relPath) return;
        const title = relPath.split('/').pop()?.replace(/\.md$/i, '') || 'Nueva nota';
        try {
          localStorage.setItem('romsidian-auto-frontmatter', String(dlg.withFrontmatter));
        } catch {}
        const starter = dlg.withFrontmatter
          ? `---\ntitle: ${title}\ncreated: ${localDateString()}\ntags:\n  - general\n---\n\n# ${title}\n\n`
          : `# ${title}\n\n`;
        const created = await createNote(relPath, starter);
        promptDialog = null;
        await refreshSnapshotAndTag();
        activeNote = created;
        centerView = 'editor';
        closeSidebarsOnMobile();
        showNotice(`Nota creada: ${created.relative_path}`);
      } else if (dlg.kind === 'rename') {
        const nextPath = ensureMarkdownExtension(dlg.value);
        if (!nextPath || nextPath === dlg.oldPath) {
          promptDialog = null;
          return;
        }
        const renamed = await renameNote(dlg.oldPath, nextPath);
        promptDialog = null;
        await refreshSnapshotAndTag();
        if (activeNote?.relative_path === dlg.oldPath) {
          activeNote = renamed;
        }
        showNotice(`Renombrada a ${renamed.relative_path}`);
      } else if (dlg.kind === 'create-folder') {
        const folderName = dlg.value.trim();
        if (!folderName) return;
        await createFolder(folderName);
        promptDialog = null;
        await refreshSnapshotAndTag();
        showNotice(`Carpeta creada: ${folderName}`);
      } else if (dlg.kind === 'create-canvas') {
        const relPath = ensureCanvasExtension(dlg.value);
        if (!relPath) return;
        const created = await createCanvasFile(relPath);
        promptDialog = null;
        await refreshSnapshotAndTag();
        activeCanvas = created;
        centerView = 'canvas';
        closeSidebarsOnMobile();
        showNotice(`Lienzo creado: ${created.relative_path}`);
      } else if (dlg.kind === 'rename-canvas') {
        const nextPath = ensureCanvasExtension(dlg.value);
        if (!nextPath || nextPath === dlg.oldPath) {
          promptDialog = null;
          return;
        }
        const renamed = await renameCanvasFile(dlg.oldPath, nextPath);
        promptDialog = null;
        await refreshSnapshotAndTag();
        if (activeCanvas?.relative_path === dlg.oldPath) {
          activeCanvas = await readCanvasFile(renamed.relative_path);
        }
        showNotice(`Lienzo renombrado a ${renamed.relative_path}`);
      }
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function confirmDeleteNote() {
    if (!promptDialog) return;
    if (promptDialog.kind === 'delete-canvas') {
      const targetPath = promptDialog.path;
      errorMessage = null;
      try {
        snapshot = await deleteCanvasFile(targetPath);
        promptDialog = null;
        if (activeCanvas?.relative_path === targetPath) {
          activeCanvas = null;
          if (centerView === 'canvas') centerView = 'editor';
        }
        showNotice(`Lienzo eliminado: ${targetPath}`);
      } catch (err) {
        errorMessage = String(err);
      }
      return;
    }
    if (promptDialog.kind !== 'delete') return;
    const targetPath = promptDialog.path;
    errorMessage = null;
    try {
      snapshot = await deleteNote(targetPath);
      promptDialog = null;
      if (activeTag) {
        tagFilteredNotes = await getNotesByTag(activeTag);
      }
      if (activeNote?.relative_path === targetPath) {
        activeNote =
          snapshot.notes.length > 0 ? await readNote(snapshot.notes[0].relative_path) : null;
      }
      showNotice(`Nota eliminada: ${targetPath}`);
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function confirmDeleteFolder() {
    if (!promptDialog || promptDialog.kind !== 'delete-folder') return;
    const targetPath = promptDialog.path;
    errorMessage = null;
    try {
      await deleteFolder(targetPath);
      promptDialog = null;
      await refreshSnapshotAndTag();
      if (
        activeNote &&
        (activeNote.relative_path === targetPath ||
          activeNote.relative_path.startsWith(`${targetPath}/`))
      ) {
        activeNote =
          snapshot?.notes && snapshot.notes.length > 0
            ? await readNote(snapshot.notes[0].relative_path)
            : null;
      }
      if (
        activeCanvas &&
        (activeCanvas.relative_path === targetPath ||
          activeCanvas.relative_path.startsWith(`${targetPath}/`))
      ) {
        activeCanvas = null;
        if (centerView === 'canvas') centerView = 'editor';
      }
      showNotice(`Carpeta eliminada: ${targetPath}`);
    } catch (err) {
      errorMessage = String(err);
    }
  }

  function formatConflictTimestamp(): string {
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}-${pad(now.getMinutes())}`;
  }

  async function refreshSyncStatus() {
    if (!snapshot) return;
    syncLoading = true;
    syncError = null;
    try {
      syncDto = await getSyncStatus();
    } catch (err) {
      syncError = String(err);
    } finally {
      syncLoading = false;
    }
  }

  async function openSyncModal() {
    syncOpen = true;
    await refreshSyncStatus();
  }

  async function handleSaveSyncConfig(
    config: SyncConfig,
    patToken?: string,
    clearToken?: boolean,
  ): Promise<boolean> {
    syncLoading = true;
    syncError = null;
    try {
      syncDto = await saveSyncConfig(config, patToken, clearToken);
      showNotice('Configuración Git actualizada');
      return true;
    } catch (err) {
      syncError = String(err);
      return false;
    } finally {
      syncLoading = false;
    }
  }

  async function handleSyncNow() {
    if (!snapshot) return;
    syncing = true;
    syncError = null;
    try {
      const report = await syncVaultNow(formatConflictTimestamp());
      syncReport = report;
      await refreshSnapshotAndTag();
      if (activeNote) {
        try {
          activeNote = await readNote(activeNote.relative_path);
        } catch {
          activeNote = null;
        }
      }
      await refreshSyncStatus();

      if (report.conflict_copies_created.length > 0) {
        conflictAlertCopies = report.conflict_copies_created;
        showNotice(
          `Sincronizado con ${report.conflict_copies_created.length} copia(s) de conflicto`,
        );
      } else {
        conflictAlertCopies = [];
        showNotice(
          report.pushed
            ? 'Sincronización completada con el remoto'
            : 'Cambios confirmados en el repositorio local',
        );
      }
    } catch (err) {
      syncError = String(err);
    } finally {
      syncing = false;
    }
  }

  function persistSettings(next: WorkspaceSettings) {
    settings = next;
    try {
      localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Ignorar si almacenamiento no está disponible
    }
  }

  function handleChangeBacklinksLocation(nextLocation: BacklinksLocation) {
    persistSettings({ ...settings, backlinksLocation: nextLocation });
    if (nextLocation === 'right-panel' || nextLocation === 'pinned-tab') {
      showRightSidebar = true;
    }
    if (nextLocation === 'pinned-tab') {
      isBacklinksPinned = true;
      if (!pinnedBacklinkNote && activeNote) {
        pinnedBacklinkNote = activeNote;
      }
    }
  }

  function handleToggleBacklinksPin() {
    if (isBacklinksPinned) {
      isBacklinksPinned = false;
      pinnedBacklinkNote = null;
    } else if (activeNote) {
      isBacklinksPinned = true;
      pinnedBacklinkNote = activeNote;
    }
  }

  function handlePinCurrentNote() {
    if (!activeNote) return;
    isBacklinksPinned = true;
    pinnedBacklinkNote = activeNote;
  }

  function handleUpdateExcludedFiles(patterns: string[]) {
    persistSettings({ ...settings, backlinksExcludedFiles: patterns });
  }

  async function handleLinkUnlinkedMention(hit: UnlinkedMentionHit, targetNotePath: string) {
    errorMessage = null;
    try {
      await linkUnlinkedMention(hit.source_path, hit.line, hit.matched_term, targetNotePath);
      await refreshSnapshotAndTag();
      if (activeNote) {
        activeNote = await readNote(activeNote.relative_path);
      }
      if (isBacklinksPinned && pinnedBacklinkNote) {
        pinnedBacklinkNote = await readNote(pinnedBacklinkNote.relative_path);
      }
      backlinksRefreshToken += 1;
      showNotice(`Enlazada mención "${hit.matched_term}" en ${hit.source_title}`);
    } catch (err) {
      errorMessage = String(err);
    }
  }

  function localDateString(): string {
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  }

  function localTimeString(): string {
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${pad(now.getHours())}:${pad(now.getMinutes())}`;
  }

  async function refreshTemplateCatalog() {
    if (!snapshot) return;
    templatesLoading = true;
    try {
      availableTemplates = await listTemplates(settings.templatesFolder);
    } catch {
      availableTemplates = [];
    } finally {
      templatesLoading = false;
    }
  }

  async function handleOpenDailyNote() {
    if (!snapshot) return;
    errorMessage = null;
    try {
      let chosenTemplate = settings.dailyTemplatePath.trim() || null;
      if (!chosenTemplate) {
        const templates = await listTemplates(settings.templatesFolder);
        const autoMatch = templates.find(
          (t) => t.name.toLowerCase() === 'diaria' || t.name.toLowerCase() === 'daily',
        );
        if (autoMatch) chosenTemplate = autoMatch.relative_path;
      }

      const result = await getOrCreateDailyNote({
        dateStr: localDateString(),
        timeStr: localTimeString(),
        dailyFolder: settings.dailyFolder,
        templatePath: chosenTemplate,
      });

      if (result.created) {
        await refreshSnapshotAndTag();
      }
      activeNote = await readNote(result.note.relative_path);
      centerView = 'editor';
      closeSidebarsOnMobile();
      showNotice(
        result.created
          ? `Nota diaria creada: ${result.note.relative_path}`
          : `Nota diaria abierta: ${result.note.relative_path}`,
      );
    } catch (err) {
      errorMessage = String(err);
    }
  }

  async function openTemplateModal(mode: 'insert' | 'create') {
    if (!snapshot) return;
    templateModalMode = mode;
    templateNewNotePath = '';
    templateModalOpen = true;
    await refreshTemplateCatalog();
  }

  async function handleApplyTemplateChoice(templatePath: string) {
    errorMessage = null;
    try {
      if (templateModalMode === 'insert') {
        if (!activeNote) return;
        const text = await applyTemplateContent(
          templatePath,
          activeNote.title,
          localDateString(),
          localTimeString(),
        );
        await editorRef?.insertText(text);
        templateModalOpen = false;
        showNotice(`Plantilla insertada desde ${templatePath}`);
      } else {
        const rawPath = templateNewNotePath.trim() || `${localDateString()} - Nota`;
        const relPath = ensureMarkdownExtension(rawPath);
        if (!relPath) return;
        const title = relPath.split('/').pop()?.replace(/\.md$/i, '') || 'Nueva nota';
        const content = await applyTemplateContent(
          templatePath,
          title,
          localDateString(),
          localTimeString(),
        );
        const created = await createNote(relPath, content);
        templateModalOpen = false;
        await refreshSnapshotAndTag();
        activeNote = created;
        centerView = 'editor';
        closeSidebarsOnMobile();
        showNotice(`Nota creada desde plantilla: ${created.relative_path}`);
      }
    } catch (err) {
      errorMessage = String(err);
    }
  }

  const DEFAULT_AI_MODELS: Record<AiProviderKind, string> = {
    deterministic: 'deterministic-rust-v1',
    claude_cli: 'claude-sonnet-4-5',
    gemini_cli: 'gemini-2.5-flash',
    gemini_http: 'gemini-2.5-flash',
    compatible_http: 'gpt-4o-mini',
  };

  async function refreshAiProviderStatus() {
    if (!snapshot) return;
    try {
      aiProviderStatus = await getAiProviderStatus();
    } catch {
      // Ignorar si aún no se completó la apertura de bóveda
    }
  }

  function openAiCopilot(tab: 'note' | 'batch' | 'canvas' | 'chat' | 'provider' = 'note') {
    if (!snapshot) return;
    aiCopilotTab = tab;
    aiCopilotOpen = true;
    void refreshAiProviderStatus();
  }

  async function handleQuickProviderChange(nextProvider: AiProviderKind) {
    if (!snapshot) return;
    aiQuickSaving = true;
    errorMessage = null;
    try {
      const current = aiProviderStatus?.active_config;
      aiProviderStatus = await saveAiProviderConfig({
        provider: nextProvider,
        model: DEFAULT_AI_MODELS[nextProvider],
        endpoint_url: current?.endpoint_url ?? null,
        cli_binary_override: current?.cli_binary_override ?? null,
        max_connections: current?.max_connections ?? 8,
        append_related_section: current?.append_related_section ?? true,
        fallback_to_deterministic: current?.fallback_to_deterministic ?? true,
      });
      showNotice(`Proveedor IA cambiado a: ${nextProvider}`);
    } catch (err) {
      errorMessage = String(err);
    } finally {
      aiQuickSaving = false;
    }
  }

  async function handleAiApplied(response: AutoConnectBatchResponse) {
    if (response.snapshot) {
      snapshot = response.snapshot;
      if (activeTag) {
        tagFilteredNotes = await getNotesByTag(activeTag);
      }
    } else {
      await refreshSnapshotAndTag();
    }
    if (activeNote) {
      try {
        activeNote = await readNote(activeNote.relative_path);
      } catch {
        activeNote = null;
      }
    }
    if (isBacklinksPinned && pinnedBacklinkNote) {
      try {
        pinnedBacklinkNote = await readNote(pinnedBacklinkNote.relative_path);
      } catch {
        pinnedBacklinkNote = null;
      }
    }
    backlinksRefreshToken += 1;
    if (response.applied_count > 0) {
      showNotice(
        `Copiloto IA: ${response.applied_count} nota(s) enriquecida(s) y reindexada(s) en SQLite`,
      );
    }
  }

  async function handleKnowledgeCanvasCreated(result: KnowledgeCanvasResult) {
    await refreshSnapshotAndTag();
    activeCanvas = result.canvas;
    centerView = 'canvas';
    aiCopilotOpen = false;
    closeSidebarsOnMobile();
    showNotice(
      `Lienzo generado (${result.node_count} nodos, ${result.edge_count} aristas): ${result.canvas.relative_path}`,
    );
  }

  async function openSettingsModal() {
    settingsOpen = true;
    await Promise.all([refreshTemplateCatalog(), refreshAiProviderStatus()]);
  }

  let paletteCommands = $derived.by<CommandAction[]>(() => [
    {
      id: 'daily-note',
      category: 'Notas Diarias',
      label: 'Abrir o crear nota diaria de hoy',
      shortcut: 'Alt+D',
      keywords: 'daily hoy diario journal calendario',
      disabled: !snapshot,
      run: () => handleOpenDailyNote(),
    },
    {
      id: 'insert-template',
      category: 'Plantillas',
      label: 'Insertar plantilla en la nota actual',
      shortcut: 'Alt+T',
      keywords: 'template snippet fecha hora titulo',
      disabled: !snapshot || !activeNote,
      run: () => openTemplateModal('insert'),
    },
    {
      id: 'create-from-template',
      category: 'Plantillas',
      label: 'Crear nueva nota desde plantilla',
      keywords: 'template nueva archivo crear',
      disabled: !snapshot,
      run: () => openTemplateModal('create'),
    },
    {
      id: 'new-note',
      category: 'Archivo',
      label: 'Nueva nota Markdown',
      shortcut: 'Ctrl+N',
      keywords: 'crear markdown',
      disabled: !snapshot,
      run: () => openCreateDialog(),
    },
    {
      id: 'insert-frontmatter',
      category: 'Metadatos',
      label: 'Insertar Frontmatter (metadatos YAML) en la nota activa',
      shortcut: 'Alt+M',
      keywords: 'frontmatter yaml metadatos tags propiedades fecha cabecera',
      disabled: !activeNote,
      run: async () => {
        if (!activeNote) return;
        if (activeNote.content.trim().startsWith('---')) {
          showNotice('La nota ya tiene un bloque Frontmatter');
          return;
        }
        const block = `---\ntitle: ${activeNote.title}\ncreated: ${localDateString()}\ntags:\n  - general\n---\n\n`;
        const updated = `${block}${activeNote.content}`;
        await handleSaveNote(activeNote.relative_path, updated);
        showNotice('Frontmatter YAML insertado con éxito');
      },
    },
    {
      id: 'rename-note',
      category: 'Archivo',
      label: 'Renombrar o mover nota activa',
      keywords: 'rename move',
      disabled: !activeNote,
      run: () => (activeNote ? openRenameDialog(activeNote.relative_path) : undefined),
    },
    {
      id: 'delete-note',
      category: 'Archivo',
      label: 'Eliminar nota activa',
      keywords: 'borrar remove',
      disabled: !activeNote,
      run: () => (activeNote ? openDeleteDialog(activeNote.relative_path) : undefined),
    },
    {
      id: 'view-split',
      category: 'Editor',
      label: 'Cambiar a vista Dividida (editor + vista previa)',
      keywords: 'split mode preview',
      disabled: !activeNote,
      run: () => editorRef?.setViewMode('split'),
    },
    {
      id: 'view-edit',
      category: 'Editor',
      label: 'Cambiar a modo solo Edición',
      keywords: 'edit codemirror',
      disabled: !activeNote,
      run: () => editorRef?.setViewMode('edit'),
    },
    {
      id: 'view-preview',
      category: 'Editor',
      label: 'Cambiar a modo Vista Previa / Lectura',
      keywords: 'reading preview markdown',
      disabled: !activeNote,
      run: () => editorRef?.setViewMode('preview'),
    },
    {
      id: 'open-search',
      category: 'Navegación',
      label: 'Buscar en toda la bóveda (SQLite FTS5)',
      shortcut: 'Ctrl+K',
      keywords: 'search grep fts',
      disabled: !snapshot,
      run: () => {
        searchOpen = true;
      },
    },
    {
      id: 'toggle-left-sidebar',
      category: 'Interfaz',
      label: showLeftSidebar ? 'Ocultar explorador izquierdo' : 'Mostrar explorador izquierdo',
      keywords: 'sidebar files tags',
      run: () => {
        showLeftSidebar = !showLeftSidebar;
      },
    },
    {
      id: 'toggle-right-sidebar',
      category: 'Interfaz',
      label: showRightSidebar ? 'Ocultar panel de backlinks' : 'Mostrar panel de backlinks',
      keywords: 'backlinks properties',
      run: () => {
        showRightSidebar = !showRightSidebar;
      },
    },
    {
      id: 'backlinks-right-panel',
      category: 'Retroenlaces',
      label: 'Retroenlaces: mover a Panel derecho',
      keywords: 'backlinks panel sidebar right',
      run: () => handleChangeBacklinksLocation('right-panel'),
    },
    {
      id: 'backlinks-document-footer',
      category: 'Retroenlaces',
      label: 'Retroenlaces: incrustar al Pie del documento activo',
      keywords: 'backlinks footer document bottom',
      run: () => handleChangeBacklinksLocation('document-footer'),
    },
    {
      id: 'backlinks-pinned-tab',
      category: 'Retroenlaces',
      label: 'Retroenlaces: abrir como Pestaña fijada',
      keywords: 'backlinks pinned tab fijar',
      run: () => handleChangeBacklinksLocation('pinned-tab'),
    },
    {
      id: 'backlinks-toggle-pin',
      category: 'Retroenlaces',
      label: isBacklinksPinned
        ? 'Retroenlaces: seguir nota activa (desfijar)'
        : 'Retroenlaces: fijar en la nota actual',
      keywords: 'pin backlinks lock',
      disabled: !activeNote && !isBacklinksPinned,
      run: () => handleToggleBacklinksPin(),
    },
    {
      id: 'toggle-theme',
      category: 'Apariencia',
      label: theme === 'dark' ? 'Cambiar a tema Claro' : 'Cambiar a tema Oscuro',
      keywords: 'dark light theme color',
      run: () => toggleTheme(),
    },
    {
      id: 'switch-vault',
      category: 'Bóveda',
      label: 'Cambiar o crear bóveda',
      keywords: 'vault boveda cambiar abrir crear nueva',
      disabled: false,
      run: () => void openVaultSwitcher(),
    },
    {
      id: 'open-sync',
      category: 'Git',
      label: 'Abrir panel de configuración y sincronización Git',
      keywords: 'github pull push commit',
      disabled: !snapshot,
      run: () => openSyncModal(),
    },
    {
      id: 'sync-now',
      category: 'Git',
      label: 'Sincronizar bóveda ahora',
      keywords: 'pull push commit',
      disabled: !snapshot || syncing,
      run: () => handleSyncNow(),
    },
    {
      id: 'canvas-new',
      category: 'Lienzo Visual',
      label: 'Canvas: Crear nuevo lienzo (.canvas)',
      keywords: 'canvas lienzo diagrama mapa json',
      disabled: !snapshot,
      run: () => openCreateCanvasDialog(),
    },
    {
      id: 'canvas-open',
      category: 'Lienzo Visual',
      label:
        centerView === 'canvas'
          ? 'Canvas: Volver al Editor Markdown'
          : 'Canvas: Abrir lienzo activo o primero disponible',
      keywords: 'canvas lienzo diagrama abrir',
      disabled: !snapshot || (centerView !== 'canvas' && !activeCanvas && snapshot.canvases.length === 0),
      run: () => {
        if (centerView === 'canvas') {
          centerView = 'editor';
        } else if (activeCanvas) {
          centerView = 'canvas';
        } else if (snapshot && snapshot.canvases.length > 0) {
          void handleSelectCanvas(snapshot.canvases[0].relative_path);
        }
      },
    },
    {
      id: 'open-graph-global',
      category: 'Grafo',
      label: 'Abrir Vista de Grafo Global',
      keywords: 'graph grafo global red conexiones nodos',
      run: () => {
        centerView = 'graph-global';
      },
    },
    {
      id: 'open-graph-local',
      category: 'Grafo',
      label: 'Abrir Vista de Grafo Local (nota actual)',
      keywords: 'graph grafo local vecindario vecinos',
      run: () => {
        centerView = 'graph-local';
      },
    },
    {
      id: 'close-graph',
      category: 'Grafo',
      label: 'Volver al Editor Markdown',
      keywords: 'editor markdown cerrar grafo',
      run: () => {
        centerView = 'editor';
      },
    },
    {
      id: 'open-settings',
      category: 'Ajustes',
      label: 'Configurar Notas Diarias, Plantillas, DataviewJS e IA',
      keywords: 'settings dataviewjs sandbox seguridad ia claude gemini',
      run: () => openSettingsModal(),
    },
    {
      id: 'ai-copilot-open',
      category: 'Copiloto IA',
      label: 'Abrir Copiloto PKM y Auto-Conexión IA',
      shortcut: 'Ctrl+Shift+A',
      keywords: 'ia ai copilot agente mcp claude gemini autoconectar',
      disabled: !snapshot,
      run: () => openAiCopilot('note'),
    },
    {
      id: 'ai-copilot-note',
      category: 'Copiloto IA',
      label: 'Auto-conectar nota activa ([[wikilinks]] + #tags + frontmatter)',
      keywords: 'ia ai enlazar wikilinks tags frontmatter nota',
      disabled: !snapshot || !activeNote,
      run: () => openAiCopilot('note'),
    },
    {
      id: 'ai-copilot-batch',
      category: 'Copiloto IA',
      label: 'Analizar y conectar carpeta Inbox/ en lote',
      keywords: 'ia ai lote batch inbox fuentes youtube',
      disabled: !snapshot,
      run: () => openAiCopilot('batch'),
    },
    {
      id: 'ai-copilot-canvas',
      category: 'Copiloto IA',
      label: 'Generar Mapa .canvas de Conocimiento desde fuentes',
      keywords: 'ia ai canvas mapa diagrama conocimiento',
      disabled: !snapshot,
      run: () => openAiCopilot('canvas'),
    },
    {
      id: 'ai-copilot-chat',
      category: 'Copiloto IA',
      label: 'Abrir Chat con el Copiloto PKM',
      keywords: 'ia ai chat preguntar boveda resumen',
      disabled: !snapshot,
      run: () => openAiCopilot('chat'),
    },
    {
      id: 'ai-copilot-provider',
      category: 'Copiloto IA',
      label: 'Configurar Proveedor de IA (Claude CLI / Gemini / Offline / MCP)',
      keywords: 'ia ai proveedor claude gemini mcp keyring api key',
      disabled: !snapshot,
      run: () => openAiCopilot('provider'),
    },
  ]);

  function handleGlobalKeydown(event: KeyboardEvent) {
    const isMod = event.ctrlKey || event.metaKey;
    if (isMod && event.shiftKey && event.key.toLowerCase() === 'a') {
      event.preventDefault();
      if (snapshot) {
        if (aiCopilotOpen) {
          aiCopilotOpen = false;
        } else {
          openAiCopilot('note');
        }
      }
      return;
    }
    if (isMod && event.key.toLowerCase() === 'p') {
      event.preventDefault();
      paletteOpen = !paletteOpen;
      return;
    }
    if (isMod && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      searchOpen = !searchOpen;
      return;
    }
    if (isMod && event.key.toLowerCase() === 'n') {
      event.preventDefault();
      if (snapshot) {
        void openCreateDialog();
      }
      return;
    }
    if (event.altKey && !isMod && event.key.toLowerCase() === 'd') {
      event.preventDefault();
      if (snapshot) {
        void handleOpenDailyNote();
      }
      return;
    }
    if (event.altKey && !isMod && event.key.toLowerCase() === 't') {
      event.preventDefault();
      if (snapshot) {
        void openTemplateModal(activeNote ? 'insert' : 'create');
      }
      return;
    }
    if (event.key === 'Escape') {
      if (promptDialog) promptDialog = null;
      if (settingsOpen) settingsOpen = false;
      if (templateModalOpen) templateModalOpen = false;
      if (aiCopilotOpen) aiCopilotOpen = false;
    }
  }

  onMount(async () => {
    try {
      const savedTheme = localStorage.getItem(THEME_STORAGE_KEY);
      if (savedTheme === 'light' || savedTheme === 'dark') {
        theme = savedTheme;
      }
      const savedSettingsRaw = localStorage.getItem(SETTINGS_STORAGE_KEY);
      if (savedSettingsRaw) {
        const parsed = JSON.parse(savedSettingsRaw) as Partial<WorkspaceSettings>;
        const loc = parsed.backlinksLocation;
        settings = {
          dailyFolder: parsed.dailyFolder?.trim() || DEFAULT_SETTINGS.dailyFolder,
          templatesFolder: parsed.templatesFolder?.trim() || DEFAULT_SETTINGS.templatesFolder,
          youtubeInboxFolder:
            parsed.youtubeInboxFolder?.trim() || DEFAULT_SETTINGS.youtubeInboxFolder,
          dailyTemplatePath: parsed.dailyTemplatePath?.trim() || '',
          enableDataviewJs: parsed.enableDataviewJs === true,
          backlinksLocation:
            loc === 'right-panel' || loc === 'document-footer' || loc === 'pinned-tab'
              ? loc
              : DEFAULT_SETTINGS.backlinksLocation,
          backlinksExcludedFiles: Array.isArray(parsed.backlinksExcludedFiles)
            ? parsed.backlinksExcludedFiles.map((s) => String(s).trim()).filter(Boolean)
            : [],
        };
      }
    } catch {
      // Ignorar si localStorage falla
    }

    closeSidebarsOnMobile();

    const onAndroidShareEvent = () => {
      void handlePendingAndroidShare();
    };
    window.addEventListener('romsidian-android-share', onAndroidShareEvent);

    try {
      const lastVault = localStorage.getItem(LAST_VAULT_STORAGE_KEY)?.trim() ?? '';
      let opened = false;
      if (lastVault) {
        try {
          const snap = await openVault(lastVault);
          snapshot = snap;
          vaultPathInput = snap.info.root_path;
          saveRecentVault({
            name: snap.info.name,
            path: snap.info.root_path,
            is_default: false,
          });
          if (snap.notes.length > 0) {
            const preferred =
              snap.notes.find((n) => normalizeNoteKey(n.title) === 'índice general') ??
              snap.notes[0];
            activeNote = await readNote(preferred.relative_path);
          } else {
            activeNote = null;
          }
          void refreshSyncStatus();
          void refreshAiProviderStatus();
          opened = true;
        } catch {
          opened = false;
        }
      }
      if (!opened) {
        const defaultPath = await getDefaultVaultPath();
        if (defaultPath) {
          vaultPathInput = defaultPath;
          await handleOpenVault(defaultPath);
        }
      }
      await handlePendingAndroidShare();
    } catch (err) {
      errorMessage = String(err);
    }

    return () => {
      window.removeEventListener('romsidian-android-share', onAndroidShareEvent);
    };
  });
</script>

<svelte:window onkeydown={handleGlobalKeydown} />

<div class="app-shell">
  <header class="topbar">
    <div class="brand-group">
      <button
        type="button"
        class="icon-btn"
        class:active={showLeftSidebar}
        onclick={() => (showLeftSidebar = !showLeftSidebar)}
        title="Mostrar/ocultar explorador"
      >
        ☰
      </button>
      <div class="brand">
        <span class="brand-logo">◆</span>
        <h1>ROMSIDIAN</h1>
      </div>
    </div>

    <form
      class="vault-form"
      onsubmit={(e) => {
        e.preventDefault();
        void handleOpenVault(vaultPathInput);
      }}
    >
      <input
        type="text"
        bind:value={vaultPathInput}
        placeholder="Ruta de la bóveda..."
        aria-label="Ruta de la bóveda"
      />
      <button
        type="button"
        class="icon-btn"
        onclick={handlePickFolder}
        title="Navegar por las carpetas en el explorador de Windows"
      >
        📁 Explorar...
      </button>
      <button type="submit" class="icon-btn">Abrir Bóveda</button>
    </form>

    <div class="topbar-actions">
      <button
        type="button"
        class="icon-btn"
        onclick={() => void openVaultSwitcher()}
        title="Cambiar o crear bóveda"
      >
        🗂 Bóvedas
      </button>

      <button
        type="button"
        class="icon-btn"
        onclick={() => (paletteOpen = true)}
        title="Paleta de comandos (Ctrl+P)"
      >
        ⌘ Comandos <span class="shortcut">Ctrl+P</span>
      </button>

      <button
        type="button"
        class="icon-btn"
        onclick={() => (searchOpen = true)}
        disabled={!snapshot}
        title="Buscar con SQLite FTS5 (Ctrl+K)"
      >
        ⌕ Buscar <span class="shortcut">Ctrl+K</span>
      </button>

      <button
        type="button"
        class="icon-btn"
        class:sync-alert={conflictAlertCopies.length > 0}
        onclick={() => void openSyncModal()}
        disabled={!snapshot}
        title="Configurar y sincronizar bóveda vía Git"
      >
        ⟳ Sincronizar
        {#if syncDto && syncDto.status.modified_files.length > 0}
          <span class="sync-badge">{syncDto.status.modified_files.length}</span>
        {:else if conflictAlertCopies.length > 0}
          <span class="sync-badge warn">!</span>
        {/if}
      </button>

      <button
        type="button"
        class="icon-btn"
        class:active={aiCopilotOpen}
        onclick={() => openAiCopilot('note')}
        disabled={!snapshot}
        title="Abrir Copiloto PKM y Auto-Conexión IA (Ctrl+Shift+A)"
      >
        ✦ Copiloto IA <span class="shortcut">Ctrl+Shift+A</span>
      </button>

      <button
        type="button"
        class="icon-btn"
        onclick={toggleTheme}
        title="Alternar tema claro / oscuro"
      >
        {theme === 'dark' ? '☀ Claro' : '☾ Oscuro'}
      </button>

      <button
        type="button"
        class="icon-btn"
        class:active={showRightSidebar}
        onclick={() => (showRightSidebar = !showRightSidebar)}
        title="Mostrar/ocultar panel de backlinks y propiedades"
      >
        Backlinks
      </button>
    </div>
  </header>

  {#if errorMessage}
    <div class="banner error" role="alert">
      <span>{errorMessage}</span>
      <button type="button" onclick={() => (errorMessage = null)}>Cerrar</button>
    </div>
  {:else if conflictAlertCopies.length > 0}
    <div class="banner warn" role="alert">
      <div class="conflict-banner-body">
        <strong>Conflicto resuelto sin pérdida de datos:</strong>
        {#each conflictAlertCopies as copyPath}
          <button
            type="button"
            class="conflict-chip"
            onclick={() => void handleSelectNote(copyPath)}
          >
            {copyPath}
          </button>
        {/each}
      </div>
      <button type="button" onclick={() => (conflictAlertCopies = [])}>Descartar</button>
    </div>
  {:else if statusNotice}
    <div class="banner info" role="status">
      <span>{statusNotice}</span>
    </div>
  {/if}

  <main
    class="workbench"
    class:hide-left={!showLeftSidebar}
    class:hide-right={!showRightSidebar}
  >
    <aside class="ribbon" aria-label="Barra rápida de acciones">
      <div class="ribbon-group">
        <button
          type="button"
          class="ribbon-btn"
          onclick={() => (paletteOpen = true)}
          title="Paleta de comandos (Ctrl+P)"
          aria-label="Paleta de comandos"
        >
          ⌘
        </button>
        <button
          type="button"
          class="ribbon-btn"
          onclick={() => void handleOpenDailyNote()}
          disabled={!snapshot}
          title="Abrir o crear Nota Diaria de hoy (Alt+D)"
          aria-label="Nota Diaria"
        >
          📅
        </button>
        <button
          type="button"
          class="ribbon-btn"
          onclick={() => void openTemplateModal(activeNote ? 'insert' : 'create')}
          disabled={!snapshot}
          title="Insertar o crear desde plantilla (Alt+T)"
          aria-label="Plantillas"
        >
          📄
        </button>
        <button
          type="button"
          class="ribbon-btn"
          onclick={() => void openCreateDialog()}
          disabled={!snapshot}
          title="Nueva nota Markdown (Ctrl+N)"
          aria-label="Nueva nota"
        >
          ＋
        </button>
        <button
          type="button"
          class="ribbon-btn"
          onclick={() => (searchOpen = true)}
          disabled={!snapshot}
          title="Buscar en la bóveda (Ctrl+K)"
          aria-label="Buscar"
        >
          ⌕
        </button>
        <button
          type="button"
          class="ribbon-btn"
          class:active={centerView === 'canvas'}
          onclick={() => {
            if (centerView === 'canvas') {
              centerView = 'editor';
            } else if (activeCanvas) {
              centerView = 'canvas';
            } else if (snapshot && snapshot.canvases.length > 0) {
              void handleSelectCanvas(snapshot.canvases[0].relative_path);
            } else {
              void openCreateCanvasDialog();
            }
          }}
          disabled={!snapshot}
          title="Abrir o crear Lienzo Visual (.canvas)"
          aria-label="Lienzo Visual Canvas"
        >
          ⊞
        </button>
        <button
          type="button"
          class="ribbon-btn"
          class:active={centerView === 'graph-global'}
          onclick={() =>
            (centerView = centerView === 'graph-global' ? 'editor' : 'graph-global')}
          disabled={!snapshot}
          title="Vista de Grafo Global"
          aria-label="Grafo Global"
        >
          ◎
        </button>
        <button
          type="button"
          class="ribbon-btn"
          class:active={centerView === 'graph-local'}
          onclick={() =>
            (centerView = centerView === 'graph-local' ? 'editor' : 'graph-local')}
          disabled={!snapshot || !activeNote}
          title="Vista de Grafo Local de la nota activa"
          aria-label="Grafo Local"
        >
          ⌖
        </button>
        <button
          type="button"
          class="ribbon-btn"
          class:active={aiCopilotOpen}
          onclick={() => openAiCopilot('note')}
          disabled={!snapshot}
          title="Copiloto PKM y Auto-Conexión IA (Ctrl+Shift+A)"
          aria-label="Copiloto IA"
        >
          ✦
        </button>
      </div>

      <div class="ribbon-group">
        <button
          type="button"
          class="ribbon-btn"
          class:alert={conflictAlertCopies.length > 0}
          onclick={() => void openSyncModal()}
          disabled={!snapshot}
          title="Sincronización Git"
          aria-label="Sincronización Git"
        >
          ⟳
        </button>
        <button
          type="button"
          class="ribbon-btn"
          onclick={() => void openSettingsModal()}
          title="Ajustes (Notas diarias, plantillas, DataviewJS)"
          aria-label="Ajustes"
        >
          ⚙
        </button>
      </div>
    </aside>

    {#if showLeftSidebar}
      <div class="pane-left">
        <FileTreeSidebar
          {snapshot}
          activePath={activeNote?.relative_path ?? null}
          activeCanvasPath={activeCanvas?.relative_path ?? null}
          {activeTag}
          {tagFilteredNotes}
          onSelectNote={handleSelectNote}
          onSelectCanvas={handleSelectCanvas}
          onCreateNote={openCreateDialog}
          onCreateFolder={() => {
            promptDialog = { kind: 'create-folder', value: '' };
          }}
          onCreateCanvas={openCreateCanvasDialog}
          onMoveItem={handleMoveItem}
          onRefreshVault={handleRefreshVault}
          onRequestRename={openRenameDialog}
          onRequestDelete={openDeleteDialog}
          onRequestDeleteFolder={openDeleteFolderDialog}
          onRequestRenameCanvas={openRenameCanvasDialog}
          onRequestDeleteCanvas={openDeleteCanvasDialog}
          onSelectTag={handleSelectTag}
        />
      </div>
    {/if}

    <div class="pane-center">
      {#if centerView === 'editor'}
        <MarkdownEditor
          bind:this={editorRef}
          note={activeNote}
          notes={snapshot?.notes ?? []}
          {knownNoteKeys}
          {theme}
          {saving}
          enableDataviewJs={settings.enableDataviewJs}
          backlinksLocation={settings.backlinksLocation}
          backlinksExcludedFiles={settings.backlinksExcludedFiles}
          {backlinksRefreshToken}
          onSave={handleSaveNote}
          onOpenWikilink={handleOpenWikilink}
          onRenameRequest={openRenameDialog}
          onOpenNote={handleSelectNote}
          onLinkMention={handleLinkUnlinkedMention}
          onUpdateExcludedFiles={handleUpdateExcludedFiles}
          onChangeBacklinksLocation={handleChangeBacklinksLocation}
        />
      {:else if centerView === 'canvas' && activeCanvas}
        {#key activeCanvas.relative_path}
          <CanvasEditor
            canvas={activeCanvas}
            notes={snapshot?.notes ?? []}
            {theme}
            {saving}
            onSave={handleSaveCanvas}
            onOpenNote={handleOpenNoteFromCanvas}
            onConvertTextToNote={handleConvertCanvasTextToNote}
            onClose={() => (centerView = 'editor')}
          />
        {/key}
      {:else}
        <GraphView
          activeNotePath={activeNote?.relative_path ?? null}
          {theme}
          refreshToken={backlinksRefreshToken}
          mode={centerView === 'graph-local' ? 'local' : 'global'}
          onModeChange={(next) => (centerView = next === 'local' ? 'graph-local' : 'graph-global')}
          onOpenNote={handleOpenNoteFromGraph}
          onOpenWikilink={handleOpenWikilinkFromGraph}
          onClose={() => (centerView = 'editor')}
        />
      {/if}
    </div>

    {#if showRightSidebar}
      <div class="pane-right">
        <BacklinksPanel
          note={activeNote}
          pinnedNote={pinnedBacklinkNote}
          notes={snapshot?.notes ?? []}
          {knownNoteKeys}
          backlinksLocation={settings.backlinksLocation}
          excludedFiles={settings.backlinksExcludedFiles}
          isPinned={isBacklinksPinned}
          refreshToken={backlinksRefreshToken}
          onOpenNote={handleSelectNote}
          onOpenWikilink={handleOpenWikilink}
          onSelectTag={(tag) => void handleSelectTag(tag)}
          onLinkMention={handleLinkUnlinkedMention}
          onUpdateExcludedFiles={handleUpdateExcludedFiles}
          onChangeLocation={handleChangeBacklinksLocation}
          onTogglePin={handleToggleBacklinksPin}
          onPinCurrentNote={handlePinCurrentNote}
        />
      </div>
    {/if}
  </main>

  <footer class="statusbar">
    <span>
      {#if snapshot}
        Bóveda: <strong>{snapshot.info.name}</strong> · {snapshot.index_stats.total_notes} notas
        indexadas
        {#if syncDto}
          · Git: <strong>{syncDto.status.branch}</strong>
          {#if syncDto.status.modified_files.length > 0}
            ({syncDto.status.modified_files.length} pendiente/s)
          {:else}
            (limpio)
          {/if}
        {/if}
      {:else}
        Sin bóveda abierta
      {/if}
    </span>
    <span>
      {#if activeNote}
        {activeNote.wikilinks.length} enlaces · {activeNote.backlinks.length} backlinks ·
        {activeNote.tags.length} etiquetas
      {:else}
        Listo
      {/if}
    </span>
  </footer>
</div>

<CommandPaletteModal
  open={paletteOpen}
  commands={paletteCommands}
  onClose={() => (paletteOpen = false)}
/>

<SearchModal
  open={searchOpen}
  onClose={() => (searchOpen = false)}
  onSelectResult={handleSelectNote}
/>

<SyncModal
  open={syncOpen}
  {syncDto}
  lastReport={syncReport}
  loading={syncLoading}
  {syncing}
  errorMsg={syncError}
  onClose={() => (syncOpen = false)}
  onRefreshStatus={refreshSyncStatus}
  onSaveConfig={handleSaveSyncConfig}
  onSyncNow={handleSyncNow}
  onOpenConflictFile={(path) => void handleSelectNote(path)}
/>

<VaultSwitcherModal
  open={vaultSwitcherOpen}
  activeVaultPath={snapshot?.info.root_path ?? null}
  activeVaultName={snapshot?.info.name ?? null}
  vaults={availableVaults}
  loading={vaultSwitcherLoading}
  errorMsg={vaultSwitcherError}
  onClose={() => (vaultSwitcherOpen = false)}
  onSelectVault={async (path) => {
    await handleOpenVault(path);
    vaultSwitcherOpen = false;
  }}
  onCreateVault={handleCreateLocalVault}
  onPickFolder={async () => {
    await handlePickFolder();
    vaultSwitcherOpen = false;
  }}
  onRemoveRecent={removeRecentVault}
  onOpenSync={() => void openSyncModal()}
/>

<AiCopilotPanel
  open={aiCopilotOpen}
  initialTab={aiCopilotTab}
  vaultPath={snapshot?.info.root_path ?? null}
  {activeNote}
  defaultInboxFolder={settings.youtubeInboxFolder}
  {knownNoteKeys}
  onClose={() => (aiCopilotOpen = false)}
  onOpenNote={handleSelectNote}
  onOpenWikilink={handleOpenWikilink}
  onApplied={handleAiApplied}
  onCanvasCreated={handleKnowledgeCanvasCreated}
  onCreateNoteFromChat={handleCreateNoteFromChat}
  onProviderStatusChange={(status) => (aiProviderStatus = status)}
/>

{#if promptDialog}
  <div
    class="dialog-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) promptDialog = null;
    }}
  >
    <div class="dialog-box" role="dialog" aria-modal="true">
      {#if promptDialog.kind === 'create'}
        <form onsubmit={submitPromptDialog}>
          <h3>Nueva nota Markdown</h3>
          <p>Puedes incluir subcarpetas, por ejemplo: <code>proyectos/Idea.md</code></p>
          <input
            bind:this={dialogInputEl}
            type="text"
            bind:value={promptDialog.value}
            placeholder="Nombre de la nota..."
            required
          />
          <label class="dialog-checkbox-label">
            <input type="checkbox" bind:checked={promptDialog.withFrontmatter} />
            <span>Generar cabecera YAML / Frontmatter automáticamente</span>
          </label>
          <div class="dialog-actions">
            <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
              Cancelar
            </button>
            <button type="submit" class="icon-btn primary">Crear nota</button>
          </div>
        </form>
      {:else if promptDialog.kind === 'create-folder'}
        <form onsubmit={submitPromptDialog}>
          <h3>Nueva Carpeta</h3>
          <p>Nombre de la carpeta o subcarpeta (por ejemplo: <code>Proyectos</code> o <code>Trabajo/2026</code>)</p>
          <input
            bind:this={dialogInputEl}
            type="text"
            bind:value={promptDialog.value}
            placeholder="Nombre de la carpeta..."
            required
          />
          <div class="dialog-actions">
            <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
              Cancelar
            </button>
            <button type="submit" class="icon-btn primary">Crear carpeta</button>
          </div>
        </form>
      {:else if promptDialog.kind === 'rename'}
        <form onsubmit={submitPromptDialog}>
          <h3>Renombrar o mover nota</h3>
          <p>Origen: <code>{promptDialog.oldPath}</code></p>
          <input
            bind:this={dialogInputEl}
            type="text"
            bind:value={promptDialog.value}
            required
          />
          <div class="dialog-actions">
            <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
              Cancelar
            </button>
            <button type="submit" class="icon-btn primary">Guardar nombre</button>
          </div>
        </form>
      {:else if promptDialog.kind === 'delete'}
        <h3>Eliminar nota</h3>
        <p>
          ¿Seguro que quieres eliminar <code>{promptDialog.path}</code> del disco? Esta
          acción borra el archivo <code>.md</code>.
        </p>
        <div class="dialog-actions">
          <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
            Cancelar
          </button>
          <button type="button" class="icon-btn danger" onclick={confirmDeleteNote}>
            Eliminar archivo
          </button>
        </div>
      {:else if promptDialog.kind === 'delete-folder'}
        <h3>Eliminar carpeta</h3>
        <p>
          ¿Seguro que quieres eliminar la carpeta <code>{promptDialog.path}</code> y todo su contenido? Esta acción borra la carpeta y las notas que tenga adentro.
        </p>
        <div class="dialog-actions">
          <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
            Cancelar
          </button>
          <button type="button" class="icon-btn danger" onclick={confirmDeleteFolder}>
            Eliminar carpeta
          </button>
        </div>
      {:else if promptDialog.kind === 'create-canvas'}
        <form onsubmit={submitPromptDialog}>
          <h3>Nuevo Lienzo Visual (.canvas)</h3>
          <p>Archivo compatible con JSON Canvas 1.0, por ejemplo: <code>Mapas/Proyecto.canvas</code></p>
          <input
            bind:this={dialogInputEl}
            type="text"
            bind:value={promptDialog.value}
            placeholder="Nombre del lienzo..."
            required
          />
          <div class="dialog-actions">
            <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
              Cancelar
            </button>
            <button type="submit" class="icon-btn primary">Crear lienzo</button>
          </div>
        </form>
      {:else if promptDialog.kind === 'rename-canvas'}
        <form onsubmit={submitPromptDialog}>
          <h3>Renombrar o mover lienzo</h3>
          <p>Origen: <code>{promptDialog.oldPath}</code></p>
          <input
            bind:this={dialogInputEl}
            type="text"
            bind:value={promptDialog.value}
            required
          />
          <div class="dialog-actions">
            <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
              Cancelar
            </button>
            <button type="submit" class="icon-btn primary">Guardar nombre</button>
          </div>
        </form>
      {:else if promptDialog.kind === 'delete-canvas'}
        <h3>Eliminar lienzo visual</h3>
        <p>
          ¿Seguro que quieres eliminar <code>{promptDialog.path}</code> del disco? Esta
          acción borra el archivo <code>.canvas</code>.
        </p>
        <div class="dialog-actions">
          <button type="button" class="icon-btn" onclick={() => (promptDialog = null)}>
            Cancelar
          </button>
          <button type="button" class="icon-btn danger" onclick={confirmDeleteNote}>
            Eliminar lienzo
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if templateModalOpen}
  <div
    class="dialog-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) templateModalOpen = false;
    }}
  >
    <div class="dialog-box wide" role="dialog" aria-modal="true" aria-label="Plantillas Markdown">
      <div class="modal-head-row">
        <h3>Plantillas ({settings.templatesFolder}/)</h3>
        <div class="mode-switch">
          <button
            type="button"
            class="icon-btn"
            class:active={templateModalMode === 'insert'}
            disabled={!activeNote}
            onclick={() => (templateModalMode = 'insert')}
          >
            Insertar en actual
          </button>
          <button
            type="button"
            class="icon-btn"
            class:active={templateModalMode === 'create'}
            onclick={() => (templateModalMode = 'create')}
          >
            Crear nota nueva
          </button>
        </div>
      </div>

      {#if templateModalMode === 'create'}
        <label class="field-label">
          Ruta de la nueva nota (se sustituyen <code>{'{{date}}'}</code>, <code>{'{{time}}'}</code> y <code>{'{{title}}'}</code>)
          <input
            type="text"
            bind:value={templateNewNotePath}
            placeholder="ej. proyectos/Plan Q2.md"
          />
        </label>
      {:else}
        <p>
          Haz clic en una plantilla para insertarla en <code>{activeNote?.relative_path}</code>.
        </p>
      {/if}

      {#if templatesLoading}
        <p>Cargando plantillas…</p>
      {:else if availableTemplates.length === 0}
        <p>
          No hay archivos <code>.md</code> en <code>{settings.templatesFolder}/</code>. Crea una nota ahí con variables como <code>{'{{date}}'}</code>, <code>{'{{time}}'}</code> o <code>{'{{title}}'}</code>.
        </p>
      {:else}
        <div class="template-list">
          {#each availableTemplates as tpl (tpl.relative_path)}
            <button
              type="button"
              class="template-item"
              onclick={() => void handleApplyTemplateChoice(tpl.relative_path)}
            >
              <span>{tpl.name}</span>
              <code>{tpl.relative_path}</code>
            </button>
          {/each}
        </div>
      {/if}

      <div class="dialog-actions">
        <button type="button" class="icon-btn" onclick={() => (templateModalOpen = false)}>
          Cerrar
        </button>
      </div>
    </div>
  </div>
{/if}

{#if settingsOpen}
  <div
    class="dialog-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) settingsOpen = false;
    }}
  >
    <div class="dialog-box wide" role="dialog" aria-modal="true" aria-label="Ajustes de bóveda">
      <h3>Ajustes de ROMSIDIAN v0.5</h3>
      <p>Configura carpetas de Notas Diarias, Plantillas, seguridad de Dataview y Proveedor IA.</p>

      <label class="field-label">
        Carpeta de Notas Diarias
        <input
          type="text"
          value={settings.dailyFolder}
          oninput={(e) =>
            persistSettings({
              ...settings,
              dailyFolder: (e.currentTarget as HTMLInputElement).value.trim() || 'daily',
            })}
          placeholder="daily"
        />
      </label>

      <label class="field-label">
        Carpeta de Plantillas
        <input
          type="text"
          value={settings.templatesFolder}
          oninput={(e) =>
            persistSettings({
              ...settings,
              templatesFolder:
                (e.currentTarget as HTMLInputElement).value.trim() || 'templates',
            })}
          placeholder="templates"
        />
      </label>

      <label class="field-label">
        Carpeta Inbox (videos compartidos / YouTube)
        <input
          type="text"
          value={settings.youtubeInboxFolder}
          oninput={(e) =>
            persistSettings({
              ...settings,
              youtubeInboxFolder:
                (e.currentTarget as HTMLInputElement).value.trim() || 'Inbox',
            })}
          placeholder="Inbox"
        />
      </label>

      <label class="field-label">
        Plantilla por defecto para Nota Diaria
        <select
          value={settings.dailyTemplatePath}
          onchange={(e) =>
            persistSettings({
              ...settings,
              dailyTemplatePath: (e.currentTarget as HTMLSelectElement).value,
            })}
        >
          <option value="">Automática (busca templates/Diaria.md o usa formato estándar)</option>
          {#each availableTemplates as tpl (tpl.relative_path)}
            <option value={tpl.relative_path}>{tpl.name} ({tpl.relative_path})</option>
          {/each}
        </select>
      </label>

      <label class="field-label">
        Ubicación de Retroenlaces
        <select
          value={settings.backlinksLocation}
          onchange={(e) =>
            handleChangeBacklinksLocation(
              (e.currentTarget as HTMLSelectElement).value as BacklinksLocation,
            )}
        >
          <option value="right-panel">Panel lateral derecho</option>
          <option value="document-footer">Incrustado al pie del documento activo</option>
          <option value="pinned-tab">Pestaña fijada dedicada</option>
        </select>
      </label>

      <label class="field-label">
        Excluded files en Retroenlaces (separados por coma)
        <input
          type="text"
          value={settings.backlinksExcludedFiles.join(', ')}
          onchange={(e) =>
            handleUpdateExcludedFiles(
              (e.currentTarget as HTMLInputElement).value
                .split(/[\n,;]+/)
                .map((s) => s.trim())
                .filter(Boolean),
            )}
          placeholder="templates/, daily/archivo*, *borrador*"
        />
      </label>

      <label class="toggle-card">
        <input
          type="checkbox"
          checked={settings.enableDataviewJs}
          onchange={(e) =>
            persistSettings({
              ...settings,
              enableDataviewJs: (e.currentTarget as HTMLInputElement).checked,
            })}
        />
        <div>
          <strong>Permitir bloques <code>```dataviewjs</code> (experimental)</strong>
          <small>
            Desactivado por defecto por seguridad. Actívalo solo si confías en todas las notas de la bóveda. El motor declarativo <code>```dataview</code> (DQL) funciona siempre en sandbox.
          </small>
        </div>
      </label>

      <div class="settings-ai-card">
        <label class="field-label">
          Proveedor de Agente IA / Copiloto PKM (v0.5)
          <select
            disabled={!snapshot || aiQuickSaving}
            value={aiProviderStatus?.active_config.provider ?? 'deterministic'}
            onchange={(e) =>
              void handleQuickProviderChange(
                (e.currentTarget as HTMLSelectElement).value as AiProviderKind,
              )}
          >
            <option value="deterministic">
              Motor Determinista Local (Rust + SQLite FTS5 · 0 tokens · Offline)
            </option>
            <option value="claude_cli">
              Claude Code CLI (claude -p · suscripción Windows sin pago por API)
              {aiProviderStatus?.claude_cli_available ? ' ✓ Detectado' : ''}
            </option>
            <option value="gemini_cli">
              Gemini CLI (gemini -p · PC)
              {aiProviderStatus?.gemini_cli_available ? ' ✓ Detectado' : ''}
            </option>
            <option value="gemini_http">
              Google Gemini API HTTP (PC + Android · AI Studio Free Tier)
            </option>
            <option value="compatible_http">
              Endpoint Compatible OpenAI / Ollama HTTP (/v1/chat/completions)
            </option>
          </select>
        </label>
        <div class="settings-ai-foot">
          <small>
            Modelo actual: <code>{aiProviderStatus?.active_config.model ?? 'deterministic-rust-v1'}</code>
          </small>
          <button
            type="button"
            class="icon-btn"
            disabled={!snapshot}
            onclick={() => {
              settingsOpen = false;
              openAiCopilot('provider');
            }}
          >
            ✦ Claves API (Llavero SO), Modelo y MCP…
          </button>
        </div>
      </div>

      <div class="dialog-actions">
        <button type="button" class="icon-btn primary" onclick={() => (settingsOpen = false)}>
          Listo
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .app-shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
  }

  .topbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: max(8px, env(safe-area-inset-top)) 12px 8px;
    background: var(--bg-sidebar);
    border-bottom: 1px solid var(--border-subtle);
  }

  .brand-group {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .brand-logo {
    color: var(--accent);
    font-size: 16px;
  }

  .brand h1 {
    margin: 0;
    font-size: 14px;
    letter-spacing: 0.08em;
    font-weight: 750;
  }

  .vault-form {
    flex: 1;
    min-width: 220px;
    max-width: 520px;
    display: flex;
    gap: 6px;
  }

  .vault-form input {
    flex: 1;
    min-width: 0;
    padding: 5px 10px;
    border-radius: 7px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    font-size: 12.5px;
  }

  .vault-form input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .topbar-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .shortcut {
    font-size: 10.5px;
    color: var(--text-muted);
    padding: 1px 4px;
    border-radius: 4px;
    border: 1px solid var(--border-subtle);
  }

  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 14px;
    font-size: 12.5px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .banner.error {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .banner.info {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .banner.warn {
    background: var(--pending-soft);
    color: var(--pending);
  }

  .conflict-banner-body {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .conflict-chip {
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--pending);
    background: var(--bg-surface);
    color: var(--pending);
    font-size: 11.5px;
    cursor: pointer;
  }

  .sync-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 17px;
    height: 17px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--accent);
    color: #fff;
    font-size: 10.5px;
    font-weight: 700;
  }

  .sync-badge.warn,
  .icon-btn.sync-alert {
    border-color: var(--pending);
  }

  .sync-badge.warn {
    background: var(--pending);
  }

  .banner button {
    border: none;
    background: transparent;
    font-weight: 600;
  }

  .workbench {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 44px 270px minmax(0, 1fr) 290px;
  }

  .workbench.hide-left {
    grid-template-columns: 44px minmax(0, 1fr) 290px;
  }

  .workbench.hide-right {
    grid-template-columns: 44px 270px minmax(0, 1fr);
  }

  .workbench.hide-left.hide-right {
    grid-template-columns: 44px minmax(0, 1fr);
  }

  .ribbon {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    align-items: center;
    padding: 8px 4px;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border-subtle);
  }

  .ribbon-group {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .ribbon-btn {
    width: 32px;
    height: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    color: var(--text-secondary);
    font-size: 15px;
  }

  .ribbon-btn:hover:not(:disabled) {
    background: var(--bg-surface-hover);
    color: var(--text-primary);
    border-color: var(--border-subtle);
  }

  .ribbon-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .ribbon-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: var(--accent);
  }

  .ribbon-btn.alert {
    color: var(--pending);
    border-color: var(--pending);
  }

  .pane-left,
  .pane-center,
  .pane-right {
    min-width: 0;
    min-height: 0;
    height: 100%;
    overflow: hidden;
  }

  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 4px 12px;
    font-size: 11.5px;
    color: var(--text-muted);
    background: var(--bg-sidebar);
    border-top: 1px solid var(--border-subtle);
  }

  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    background: var(--bg-backdrop);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
  }

  .dialog-box {
    width: min(440px, 100%);
    background: var(--bg-modal);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    padding: 18px;
    box-shadow: var(--shadow-lg);
  }

  .dialog-box h3 {
    margin: 0 0 8px;
    font-size: 16px;
  }

  .dialog-box p {
    margin: 0 0 12px;
    font-size: 13px;
    color: var(--text-secondary);
  }

  .dialog-box input {
    width: 100%;
    padding: 8px 10px;
    border-radius: 7px;
    border: 1px solid var(--border-strong);
    background: var(--bg-surface);
    margin-bottom: 14px;
  }

  .dialog-box input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .dialog-checkbox-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-primary);
    cursor: pointer;
    margin-bottom: 12px;
    user-select: none;
  }

  .dialog-checkbox-label input[type='checkbox'] {
    width: 16px;
    height: 16px;
    margin: 0;
    cursor: pointer;
    accent-color: var(--accent);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }

  .dialog-box.wide {
    width: min(520px, 100%);
    max-height: 90vh;
    overflow-y: auto;
  }

  .settings-ai-card {
    margin-top: 12px;
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }

  .settings-ai-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .settings-ai-foot small {
    color: var(--text-muted);
    font-size: 12px;
  }

  .modal-head-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 10px;
  }

  .modal-head-row h3 {
    margin: 0;
  }

  .mode-switch {
    display: flex;
    gap: 6px;
  }

  .field-label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin-bottom: 12px;
    font-size: 12.5px;
    color: var(--text-secondary);
  }

  .field-label input,
  .field-label select {
    width: 100%;
    padding: 7px 10px;
    border-radius: 7px;
    border: 1px solid var(--border-strong);
    background: var(--bg-surface);
    color: var(--text-primary);
    margin-bottom: 0;
  }

  .template-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 240px;
    overflow-y: auto;
    margin-bottom: 10px;
  }

  .template-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    width: 100%;
    text-align: left;
    padding: 9px 11px;
    border-radius: 8px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .template-item:hover {
    background: var(--bg-active);
    border-color: var(--accent);
  }

  .template-item code {
    font-size: 11.5px;
    color: var(--text-muted);
  }

  .toggle-card {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    cursor: pointer;
    font-size: 12.5px;
  }

  .toggle-card input[type='checkbox'] {
    width: auto;
    margin: 3px 0 0;
  }

  .toggle-card small {
    display: block;
    margin-top: 3px;
    color: var(--text-muted);
    line-height: 1.4;
  }

  @media (max-width: 900px) {
    .workbench,
    .workbench.hide-left,
    .workbench.hide-right,
    .workbench.hide-left.hide-right {
      grid-template-columns: 40px minmax(0, 1fr);
      position: relative;
    }

    .pane-left {
      position: absolute;
      inset: 0 auto 0 40px;
      width: min(290px, 85vw);
      z-index: 20;
      box-shadow: var(--shadow-lg);
    }

    .pane-right {
      position: absolute;
      inset: 0 0 0 auto;
      width: min(300px, 88vw);
      z-index: 20;
      box-shadow: var(--shadow-lg);
    }

    .shortcut {
      display: none;
    }

    .vault-form {
      flex: 1 1 100%;
      max-width: none;
      min-width: 0;
      order: 3;
    }

    .statusbar {
      padding-bottom: max(6px, env(safe-area-inset-bottom));
    }
  }
</style>
