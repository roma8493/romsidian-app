import { invoke } from '@tauri-apps/api/core';
import type {
  AiChatRequest,
  AiChatResponse,
  AiEnrichmentPlan,
  AiEnrichmentPreview,
  AiProviderConfig,
  AiProviderStatus,
  AutoConnectBatchResponse,
  AutoConnectNoteOutcome,
  AutoConnectRequest,
  BacklinkHit,
  BlockRef,
  CandidateConnection,
  CanvasDocument,
  CanvasSummary,
  ConnectionSignal,
  DailyNoteResult,
  DataviewInlineEvaluation,
  DataviewQueryResult,
  DataviewValue,
  FileTreeNode,
  FrontmatterKeyStat,
  GithubRepoInfo,
  GraphData,
  HeadingItem,
  IndexedField,
  KnowledgeCanvasRequest,
  KnowledgeCanvasResult,
  KnowledgeNoteEntry,
  LocalVaultEntry,
  NoteDetail,
  NoteSummary,
  SearchResult,
  SyncConfig,
  SyncReport,
  SyncStatusDto,
  TagCount,
  TemplateSummary,
  TransclusionResolution,
  UnlinkedMentionHit,
  VaultKnowledgeMap,
  VaultSnapshot,
  WikilinkReplacement,
} from './types';
import {
  clearAllWebSessionData,
  createWebVault,
  getActiveWebVault,
  getWebVaultSyncStatusDto,
  listUserGithubRepos,
  listWebVaults,
  markWebPathDeleted,
  markWebPathModified,
  saveWebSessionStore,
  saveWebVaultSyncConfig,
  switchWebVault,
  syncWebVaultWithGithub,
} from './webGithubSync';

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function isWebBrowserMode(): boolean {
  return !isTauriRuntime();
}


// Estado simulado en memoria únicamente cuando se abre en navegador web fuera de Tauri
const browserVaultNotes = new Map<string, string>([
  [
    'Índice General.md',
    `---
title: Índice General
tags:
  - inicio
  - arquitectura
---

# Índice General

Estado:: Activo
Prioridad:: Alta

Esta es la bóveda de prueba conectada con [[Arquitectura de RomSidian]], [[Análisis Macroeconómico]] y [[Sincronización Git]].

Nota actual: \`= this.title\` · Estado: \`= this.estado\`

![[Arquitectura de RomSidian#Núcleo]]

\`\`\`dataview
TABLE estado, prioridad, file.tags AS "Etiquetas"
SORT file.name ASC
\`\`\`

#inicio #romsidian/mvp
`,
  ],
  [
    'Análisis Macroeconómico.md',
    `---
title: Análisis Macroeconómico
tags:
  - economia/macro
  - inflacion
---

# Análisis Macroeconómico y Política Monetaria

Estado:: Revisión
Prioridad:: Media

Notas de estudio conectadas con [[Arquitectura de RomSidian]] y [[Índice General]].

- [ ] Actualizar serie histórica de IPC #economia/macro
- [x] Revisar informe trimestral

Proyección de inflación anual e indicadores.

#economia/macro #finanzas
`,
  ],
  [
    'Arquitectura de RomSidian.md',
    `---
title: Arquitectura de RomSidian
tags:
  - arquitectura
  - rust
---

# Arquitectura de RomSidian

Estado:: Producción

## Núcleo

El núcleo de ROMSIDIAN está escrito en Rust (SQLite FTS5 + git2) y se conecta con [[Sincronización Git]] y [[Índice General]]. ^nucleo-rust

#arquitectura #rust
`,
  ],
  [
    'templates/Diaria.md',
    `---
title: "{{title}}"
date: "{{date}}"
tags:
  - diaria
---

# {{title}}

Creada el {{date}} a las {{time}}.

Estado:: En curso

## Foco del día
- [ ] Revisar notas pendientes

## Bitácora
- 
`,
  ],
  [
    'templates/Proyecto.md',
    `---
title: "{{title}}"
tags:
  - proyecto
---

# {{title}}

Fecha:: {{date}}
Estado:: Borrador
Responsable:: 

## Objetivos
- 

## Próximos pasos
- [ ] Definir alcance inicial
`,
  ],
]);

const browserVaultCanvases = new Map<string, string>([
  [
    'Mapa de Conocimiento.canvas',
    JSON.stringify(
      {
        nodes: [
          {
            id: 'grp-core',
            type: 'group',
            x: -60,
            y: -40,
            width: 700,
            height: 410,
            color: '5',
            label: 'Arquitectura PKM Local-First',
          },
          {
            id: 'txt-intro',
            type: 'text',
            x: -20,
            y: 20,
            width: 280,
            height: 150,
            color: '4',
            text: '# Lienzo Visual ROMSIDIAN\n\nLas tarjetas de texto soportan **Markdown** libre y pueden convertirse a archivos `.md`.',
          },
          {
            id: 'file-note',
            type: 'file',
            x: 330,
            y: 20,
            width: 270,
            height: 150,
            color: '6',
            file: 'Bienvenida.md',
            subpath: '#Bienvenida a ROMSIDIAN',
            mediaKind: 'note',
          },
          {
            id: 'file-media',
            type: 'file',
            x: -20,
            y: 210,
            width: 280,
            height: 130,
            color: '3',
            file: 'assets/diagrama-pkm.svg',
            mediaKind: 'media',
          },
          {
            id: 'lnk-spec',
            type: 'link',
            x: 330,
            y: 210,
            width: 270,
            height: 130,
            color: '2',
            url: 'https://jsoncanvas.org/spec/1.0/',
          },
        ],
        edges: [
          {
            id: 'edge-1',
            fromNode: 'txt-intro',
            fromSide: 'right',
            fromEnd: 'none',
            toNode: 'file-note',
            toSide: 'left',
            toEnd: 'arrow',
            color: '4',
            label: 'Enlaza a nota',
          },
          {
            id: 'edge-2',
            fromNode: 'file-note',
            fromSide: 'bottom',
            fromEnd: 'arrow',
            toNode: 'lnk-spec',
            toSide: 'top',
            toEnd: 'arrow',
            color: '2',
            label: 'Especificación 1.0',
          },
        ],
      },
      null,
      2,
    ),
  ],
]);

const defaultBrowserSeedNotes = new Map<string, string>(browserVaultNotes.entries());
const defaultBrowserSeedCanvases = new Map<string, string>(browserVaultCanvases.entries());

function syncBrowserMapsFromActiveVault(): string {
  const activeVault = getActiveWebVault(defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
  browserVaultNotes.clear();
  for (const [k, v] of Object.entries(activeVault.notes)) {
    browserVaultNotes.set(k, v);
  }
  browserVaultCanvases.clear();
  for (const [k, v] of Object.entries(activeVault.canvases)) {
    browserVaultCanvases.set(k, v);
  }
  return activeVault.path;
}

function persistBrowserMapsToActiveVault(modifiedPath?: string, deletedPath?: string): void {
  const activeVault = getActiveWebVault(defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
  activeVault.notes = {};
  for (const [k, v] of browserVaultNotes.entries()) {
    activeVault.notes[k] = v;
  }
  activeVault.canvases = {};
  for (const [k, v] of browserVaultCanvases.entries()) {
    activeVault.canvases[k] = v;
  }
  if (modifiedPath) {
    markWebPathModified(activeVault, modifiedPath);
  }
  if (deletedPath) {
    markWebPathDeleted(activeVault, deletedPath);
  }
  saveWebSessionStore();
}

function buildBrowserFileTree(notes: NoteSummary[]): FileTreeNode[] {
  const rootChildren: FileTreeNode[] = [];
  const folderMap = new Map<string, { kind: 'folder'; name: string; relative_path: string; children: FileTreeNode[] }>();

  const ensureFolder = (folderPath: string) => {
    if (folderMap.has(folderPath)) return folderMap.get(folderPath)!;
    const parts = folderPath.split('/');
    const name = parts[parts.length - 1];
    const node = {
      kind: 'folder' as const,
      name,
      relative_path: folderPath,
      children: [] as FileTreeNode[],
    };
    folderMap.set(folderPath, node);
    if (parts.length === 1) {
      rootChildren.push(node);
    } else {
      const parentPath = parts.slice(0, -1).join('/');
      const parent = ensureFolder(parentPath);
      parent.children.push(node);
    }
    return node;
  };

  const sortedNotes = [...notes].sort((a, b) => a.relative_path.localeCompare(b.relative_path));
  for (const note of sortedNotes) {
    const normPath = note.relative_path.replace(/\\/g, '/');
    const lastSlash = normPath.lastIndexOf('/');
    const noteNode: FileTreeNode = {
      kind: 'note',
      ...note,
      title: normPath.replace(/\.md$/i, '').split('/').pop() || note.title,
    };
    if (lastSlash < 0) {
      rootChildren.push(noteNode);
    } else {
      const folderPath = normPath.slice(0, lastSlash);
      const folder = ensureFolder(folderPath);
      folder.children.push(noteNode);
    }
  }

  return rootChildren;
}

function buildBrowserTagCounts(): TagCount[] {
  const counts = new Map<string, number>();
  for (const content of browserVaultNotes.values()) {
    const inlineTags = Array.from(
      content.matchAll(/(?:^|\s)#([a-zA-Z0-9_áéíóúñÁÉÍÓÚÑ/-]+)/g),
    ).map((m) => m[1].toLowerCase());
    const fmTags = extractBrowserFrontmatterTags(content);
    const unique = new Set([...inlineTags, ...fmTags]);
    for (const tag of unique) {
      counts.set(tag, (counts.get(tag) ?? 0) + 1);
    }
  }
  return Array.from(counts.entries())
    .map(([tag, count]) => ({ tag, count }))
    .sort((a, b) => b.count - a.count || a.tag.localeCompare(b.tag));
}

function buildBrowserSnapshot(vaultPath?: string): VaultSnapshot {
  const activePath = syncBrowserMapsFromActiveVault();
  const effectivePath = vaultPath || activePath;
  const notes: NoteSummary[] = Array.from(browserVaultNotes.entries()).map(([rel, content]) => ({
    relative_path: rel,
    title: rel.replace(/\.md$/i, '').split('/').pop() || rel.replace(/\.md$/i, ''),
    modified_unix_secs: Math.floor(Date.now() / 1000),
    size_bytes: new TextEncoder().encode(content).length,
  }));
  const tags = buildBrowserTagCounts();
  return {
    info: {
      name: effectivePath.split(/[\\/]/).filter(Boolean).pop() || 'boveda-web',
      root_path: effectivePath,
      metadata_dir: `${effectivePath}/.romsidian`,
    },
    tree: buildBrowserFileTree(notes),
    notes,
    canvases: Array.from(browserVaultCanvases.keys()).map((rel) => ({
      relative_path: rel,
      title: rel.replace(/\.canvas$/i, '').split('/').pop() || rel,
      modified_unix_secs: Math.floor(Date.now() / 1000),
      size_bytes: browserVaultCanvases.get(rel)?.length ?? 2,
    })),
    tags,
    index_stats: {
      total_notes: notes.length,
      indexed_or_updated: notes.length,
      skipped_unchanged: 0,
      removed_deleted: 0,
    },
  };
}

function buildBrowserNoteDetail(relativePath: string): NoteDetail {
  syncBrowserMapsFromActiveVault();
  const content = browserVaultNotes.get(relativePath) ?? `# ${relativePath.replace(/\.md$/i, '')}\n`;
  const wikilinks = Array.from(content.matchAll(/\[\[([^\]|#]+)(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g)).map(
    (m) => ({
      target: m[1].trim(),
      heading: m[2]?.trim() ?? null,
      alias: m[3]?.trim() ?? null,
      line: 1,
      context: m[0],
      is_embed: false,
    }),
  );
  const inlineTags = Array.from(content.matchAll(/(?:^|\s)#([a-zA-Z0-9_áéíóúñÁÉÍÓÚÑ/-]+)/g)).map((m) =>
    m[1].toLowerCase(),
  );
  const fmTags = extractBrowserFrontmatterTags(content);
  const stem = (relativePath.replace(/\.md$/i, '').split('/').pop() || relativePath).toLowerCase();
  const backlinks: BacklinkHit[] = [];
  for (const [sourcePath, sourceContent] of browserVaultNotes.entries()) {
    if (sourcePath === relativePath) continue;
    if (sourceContent.toLowerCase().includes(`[[${stem}`)) {
      backlinks.push({
        source_path: sourcePath,
        source_title: sourcePath.replace(/\.md$/i, ''),
        target_raw: relativePath.replace(/\.md$/i, ''),
        alias: null,
        heading: null,
        line: 1,
        context: `Enlace hacia [[${relativePath.replace(/\.md$/i, '')}]]`,
        is_embed: false,
      });
    }
  }
  return {
    relative_path: relativePath,
    title: relativePath.replace(/\.md$/i, '').split('/').pop() || relativePath.replace(/\.md$/i, ''),
    content,
    modified_unix_secs: Math.floor(Date.now() / 1000),
    frontmatter: {},
    wikilinks,
    tags: Array.from(new Set([...fmTags, ...inlineTags])),
    backlinks,
  };
}

export async function getDefaultVaultPath(): Promise<string | null> {
  if (!isTauriRuntime()) {
    return getActiveWebVault(defaultBrowserSeedNotes, defaultBrowserSeedCanvases).path;
  }
  return invoke<string | null>('get_default_vault_path');
}

export async function listLocalVaults(): Promise<LocalVaultEntry[]> {
  if (!isTauriRuntime()) {
    return listWebVaults(defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
  }
  return invoke<LocalVaultEntry[]>('list_local_vaults');
}

export async function createLocalVault(name: string): Promise<LocalVaultEntry> {
  if (!isTauriRuntime()) {
    const created = createWebVault(name, defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
    syncBrowserMapsFromActiveVault();
    return created;
  }
  return invoke<LocalVaultEntry>('create_local_vault', { name });
}

export async function openVault(path: string): Promise<VaultSnapshot> {
  if (!isTauriRuntime()) {
    const active = switchWebVault(path, defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
    syncBrowserMapsFromActiveVault();
    return buildBrowserSnapshot(active.path);
  }
  return invoke<VaultSnapshot>('open_vault', { path });
}

export async function getVaultSnapshot(): Promise<VaultSnapshot> {
  if (!isTauriRuntime()) return buildBrowserSnapshot();
  return invoke<VaultSnapshot>('get_vault_snapshot');
}

export async function readNote(relativePath: string): Promise<NoteDetail> {
  if (!isTauriRuntime()) return buildBrowserNoteDetail(relativePath);
  return invoke<NoteDetail>('read_note', { relativePath });
}

export async function createNote(relativePath: string, content: string): Promise<NoteDetail> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const clean = relativePath.endsWith('.md') ? relativePath : `${relativePath}.md`;
    browserVaultNotes.set(clean, content);
    persistBrowserMapsToActiveVault(clean);
    return buildBrowserNoteDetail(clean);
  }
  return invoke<NoteDetail>('create_note', { relativePath, content });
}

export async function saveNote(relativePath: string, content: string): Promise<NoteDetail> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    browserVaultNotes.set(relativePath, content);
    persistBrowserMapsToActiveVault(relativePath);
    return buildBrowserNoteDetail(relativePath);
  }
  return invoke<NoteDetail>('save_note', { relativePath, content });
}

export async function renameNote(oldPath: string, newPath: string): Promise<NoteDetail> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const content = browserVaultNotes.get(oldPath) ?? '';
    browserVaultNotes.delete(oldPath);
    const clean = newPath.endsWith('.md') ? newPath : `${newPath}.md`;
    browserVaultNotes.set(clean, content);
    persistBrowserMapsToActiveVault(clean, oldPath);
    return buildBrowserNoteDetail(clean);
  }
  return invoke<NoteDetail>('rename_note', { oldPath, newPath });
}

export async function deleteNote(relativePath: string): Promise<VaultSnapshot> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    browserVaultNotes.delete(relativePath);
    persistBrowserMapsToActiveVault(undefined, relativePath);
    return buildBrowserSnapshot();
  }
  return invoke<VaultSnapshot>('delete_note', { relativePath });
}

export async function resolveWikilink(target: string): Promise<string | null> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    return resolveBrowserNotePath(target);
  }
  return invoke<string | null>('resolve_wikilink', { target });
}

export async function getBacklinks(relativePath: string): Promise<BacklinkHit[]> {
  if (!isTauriRuntime()) return buildBrowserNoteDetail(relativePath).backlinks;
  return invoke<BacklinkHit[]>('get_backlinks', { relativePath });
}

export async function searchNotes(query: string, limit = 30): Promise<SearchResult[]> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const q = query.trim().toLowerCase();
    if (!q) return [];
    const results: SearchResult[] = [];
    for (const [path, content] of browserVaultNotes.entries()) {
      if (path.toLowerCase().includes(q) || content.toLowerCase().includes(q)) {
        results.push({
          path,
          title: path.replace(/\.md$/i, ''),
          snippet: content.slice(0, 140),
          score: 1,
          tags: [],
        });
      }
    }
    return results.slice(0, limit);
  }
  return invoke<SearchResult[]>('search_notes', { query, limit });
}

export async function getAllTags(): Promise<TagCount[]> {
  if (!isTauriRuntime()) return buildBrowserSnapshot().tags;
  return invoke<TagCount[]>('get_all_tags');
}

export async function getNotesByTag(tag: string): Promise<NoteSummary[]> {
  if (!isTauriRuntime()) {
    const snap = buildBrowserSnapshot();
    const cleanTag = tag.replace(/^#+/, '').toLowerCase();
    return snap.notes.filter((n) => {
      const detail = buildBrowserNoteDetail(n.relative_path);
      return detail.tags.some((t) => t === cleanTag || t.startsWith(`${cleanTag}/`));
    });
  }
  return invoke<NoteSummary[]>('get_notes_by_tag', { tag });
}

export async function getSyncStatus(): Promise<SyncStatusDto> {
  if (!isTauriRuntime()) {
    const vault = getActiveWebVault(defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
    return getWebVaultSyncStatusDto(vault);
  }
  return invoke<SyncStatusDto>('get_sync_status');
}

export async function saveSyncConfig(
  config: SyncConfig,
  patToken?: string,
  clearToken?: boolean,
): Promise<SyncStatusDto> {
  if (!isTauriRuntime()) {
    const vault = getActiveWebVault(defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
    if (clearToken) {
      vault.patToken = '';
    }
    return saveWebVaultSyncConfig(vault, config, patToken);
  }
  return invoke<SyncStatusDto>('save_sync_config', {
    config,
    patToken: patToken ?? null,
    clearToken: clearToken ?? false,
  });
}

export async function syncVaultNow(timestampLabel?: string): Promise<SyncReport> {
  if (!isTauriRuntime()) {
    const vault = getActiveWebVault(defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
    const report = await syncWebVaultWithGithub(vault);
    syncBrowserMapsFromActiveVault();
    return report;
  }
  return invoke<SyncReport>('sync_vault_now', {
    timestampLabel: timestampLabel ?? null,
  });
}

export async function listGithubRepositoriesForWeb(patToken: string): Promise<GithubRepoInfo[]> {
  return listUserGithubRepos(patToken);
}

export async function logoutAndClearWebSession(): Promise<VaultSnapshot> {
  clearAllWebSessionData(defaultBrowserSeedNotes, defaultBrowserSeedCanvases);
  syncBrowserMapsFromActiveVault();
  return buildBrowserSnapshot();
}

// ============================================================================
// Helpers de respaldo en navegador (modo sin backend Tauri)
// ============================================================================

function currentLocalDateStr(): string {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function currentLocalTimeStr(): string {
  const d = new Date();
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

function substituteBrowserTemplate(
  raw: string,
  title: string,
  dateStr?: string | null,
  timeStr?: string | null,
): string {
  const d = dateStr?.trim() || currentLocalDateStr();
  const t = timeStr?.trim() || currentLocalTimeStr();
  return raw
    .replaceAll('{{date}}', d)
    .replaceAll('{{time}}', t)
    .replaceAll('{{title}}', title);
}

function extractBrowserFields(content: string): Map<string, string> {
  const fields = new Map<string, string>();
  const lines = content.split(/\r?\n/);
  let inFrontmatter = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (i === 0 && line === '---') {
      inFrontmatter = true;
      continue;
    }
    if (inFrontmatter) {
      if (line === '---') {
        inFrontmatter = false;
        continue;
      }
      const fmMatch = /^([a-zA-Z0-9_\-\u00C0-\u017F]+)\s*:\s*(.+)$/.exec(line);
      if (fmMatch) {
        fields.set(fmMatch[1].toLowerCase(), fmMatch[2].replace(/^["']|["']$/g, '').trim());
      }
      continue;
    }
    const bareMatch = /^(?:[-*]\s+)?([a-zA-Z0-9_\-\s\u00C0-\u017F]{1,40})::\s*(.+)$/.exec(line);
    if (bareMatch) {
      fields.set(bareMatch[1].trim().toLowerCase(), bareMatch[2].trim());
    }
    for (const m of line.matchAll(/[\[(]([a-zA-Z0-9_\-\s\u00C0-\u017F]{1,40})::\s*([^\])]+)[\])]/g)) {
      fields.set(m[1].trim().toLowerCase(), m[2].trim());
    }
  }
  return fields;
}

function resolveBrowserNotePath(target: string): string | null {
  const clean = target.trim().replace(/\.md$/i, '').toLowerCase();
  for (const key of browserVaultNotes.keys()) {
    const noExt = key.replace(/\.md$/i, '').toLowerCase();
    const stem = noExt.split('/').pop() || noExt;
    if (noExt === clean || stem === clean) return key;
  }
  return null;
}

function resolveBrowserTransclusion(
  rawTarget: string,
  subtarget?: string | null,
): TransclusionResolution | null {
  const resolvedPath = resolveBrowserNotePath(rawTarget);
  if (!resolvedPath) return null;
  const rawContent = browserVaultNotes.get(resolvedPath) ?? '';
  const body = rawContent.replace(/^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/, '');
  const title = resolvedPath.replace(/\.md$/i, '').split('/').pop() || resolvedPath;
  const sub = subtarget?.trim() || null;
  if (!sub) {
    return { resolved_path: resolvedPath, title, subtarget: null, content: body.trim() };
  }
  const lines = body.split(/\r?\n/);
  if (sub.startsWith('^')) {
    const blockId = sub.slice(1).toLowerCase();
    const hit = lines.find((l) => l.toLowerCase().includes(`^${blockId}`));
    if (!hit) return null;
    return {
      resolved_path: resolvedPath,
      title,
      subtarget: sub,
      content: hit.replace(/\s+\^[a-zA-Z0-9_-]+\s*$/, '').trim(),
    };
  }
  const wanted = sub.toLowerCase();
  let startIdx = -1;
  let headingLevel = 0;
  for (let i = 0; i < lines.length; i++) {
    const m = /^(#{1,6})\s+(.+)$/.exec(lines[i].trim());
    if (m && m[2].trim().toLowerCase() === wanted) {
      startIdx = i;
      headingLevel = m[1].length;
      break;
    }
  }
  if (startIdx < 0) return null;
  const collected: string[] = [lines[startIdx]];
  for (let i = startIdx + 1; i < lines.length; i++) {
    const m = /^(#{1,6})\s+/.exec(lines[i].trim());
    if (m && m[1].length <= headingLevel) break;
    collected.push(lines[i]);
  }
  return {
    resolved_path: resolvedPath,
    title,
    subtarget: sub,
    content: collected.join('\n').trim(),
  };
}

function evalBrowserInlineValue(notePath: string, expression: string): DataviewValue {
  const expr = expression.trim().replace(/^=\s*/, '');
  const content = browserVaultNotes.get(notePath) ?? '';
  const detail = buildBrowserNoteDetail(notePath);
  const fields = extractBrowserFields(content);
  const key = expr.replace(/^this\./i, '').toLowerCase();
  if (key === 'title' || key === 'file.name') {
    return { kind: 'text', value: detail.title };
  }
  if (key === 'file.path') {
    return { kind: 'text', value: notePath };
  }
  if (key === 'tags' || key === 'file.tags') {
    return { kind: 'list', items: detail.tags.map((t) => ({ kind: 'text', value: `#${t}` })) };
  }
  const found = fields.get(key);
  return found !== undefined ? { kind: 'text', value: found } : { kind: 'null' };
}

function evalBrowserDataview(query: string): DataviewQueryResult {
  const normalized = query.trim();
  const upper = normalized.toUpperCase();
  const entries = Array.from(browserVaultNotes.entries());

  if (upper.startsWith('TASK')) {
    const items = [];
    for (const [path, content] of entries) {
      const noteTitle = path.replace(/\.md$/i, '').split('/').pop() || path;
      const lines = content.split(/\r?\n/);
      for (let i = 0; i < lines.length; i++) {
        const m = /^\s*[-*]\s+\[([ xX])\]\s+(.+)$/.exec(lines[i]);
        if (m) {
          const text = m[2].trim();
          const tags = Array.from(text.matchAll(/#([a-zA-Z0-9_áéíóúñÁÉÍÓÚÑ/-]+)/g)).map(
            (t) => t[1].toLowerCase(),
          );
          items.push({
            path,
            note_title: noteTitle,
            line: i + 1,
            completed: m[1].toLowerCase() === 'x',
            text,
            tags,
          });
        }
      }
    }
    return { view: 'task', items, total_count: items.length };
  }

  if (upper.startsWith('LIST')) {
    const items = entries.map(([path]) => ({
      path,
      title: path.replace(/\.md$/i, '').split('/').pop() || path,
      value: null,
    }));
    return { view: 'list', items, total_count: items.length };
  }

  const firstLine = normalized.split(/\r?\n/)[0] ?? 'TABLE';
  const rawCols = firstLine
    .replace(/^TABLE(?:\s+WITHOUT\s+ID)?\s*/i, '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  const colDefs = rawCols.map((c) => {
    const asMatch = /^(.+?)\s+AS\s+"?([^"]+)"?$/i.exec(c);
    return {
      expr: (asMatch ? asMatch[1] : c).trim().toLowerCase(),
      header: (asMatch ? asMatch[2] : c).trim(),
    };
  });
  const headers = ['Archivo', ...colDefs.map((c) => c.header)];
  const rows: DataviewValue[][] = entries.map(([path]) => {
    const title = path.replace(/\.md$/i, '').split('/').pop() || path;
    const cells: DataviewValue[] = [{ kind: 'link', path, label: title }];
    for (const col of colDefs) {
      cells.push(evalBrowserInlineValue(path, col.expr));
    }
    return cells;
  });
  return { view: 'table', headers, rows, total_count: rows.length };
}

// ============================================================================
// Comandos IPC v0.2 (Dataview, Transclusiones, Menciones, Canvas, Diarias)
// ============================================================================

export async function runDataviewQuery(query: string): Promise<DataviewQueryResult> {
  if (!isTauriRuntime()) return evalBrowserDataview(query);
  return invoke<DataviewQueryResult>('run_dataview_query', { query });
}

export async function evalDataviewInline(
  notePath: string,
  expression: string,
): Promise<DataviewValue> {
  if (!isTauriRuntime()) return evalBrowserInlineValue(notePath, expression);
  return invoke<DataviewValue>('eval_dataview_inline', { notePath, expression });
}

export async function evalDataviewInlineBatch(
  notePath: string,
  expressions: string[],
): Promise<DataviewInlineEvaluation[]> {
  if (!isTauriRuntime()) {
    return expressions.map((expression) => ({
      expression,
      value: evalBrowserInlineValue(notePath, expression),
    }));
  }
  return invoke<DataviewInlineEvaluation[]>('eval_dataview_inline_batch', {
    notePath,
    expressions,
  });
}

export async function getNoteFields(relativePath: string): Promise<IndexedField[]> {
  if (!isTauriRuntime()) {
    const map = extractBrowserFields(browserVaultNotes.get(relativePath) ?? '');
    return Array.from(map.entries()).map(([key, value]) => ({
      path: relativePath,
      key,
      key_norm: key,
      value,
      source: 'inline',
      line: 1,
    }));
  }
  return invoke<IndexedField[]>('get_note_fields', { relativePath });
}

export async function getNoteHeadings(relativePath: string): Promise<HeadingItem[]> {
  if (!isTauriRuntime()) return [];
  return invoke<HeadingItem[]>('get_note_headings', { relativePath });
}

export async function getNoteBlocks(relativePath: string): Promise<BlockRef[]> {
  if (!isTauriRuntime()) return [];
  return invoke<BlockRef[]>('get_note_blocks', { relativePath });
}

export async function resolveTransclusion(
  rawTarget: string,
  subtarget?: string | null,
): Promise<TransclusionResolution | null> {
  if (!isTauriRuntime()) return resolveBrowserTransclusion(rawTarget, subtarget);
  return invoke<TransclusionResolution | null>('resolve_transclusion', {
    rawTarget,
    subtarget: subtarget ?? null,
  });
}

function matchesBrowserExclude(path: string, patterns?: string[] | null): boolean {
  if (!patterns || patterns.length === 0) return false;
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

function browserProtectedRanges(line: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  const patterns = [
    /!?\[\[[^\]]+\]\]/g,
    /`[^`]+`/g,
    /\[[^\]]+\]\([^)]+\)/g,
    /(^|\s)#[\p{L}\p{N}_/-]+/gu,
  ];
  for (const re of patterns) {
    for (const match of line.matchAll(re)) {
      const start = match.index ?? 0;
      ranges.push([start, start + match[0].length]);
    }
  }
  return ranges;
}

function browserUnlinkedMentions(
  relativePath: string,
  excludedPatterns?: string[] | null,
): UnlinkedMentionHit[] {
  const targetDetail = buildBrowserNoteDetail(relativePath);
  const baseName = relativePath.replace(/\.md$/i, '').split('/').pop() || relativePath;
  const aliasesRaw = targetDetail.frontmatter.aliases ?? targetDetail.frontmatter.alias;
  const aliases = Array.isArray(aliasesRaw)
    ? aliasesRaw
    : typeof aliasesRaw === 'string'
      ? [aliasesRaw]
      : [];
  const terms = Array.from(
    new Set([targetDetail.title, baseName, ...aliases].map((t) => t.trim()).filter((t) => t.length >= 2)),
  ).sort((a, b) => b.length - a.length);

  if (terms.length === 0) return [];
  const hits: UnlinkedMentionHit[] = [];

  for (const [sourcePath, rawContent] of browserVaultNotes.entries()) {
    if (sourcePath === relativePath || matchesBrowserExclude(sourcePath, excludedPatterns)) {
      continue;
    }
    const sourceTitle = sourcePath.replace(/\.md$/i, '').split('/').pop() || sourcePath;
    const lines = rawContent.split(/\r?\n/);
    let inFence = false;
    let inFrontmatter = lines[0]?.trim() === '---';
    for (let i = 0; i < lines.length; i++) {
      const lineText = lines[i];
      if (inFrontmatter) {
        if (i > 0 && lineText.trim() === '---') inFrontmatter = false;
        continue;
      }
      const trimmed = lineText.trim();
      if (trimmed.startsWith('```') || trimmed.startsWith('~~~')) {
        inFence = !inFence;
        continue;
      }
      if (inFence || !trimmed) continue;

      const protectedRanges = browserProtectedRanges(lineText);
      const lowerLine = lineText.toLowerCase();
      for (const term of terms) {
        const lowerTerm = term.toLowerCase();
        let searchFrom = 0;
        let foundOnLine = false;
        while (searchFrom < lowerLine.length) {
          const idx = lowerLine.indexOf(lowerTerm, searchFrom);
          if (idx < 0) break;
          const end = idx + term.length;
          searchFrom = end;
          const prevChar = idx > 0 ? lineText[idx - 1] : '';
          const nextChar = end < lineText.length ? lineText[end] : '';
          if (/\p{L}|\p{N}/u.test(prevChar) || /\p{L}|\p{N}/u.test(nextChar)) continue;
          if (protectedRanges.some(([ps, pe]) => idx < pe && end > ps)) continue;
          hits.push({
            source_path: sourcePath,
            source_title: sourceTitle,
            matched_term: lineText.slice(idx, end),
            line: i + 1,
            column_start: idx,
            column_end: end,
            context: lineText.trim(),
          });
          foundOnLine = true;
          break;
        }
        if (foundOnLine) break;
      }
    }
  }
  return hits;
}

export async function getUnlinkedMentions(
  relativePath: string,
  excludedPatterns?: string[] | null,
): Promise<UnlinkedMentionHit[]> {
  if (!isTauriRuntime()) return browserUnlinkedMentions(relativePath, excludedPatterns);
  return invoke<UnlinkedMentionHit[]>('get_unlinked_mentions', {
    relativePath,
    excludedPatterns: excludedPatterns ?? null,
  });
}

export async function linkUnlinkedMention(
  sourcePath: string,
  line: number,
  matchedTerm: string,
  targetNotePath: string,
): Promise<NoteDetail> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const raw = browserVaultNotes.get(sourcePath) ?? '';
    const lines = raw.split(/\r?\n/);
    const idx = line - 1;
    if (idx >= 0 && idx < lines.length) {
      const targetStem = targetNotePath.replace(/\.md$/i, '');
      const targetTitle = targetStem.split('/').pop() || targetStem;
      const replacement =
        matchedTerm.toLowerCase() === targetTitle.toLowerCase()
          ? `[[${targetTitle}]]`
          : `[[${targetTitle}|${matchedTerm}]]`;
      const pos = lines[idx].toLowerCase().indexOf(matchedTerm.toLowerCase());
      if (pos >= 0) {
        lines[idx] =
          lines[idx].slice(0, pos) +
          replacement +
          lines[idx].slice(pos + matchedTerm.length);
        browserVaultNotes.set(sourcePath, lines.join('\n'));
        persistBrowserMapsToActiveVault(sourcePath);
      }
    }
    return buildBrowserNoteDetail(sourcePath);
  }
  return invoke<NoteDetail>('link_unlinked_mention', {
    sourcePath,
    line,
    matchedTerm,
    targetNotePath,
  });
}

export async function listCanvasFiles(): Promise<CanvasSummary[]> {
  if (!isTauriRuntime()) return buildBrowserSnapshot().canvases;
  return invoke<CanvasSummary[]>('list_canvas_files');
}

export async function createCanvasFile(
  relativePath: string,
  initialContent?: string | null,
): Promise<CanvasDocument> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const clean = relativePath.endsWith('.canvas') ? relativePath : `${relativePath}.canvas`;
    const content = initialContent ?? JSON.stringify({ nodes: [], edges: [] }, null, 2);
    browserVaultCanvases.set(clean, content);
    persistBrowserMapsToActiveVault(clean);
    return {
      relative_path: clean,
      title: clean.replace(/\.canvas$/i, ''),
      content,
      modified_unix_secs: Math.floor(Date.now() / 1000),
    };
  }
  return invoke<CanvasDocument>('create_canvas_file', {
    relativePath,
    initialContent: initialContent ?? null,
  });
}

export async function readCanvasFile(relativePath: string): Promise<CanvasDocument> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    return {
      relative_path: relativePath,
      title: relativePath.replace(/\.canvas$/i, ''),
      content: browserVaultCanvases.get(relativePath) ?? JSON.stringify({ nodes: [], edges: [] }),
      modified_unix_secs: Math.floor(Date.now() / 1000),
    };
  }
  return invoke<CanvasDocument>('read_canvas_file', { relativePath });
}

export async function saveCanvasFile(
  relativePath: string,
  content: string,
): Promise<CanvasDocument> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    browserVaultCanvases.set(relativePath, content);
    persistBrowserMapsToActiveVault(relativePath);
    return readCanvasFile(relativePath);
  }
  return invoke<CanvasDocument>('save_canvas_file', { relativePath, content });
}

export async function renameCanvasFile(
  oldPath: string,
  newPath: string,
): Promise<CanvasSummary> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const content = browserVaultCanvases.get(oldPath) ?? '{"nodes":[],"edges":[]}';
    browserVaultCanvases.delete(oldPath);
    browserVaultCanvases.set(newPath, content);
    persistBrowserMapsToActiveVault(newPath, oldPath);
    return {
      relative_path: newPath,
      title: newPath.replace(/\.canvas$/i, ''),
      modified_unix_secs: Math.floor(Date.now() / 1000),
      size_bytes: content.length,
    };
  }
  return invoke<CanvasSummary>('rename_canvas_file', { oldPath, newPath });
}

export async function deleteCanvasFile(relativePath: string): Promise<VaultSnapshot> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    browserVaultCanvases.delete(relativePath);
    persistBrowserMapsToActiveVault(undefined, relativePath);
    return buildBrowserSnapshot();
  }
  return invoke<VaultSnapshot>('delete_canvas_file', { relativePath });
}

export async function getOrCreateDailyNote(options?: {
  dateStr?: string | null;
  timeStr?: string | null;
  dailyFolder?: string | null;
  templatePath?: string | null;
}): Promise<DailyNoteResult> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const dateStr = options?.dateStr?.trim() || currentLocalDateStr();
    const timeStr = options?.timeStr?.trim() || currentLocalTimeStr();
    const folder = (options?.dailyFolder?.trim() || 'daily').replace(/^\/+|\/+$/g, '');
    const relPath = folder ? `${folder}/${dateStr}.md` : `${dateStr}.md`;
    const existed = browserVaultNotes.has(relPath);
    if (!existed) {
      const tplContent =
        options?.templatePath && browserVaultNotes.get(options.templatePath)
          ? browserVaultNotes.get(options.templatePath)!
          : `# ${dateStr}\n\nCreada el {{date}} a las {{time}}.\n\n## Tareas\n- [ ] \n\n## Notas del día\n- \n`;
      browserVaultNotes.set(relPath, substituteBrowserTemplate(tplContent, dateStr, dateStr, timeStr));
      persistBrowserMapsToActiveVault(relPath);
    }
    return {
      note: buildBrowserNoteDetail(relPath),
      created: !existed,
    };
  }
  return invoke<DailyNoteResult>('get_or_create_daily_note', {
    dateStr: options?.dateStr ?? null,
    timeStr: options?.timeStr ?? null,
    dailyFolder: options?.dailyFolder ?? null,
    templatePath: options?.templatePath ?? null,
  });
}

export async function listTemplates(templatesFolder?: string | null): Promise<TemplateSummary[]> {
  if (!isTauriRuntime()) {
    const folderPrefix = `${(templatesFolder?.trim() || 'templates').replace(/^\/+|\/+$/g, '')}/`;
    const list: TemplateSummary[] = [];
    for (const key of browserVaultNotes.keys()) {
      if (key.startsWith(folderPrefix)) {
        list.push({
          relative_path: key,
          name: key.slice(folderPrefix.length).replace(/\.md$/i, ''),
        });
      }
    }
    return list.sort((a, b) => a.name.localeCompare(b.name));
  }
  return invoke<TemplateSummary[]>('list_templates', {
    templatesFolder: templatesFolder ?? null,
  });
}

export async function applyTemplateContent(
  templatePath: string,
  targetTitle: string,
  dateStr?: string | null,
  timeStr?: string | null,
): Promise<string> {
  if (!isTauriRuntime()) {
    const raw = browserVaultNotes.get(templatePath) ?? `# {{title}}\n\n{{date}} {{time}}\n`;
    return substituteBrowserTemplate(raw, targetTitle, dateStr, timeStr);
  }
  return invoke<string>('apply_template_content', {
    templatePath,
    targetTitle,
    dateStr: dateStr ?? null,
    timeStr: timeStr ?? null,
  });
}

function extractBrowserFrontmatterTags(content: string): string[] {
  const fmMatch = content.match(/^---\r?\n([\s\S]*?)\r?\n---/);
  if (!fmMatch) return [];
  const tags: string[] = [];
  const lines = fmMatch[1].split(/\r?\n/);
  let inTagsList = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (/^tags\s*:/i.test(trimmed)) {
      const rest = trimmed.slice(trimmed.indexOf(':') + 1).trim();
      if (rest.startsWith('[') && rest.endsWith(']')) {
        for (const item of rest.slice(1, -1).split(',')) {
          const clean = item.trim().replace(/^["'#]+|["']+$/g, '').toLowerCase();
          if (clean) tags.push(clean);
        }
        inTagsList = false;
      } else if (rest) {
        for (const item of rest.split(',')) {
          const clean = item.trim().replace(/^["'#]+|["']+$/g, '').toLowerCase();
          if (clean) tags.push(clean);
        }
        inTagsList = false;
      } else {
        inTagsList = true;
      }
      continue;
    }
    if (inTagsList) {
      if (trimmed.startsWith('-')) {
        const clean = trimmed.slice(1).trim().replace(/^["'#]+|["']+$/g, '').toLowerCase();
        if (clean) tags.push(clean);
      } else if (trimmed && !line.startsWith(' ') && !line.startsWith('\t')) {
        inTagsList = false;
      }
    }
  }
  return tags;
}

function buildBrowserGraphData(): GraphData {
  const stemToPath = new Map<string, string>();
  const baseToPath = new Map<string, string>();
  const noteTags = new Map<string, string[]>();

  for (const [relPath, content] of browserVaultNotes.entries()) {
    const stem = relPath.replace(/\.md$/i, '').toLowerCase();
    const base = (stem.split('/').pop() || stem).toLowerCase();
    stemToPath.set(stem, relPath);
    if (!baseToPath.has(base)) baseToPath.set(base, relPath);

    const inlineTags = Array.from(
      content.matchAll(/(?:^|\s)#([a-zA-Z0-9_áéíóúñÁÉÍÓÚÑ/-]+)/g),
    ).map((m) => m[1].toLowerCase());
    const fmTags = extractBrowserFrontmatterTags(content);
    noteTags.set(relPath, Array.from(new Set([...fmTags, ...inlineTags])));
  }

  const degrees = new Map<string, number>();
  const edgeKeys = new Set<string>();
  const edges: GraphData['edges'] = [];
  const missingNodes = new Map<string, string>();

  for (const [sourcePath, content] of browserVaultNotes.entries()) {
    for (const m of content.matchAll(/!?\[\[([^\]|#]+)(?:#[^\]|]+)?(?:\|[^\]]+)?\]\]/g)) {
      const cleanRaw = m[1].trim();
      if (!cleanRaw) continue;
      const lookup = cleanRaw.replace(/\.md$/i, '').toLowerCase();
      const targetResolved = stemToPath.get(lookup) ?? baseToPath.get(lookup) ?? null;

      if (targetResolved && targetResolved !== sourcePath) {
        const key = `${sourcePath}::${targetResolved}::1`;
        if (!edgeKeys.has(key)) {
          edgeKeys.add(key);
          edges.push({ source: sourcePath, target: targetResolved, resolved: true });
          degrees.set(sourcePath, (degrees.get(sourcePath) ?? 0) + 1);
          degrees.set(targetResolved, (degrees.get(targetResolved) ?? 0) + 1);
        }
      } else if (!targetResolved) {
        const ghostId = `__unresolved__:${cleanRaw}`;
        if (!missingNodes.has(ghostId)) missingNodes.set(ghostId, cleanRaw);
        const key = `${sourcePath}::${ghostId}::0`;
        if (!edgeKeys.has(key)) {
          edgeKeys.add(key);
          edges.push({ source: sourcePath, target: ghostId, resolved: false });
          degrees.set(sourcePath, (degrees.get(sourcePath) ?? 0) + 1);
          degrees.set(ghostId, (degrees.get(ghostId) ?? 0) + 1);
        }
      }
    }
  }

  const nodes: GraphData['nodes'] = Array.from(browserVaultNotes.keys()).map((relPath) => {
    const slashIdx = relPath.lastIndexOf('/');
    const folder = slashIdx >= 0 ? relPath.slice(0, slashIdx) : '';
    const title = relPath.replace(/\.md$/i, '').split('/').pop() || relPath;
    return {
      id: relPath,
      label: title,
      tags: noteTags.get(relPath) ?? [],
      degree: degrees.get(relPath) ?? 0,
      folder,
      exists: true,
    };
  });

  for (const [ghostId, label] of missingNodes.entries()) {
    nodes.push({
      id: ghostId,
      label,
      tags: [],
      degree: degrees.get(ghostId) ?? 0,
      folder: '',
      exists: false,
    });
  }

  return { nodes, edges };
}

export async function getGraphData(): Promise<GraphData> {
  if (!isTauriRuntime()) return buildBrowserGraphData();
  return invoke<GraphData>('get_graph_data');
}

export async function pickVaultFolder(): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  return invoke<string | null>('pick_vault_folder');
}

export async function createFolder(folderPath: string): Promise<void> {
  if (!isTauriRuntime()) return;
  return invoke<void>('create_folder', { folderPath });
}

export async function deleteFolder(folderPath: string): Promise<void> {
  if (!isTauriRuntime()) return;
  return invoke<void>('delete_folder', { folderPath });
}

export async function moveVaultItem(
  sourceRelativePath: string,
  targetFolderRelativePath: string,
): Promise<string> {
  if (!isTauriRuntime()) return sourceRelativePath;
  return invoke<string>('move_vault_item', {
    sourceRelativePath,
    targetFolderRelativePath,
  });
}

let browserAiConfig: AiProviderConfig = {
  provider: 'deterministic',
  model: 'deterministic-rust-v1',
  endpoint_url: null,
  cli_binary_override: null,
  max_connections: 8,
  append_related_section: true,
  fallback_to_deterministic: true,
};
let browserHasGeminiKey = false;
let browserHasCompatibleKey = false;

function buildBrowserAiStatus(): AiProviderStatus {
  return {
    active_config: { ...browserAiConfig },
    claude_cli_available: false,
    claude_cli_path: null,
    gemini_cli_available: false,
    gemini_cli_path: null,
    has_gemini_api_key: browserHasGeminiKey,
    has_compatible_api_key: browserHasCompatibleKey,
    platform: 'browser-preview',
  };
}

function buildBrowserKnowledgeMap(): VaultKnowledgeMap {
  const folderSet = new Set<string>();
  const tagCounts = new Map<string, number>();
  const fmStats = new Map<string, { count: number; samples: string[] }>();
  const notes: KnowledgeNoteEntry[] = [];
  let totalLinks = 0;

  for (const [relPath, raw] of browserVaultNotes.entries()) {
    const detail = buildBrowserNoteDetail(relPath);
    const stem = relPath.replace(/\.md$/i, '').split('/').pop() || relPath;
    const slashIdx = relPath.lastIndexOf('/');
    const folder = slashIdx >= 0 ? relPath.slice(0, slashIdx) : '';
    if (folder) folderSet.add(folder);

    const fields = extractBrowserFields(raw);
    const fmKeys = Array.from(fields.keys());
    for (const [k, v] of fields.entries()) {
      const entry = fmStats.get(k) ?? { count: 0, samples: [] };
      entry.count += 1;
      if (v && !entry.samples.includes(v) && entry.samples.length < 3) {
        entry.samples.push(v);
      }
      fmStats.set(k, entry);
    }

    for (const tag of detail.tags) {
      tagCounts.set(tag, (tagCounts.get(tag) ?? 0) + 1);
    }

    const headings = raw
      .split(/\r?\n/)
      .map((l) => /^#{1,6}\s+(.+)$/.exec(l.trim())?.[1]?.trim() ?? null)
      .filter((h): h is string => Boolean(h));

    const outCount = detail.wikilinks.length;
    const inCount = detail.backlinks.length;
    totalLinks += outCount;

    notes.push({
      path: relPath,
      stem,
      title: stem,
      folder,
      aliases: [],
      tags: detail.tags,
      headings,
      frontmatter_keys: fmKeys,
      outlink_count: outCount,
      inlink_count: inCount,
      is_orphan: outCount === 0 && inCount === 0,
    });
  }

  const tag_taxonomy: TagCount[] = Array.from(tagCounts.entries())
    .map(([tag, count]) => ({ tag, count }))
    .sort((a, b) => b.count - a.count || a.tag.localeCompare(b.tag));

  const frontmatter_schema: FrontmatterKeyStat[] = Array.from(fmStats.entries())
    .map(([key, data]) => ({ key, count: data.count, sample_values: data.samples }))
    .sort((a, b) => b.count - a.count || a.key.localeCompare(b.key));

  return {
    total_notes: notes.length,
    total_links: totalLinks,
    total_tags: tag_taxonomy.length,
    orphan_paths: notes.filter((n) => n.is_orphan).map((n) => n.path),
    folders: Array.from(folderSet).sort(),
    tag_taxonomy,
    frontmatter_schema,
    notes,
  };
}

function browserFindInlineReplacements(
  sourcePath: string,
  targetPath: string,
  targetStem: string,
): WikilinkReplacement[] {
  const raw = browserVaultNotes.get(sourcePath) ?? '';
  const lines = raw.split(/\r?\n/);
  const lowerTerm = targetStem.toLowerCase();
  if (lowerTerm.length < 2) return [];
  let inFence = false;
  let inFm = lines[0]?.trim() === '---';
  const hits: WikilinkReplacement[] = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (inFm) {
      if (i > 0 && line.trim() === '---') inFm = false;
      continue;
    }
    const trimmed = line.trim();
    if (trimmed.startsWith('```') || trimmed.startsWith('~~~')) {
      inFence = !inFence;
      continue;
    }
    if (inFence || !trimmed || trimmed.startsWith('#')) continue;
    const ranges = browserProtectedRanges(line);
    const idx = line.toLowerCase().indexOf(lowerTerm);
    if (idx < 0) continue;
    const end = idx + targetStem.length;
    if (ranges.some(([ps, pe]) => idx < pe && end > ps)) continue;
    hits.push({
      target_path: targetPath,
      target_stem: targetStem,
      matched_term: line.slice(idx, end),
      replacement_wikilink: `[[${targetStem}]]`,
      line: i + 1,
      context: trimmed,
    });
    break;
  }
  return hits;
}

function buildBrowserCandidateConnections(
  relativePath: string,
  maxCandidates = 10,
): CandidateConnection[] {
  const sourceRaw = browserVaultNotes.get(relativePath);
  if (!sourceRaw) return [];
  const sourceDetail = buildBrowserNoteDetail(relativePath);
  const sourceLowerWords = new Set(
    sourceRaw
      .toLowerCase()
      .split(/[^\p{L}\p{N}]+/u)
      .filter((w) => w.length >= 4),
  );
  const linkedPaths = new Set(
    sourceDetail.wikilinks
      .map((w) => resolveBrowserNotePath(w.target))
      .filter((p): p is string => Boolean(p)),
  );

  const candidates: CandidateConnection[] = [];
  for (const [targetPath, targetRaw] of browserVaultNotes.entries()) {
    if (targetPath === relativePath) continue;
    const targetDetail = buildBrowserNoteDetail(targetPath);
    const targetStem = targetPath.replace(/\.md$/i, '').split('/').pop() || targetPath;
    const signals: ConnectionSignal[] = [];
    let score = 0;
    const reasons: string[] = [];

    const inlineHits = browserFindInlineReplacements(relativePath, targetPath, targetStem);
    if (inlineHits.length > 0) {
      score += 45;
      reasons.push(`menciona "${inlineHits[0].matched_term}" sin enlazar`);
      signals.push({
        kind: 'outgoing_unlinked_mention',
        matched_term: inlineHits[0].matched_term,
        line: inlineHits[0].line,
        context: inlineHits[0].context,
      });
    }

    const sharedTags = sourceDetail.tags.filter((t) => targetDetail.tags.includes(t));
    if (sharedTags.length > 0) {
      score += 15 * sharedTags.length;
      reasons.push(`comparte #${sharedTags[0]}`);
      signals.push({ kind: 'shared_tags', tags: sharedTags });
    }

    const overlapWords: string[] = [];
    for (const w of targetRaw.toLowerCase().split(/[^\p{L}\p{N}]+/u)) {
      if (w.length >= 4 && sourceLowerWords.has(w) && !overlapWords.includes(w)) {
        overlapWords.push(w);
        if (overlapWords.length >= 4) break;
      }
    }
    if (overlapWords.length > 0) {
      score += Math.min(20, overlapWords.length * 5);
      reasons.push(`vocabulario común (${overlapWords.slice(0, 3).join(', ')})`);
      signals.push({ kind: 'lexical_bm25', score: overlapWords.length * 5, matched_keywords: overlapWords });
    }

    if (score > 0) {
      candidates.push({
        target_path: targetPath,
        target_title: targetStem,
        target_stem: targetStem,
        target_tags: targetDetail.tags,
        score,
        already_linked: linkedPaths.has(targetPath),
        suggested_wikilink: `[[${targetStem}]]`,
        inline_replacements: inlineHits,
        signals,
        reason_summary: reasons.join('; '),
      });
    }
  }

  return candidates
    .sort((a, b) => Number(a.already_linked) - Number(b.already_linked) || b.score - a.score)
    .slice(0, maxCandidates);
}

function previewBrowserEnrichment(relativePath: string, plan: AiEnrichmentPlan): AiEnrichmentPreview {
  const original = browserVaultNotes.get(relativePath) ?? '';
  let lines = original.split(/\r?\n/);
  const injected: WikilinkReplacement[] = [];
  const addedFmKeys: string[] = [];
  const addedTags: string[] = [];
  const appendedRelated: string[] = [];

  for (const rep of plan.inline_wikilinks) {
    const idx = rep.line - 1;
    if (idx < 0 || idx >= lines.length) continue;
    const pos = lines[idx].toLowerCase().indexOf(rep.matched_term.toLowerCase());
    if (pos >= 0 && !lines[idx].includes(rep.replacement_wikilink)) {
      const before = lines[idx].slice(0, pos);
      const after = lines[idx].slice(pos + rep.matched_term.length);
      lines[idx] = `${before}${rep.replacement_wikilink}${after}`;
      injected.push(rep);
    }
  }

  let body = lines.join('\n');
  if (!body.startsWith('---') && Object.keys(plan.frontmatter_updates).length > 0) {
    const fmEntries = Object.entries(plan.frontmatter_updates).map(([k, v]) => {
      addedFmKeys.push(k);
      return Array.isArray(v)
        ? `${k}:\n${v.map((item) => `  - ${item}`).join('\n')}`
        : `${k}: ${v}`;
    });
    body = `---\n${fmEntries.join('\n')}\n---\n\n${body}`;
  }

  const missingTags = plan.suggested_tags
    .map((t) => t.replace(/^#/, '').trim().toLowerCase())
    .filter((t) => t && !body.toLowerCase().includes(`#${t}`));
  if (missingTags.length > 0) {
    addedTags.push(...missingTags);
    body = `${body.trimEnd()}\n\n${missingTags.map((t) => `#${t}`).join(' ')}\n`;
  }

  if (plan.append_related_section && plan.related_connections.length > 0) {
    const unlinked = plan.related_connections.filter(
      (rc) => !body.toLowerCase().includes(`[[${rc.target_title.toLowerCase()}`),
    );
    if (unlinked.length > 0) {
      const sectionLines = unlinked.map((rc) => {
        const wl = `[[${rc.target_title}]]`;
        appendedRelated.push(wl);
        return `- ${wl}${rc.relation ? ` — ${rc.relation}` : ''}`;
      });
      body = `${body.trimEnd()}\n\n## Conexiones Relacionadas\n${sectionLines.join('\n')}\n`;
    }
  }

  const changed = body !== original;
  const origLines = original.split(/\r?\n/);
  const nextLines = body.split(/\r?\n/);
  const unified_diff_lines: string[] = [];
  if (changed) {
    unified_diff_lines.push(`--- ${relativePath}`, `+++ ${relativePath}`);
    const maxLen = Math.max(origLines.length, nextLines.length);
    for (let i = 0; i < maxLen; i++) {
      if (origLines[i] !== nextLines[i]) {
        if (origLines[i] !== undefined) unified_diff_lines.push(`-${origLines[i]}`);
        if (nextLines[i] !== undefined) unified_diff_lines.push(`+${nextLines[i]}`);
      }
    }
  }

  return {
    relative_path: relativePath,
    original_content: original,
    enriched_content: body,
    changed,
    added_frontmatter_keys: addedFmKeys,
    added_tags: addedTags,
    injected_wikilinks: injected,
    appended_related_links: appendedRelated,
    unified_diff_lines,
  };
}

export async function getAiProviderStatus(): Promise<AiProviderStatus> {
  if (!isTauriRuntime()) return buildBrowserAiStatus();
  return invoke<AiProviderStatus>('get_ai_provider_status');
}

export async function saveAiProviderConfig(
  config: AiProviderConfig,
  options?: {
    geminiApiKey?: string;
    compatibleApiKey?: string;
    clearGeminiApiKey?: boolean;
    clearCompatibleApiKey?: boolean;
  },
): Promise<AiProviderStatus> {
  if (!isTauriRuntime()) {
    browserAiConfig = { ...config };
    if (options?.clearGeminiApiKey) browserHasGeminiKey = false;
    else if (options?.geminiApiKey?.trim()) browserHasGeminiKey = true;
    if (options?.clearCompatibleApiKey) browserHasCompatibleKey = false;
    else if (options?.compatibleApiKey?.trim()) browserHasCompatibleKey = true;
    return buildBrowserAiStatus();
  }
  return invoke<AiProviderStatus>('save_ai_provider_config', {
    config,
    geminiApiKey: options?.geminiApiKey ?? null,
    compatibleApiKey: options?.compatibleApiKey ?? null,
    clearGeminiApiKey: options?.clearGeminiApiKey ?? false,
    clearCompatibleApiKey: options?.clearCompatibleApiKey ?? false,
  });
}

export async function getVaultKnowledgeMap(): Promise<VaultKnowledgeMap> {
  if (!isTauriRuntime()) return buildBrowserKnowledgeMap();
  return invoke<VaultKnowledgeMap>('get_vault_knowledge_map');
}

export async function suggestNoteConnections(
  relativePath: string,
  maxCandidates?: number,
): Promise<CandidateConnection[]> {
  if (!isTauriRuntime()) return buildBrowserCandidateConnections(relativePath, maxCandidates ?? 10);
  return invoke<CandidateConnection[]>('suggest_note_connections_cmd', {
    relativePath,
    maxCandidates: maxCandidates ?? null,
  });
}

export async function autoConnectNotes(
  request: AutoConnectRequest,
): Promise<AutoConnectBatchResponse> {
  if (!isTauriRuntime()) {
    const mode = request.mode ?? 'preview';
    const maxConn = request.max_connections ?? browserAiConfig.max_connections;
    const appendRelated =
      request.append_related_section ?? browserAiConfig.append_related_section;
    let targetPaths: string[] = [];
    if (request.path) {
      targetPaths = [request.path];
    } else if (request.paths && request.paths.length > 0) {
      targetPaths = request.paths;
    } else {
      const folderClean = (request.folder ?? '').trim().replace(/^[/\\]+|[/\\]+$/g, '').toLowerCase();
      targetPaths = Array.from(browserVaultNotes.keys()).filter((p) => {
        if (!folderClean) return !p.toLowerCase().startsWith('templates/');
        const lower = p.toLowerCase();
        return lower === folderClean || lower.startsWith(`${folderClean}/`);
      });
    }

    const items: AutoConnectNoteOutcome[] = targetPaths
      .filter((p) => browserVaultNotes.has(p))
      .map((path) => {
        const plan: AiEnrichmentPlan = request.custom_plan
          ? request.custom_plan
          : (() => {
              const candidates = buildBrowserCandidateConnections(path, maxConn);
              const stem = path.replace(/\.md$/i, '').split('/').pop() || path;
              const inline_wikilinks = candidates.flatMap((c) => c.inline_replacements);
              const related_connections = candidates
                .filter((c) => !c.already_linked)
                .slice(0, maxConn)
                .map((c) => ({
                  target_path: c.target_path,
                  target_title: c.target_stem,
                  relation: c.reason_summary || null,
                }));
              const suggested_tags = Array.from(
                new Set(candidates.flatMap((c) => c.target_tags)),
              ).slice(0, 4);
              return {
                frontmatter_updates: { title: stem },
                overwrite_existing_frontmatter: false,
                suggested_tags,
                inline_wikilinks,
                related_connections,
                append_related_section: appendRelated,
              };
            })();

        const preview = previewBrowserEnrichment(path, plan);
        let applied = false;
        if (mode === 'apply' && preview.changed) {
          browserVaultNotes.set(path, preview.enriched_content);
          persistBrowserMapsToActiveVault(path);
          applied = true;
        }
        return {
          path,
          plan,
          preview,
          applied,
          provider_used: browserAiConfig.provider,
          warning: null,
        };
      });

    return {
      mode,
      total_processed: items.length,
      changed_count: items.filter((i) => i.preview.changed).length,
      applied_count: items.filter((i) => i.applied).length,
      provider_used: browserAiConfig.provider,
      items,
      snapshot: mode === 'apply' ? buildBrowserSnapshot() : null,
    };
  }
  return invoke<AutoConnectBatchResponse>('auto_connect_notes_cmd', { request });
}

export async function createKnowledgeCanvas(
  request: KnowledgeCanvasRequest,
): Promise<KnowledgeCanvasResult> {
  if (!isTauriRuntime()) {
    syncBrowserMapsFromActiveVault();
    const seeds =
      request.seed_note_paths && request.seed_note_paths.length > 0
        ? request.seed_note_paths
        : Array.from(browserVaultNotes.keys()).slice(0, request.max_nodes ?? 8);
    const nodes = seeds.map((file, idx) => ({
      id: `node-${idx}`,
      type: 'file' as const,
      file,
      x: (idx % 3) * 320,
      y: Math.floor(idx / 3) * 200,
      width: 260,
      height: 140,
      color: String((idx % 6) + 1),
      mediaKind: 'note' as const,
    }));
    const content = JSON.stringify({ nodes, edges: [] }, null, 2);
    const cleanPath = request.canvas_path.endsWith('.canvas')
      ? request.canvas_path
      : `${request.canvas_path}.canvas`;
    browserVaultCanvases.set(cleanPath, content);
    persistBrowserMapsToActiveVault(cleanPath);
    return {
      canvas: {
        relative_path: cleanPath,
        title: request.title?.trim() || cleanPath.replace(/\.canvas$/i, ''),
        content,
        modified_unix_secs: Math.floor(Date.now() / 1000),
      },
      node_count: nodes.length,
      edge_count: 0,
    };
  }
  return invoke<KnowledgeCanvasResult>('create_knowledge_canvas_cmd', { request });
}

export async function askAiCopilot(request: AiChatRequest): Promise<AiChatResponse> {
  if (!isTauriRuntime()) {
    const map = buildBrowserKnowledgeMap();
    const active = request.active_note_path
      ? buildBrowserCandidateConnections(request.active_note_path, 5)
      : [];
    const referenced = active.map((c) => c.target_path);
    const summaryLines = [
      `Análisis en modo **Vista previa navegador** sobre **${map.total_notes} notas**, **${map.total_links} enlaces** y **${map.total_tags} etiquetas**.`,
      request.active_note_path
        ? `Para [[${request.active_note_path.replace(/\.md$/i, '')}]], detecté **${active.length}** conexiones sugeridas.`
        : `Notas huérfanas detectadas: ${map.orphan_paths.length}.`,
    ];
    return {
      reply_markdown: summaryLines.join('\n\n'),
      provider_used: browserAiConfig.provider,
      model_used: browserAiConfig.model,
      referenced_notes: referenced,
      suggested_actions: [
        'Auto-conectar nota activa en modo Vista Previa',
        'Analizar carpeta Inbox/ en lote',
      ],
      warning: 'Modo navegador simulado: conecta Tauri Desktop/Android para ejecutar Claude CLI o Gemini.',
    };
  }
  return invoke<AiChatResponse>('ask_ai_copilot', { request });
}

export async function openExternalUrl(url: string): Promise<void> {
  const clean = url.trim();
  if (!clean) return;
  if (isTauriRuntime()) {
    try {
      await invoke('open_external_url', { url: clean });
      return;
    } catch (err) {
      console.warn('Error en open_external_url via Tauri, usando fallback:', err);
    }
  }
  window.open(clean, '_blank', 'noopener,noreferrer');
}
