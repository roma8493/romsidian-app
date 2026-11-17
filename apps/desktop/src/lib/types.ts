export interface VaultInfo {
  name: string;
  root_path: string;
  metadata_dir: string;
}

export interface LocalVaultEntry {
  name: string;
  path: string;
  is_default: boolean;
}

export interface NoteSummary {
  relative_path: string;
  title: string;
  modified_unix_secs: number;
  size_bytes: number;
}

export type FileTreeNode =
  | {
      kind: 'folder';
      name: string;
      relative_path: string;
      children: FileTreeNode[];
    }
  | ({
      kind: 'note';
    } & NoteSummary);

export interface TagCount {
  tag: string;
  count: number;
}

export interface IndexStats {
  total_notes: number;
  indexed_or_updated: number;
  skipped_unchanged: number;
  removed_deleted: number;
}

export interface CanvasSummary {
  relative_path: string;
  title: string;
  modified_unix_secs: number;
  size_bytes: number;
}

export interface CanvasDocument {
  relative_path: string;
  title: string;
  content: string;
  modified_unix_secs: number;
}

export interface TemplateSummary {
  relative_path: string;
  name: string;
}

export interface DailyNoteResult {
  note: NoteDetail;
  created: boolean;
}

export interface VaultSnapshot {
  info: VaultInfo;
  tree: FileTreeNode[];
  notes: NoteSummary[];
  canvases: CanvasSummary[];
  tags: TagCount[];
  index_stats: IndexStats;
}

export type FrontmatterValue = string | string[];

export interface WikiLink {
  target: string;
  heading: string | null;
  alias: string | null;
  line: number;
  context: string;
  is_embed: boolean;
}

export interface BacklinkHit {
  source_path: string;
  source_title: string;
  target_raw: string;
  alias: string | null;
  heading: string | null;
  line: number;
  context: string;
  is_embed: boolean;
}

export interface NoteDetail {
  relative_path: string;
  title: string;
  content: string;
  modified_unix_secs: number;
  frontmatter: Record<string, FrontmatterValue>;
  wikilinks: WikiLink[];
  tags: string[];
  backlinks: BacklinkHit[];
}

export interface SearchResult {
  path: string;
  title: string;
  snippet: string;
  score: number;
  tags: string[];
}

export type SyncState =
  | 'Idle'
  | 'Syncing'
  | 'UpToDate'
  | { ConflictDetected: { conflict_files: string[] } }
  | { Error: { message: string } };

export interface SyncConfig {
  remote_url: string;
  branch: string;
  device_name: string;
  author_name: string;
  author_email: string;
}

export interface SyncStatus {
  state: SyncState;
  branch: string;
  remote_url: string | null;
  modified_files: string[];
  conflict_files: string[];
  last_commit_id: string | null;
}

export interface SyncStatusDto {
  status: SyncStatus;
  config: SyncConfig;
  has_pat_token: boolean;
}

export interface SyncReport {
  state: SyncState;
  committed_files: number;
  merged_cleanly: string[];
  conflict_copies_created: string[];
  pushed: boolean;
  head_commit: string | null;
}

export interface IndexedField {
  path: string;
  key: string;
  key_norm: string;
  value: string;
  source: string;
  line: number;
}

export interface HeadingItem {
  level: number;
  text: string;
  line: number;
}

export interface BlockRef {
  id: string;
  line: number;
  text: string;
}

export interface TransclusionResolution {
  resolved_path: string;
  title: string;
  subtarget: string | null;
  content: string;
}

export interface UnlinkedMentionHit {
  source_path: string;
  source_title: string;
  matched_term: string;
  line: number;
  column_start: number;
  column_end: number;
  context: string;
}

export type DataviewValue =
  | { kind: 'null' }
  | { kind: 'bool'; value: boolean }
  | { kind: 'number'; value: number }
  | { kind: 'text'; value: string }
  | { kind: 'link'; path: string; label: string }
  | { kind: 'list'; items: DataviewValue[] };

export interface DataviewListItem {
  path: string;
  title: string;
  value: DataviewValue | null;
}

export interface DataviewTaskItem {
  path: string;
  note_title: string;
  line: number;
  completed: boolean;
  text: string;
  tags: string[];
}

export type DataviewQueryResult =
  | {
      view: 'table';
      headers: string[];
      rows: DataviewValue[][];
      total_count: number;
    }
  | {
      view: 'list';
      items: DataviewListItem[];
      total_count: number;
    }
  | {
      view: 'task';
      items: DataviewTaskItem[];
      total_count: number;
    };

export interface DataviewInlineEvaluation {
  expression: string;
  value: DataviewValue;
}

export interface GraphNode {
  id: string;
  label: string;
  tags: string[];
  degree: number;
  folder: string;
  exists: boolean;
}

export interface GraphEdge {
  source: string;
  target: string;
  resolved: boolean;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export type BacklinksLocation = 'right-panel' | 'document-footer' | 'pinned-tab';

export interface WorkspaceSettings {
  dailyFolder: string;
  templatesFolder: string;
  youtubeInboxFolder: string;
  dailyTemplatePath: string;
  enableDataviewJs: boolean;
  backlinksLocation: BacklinksLocation;
  backlinksExcludedFiles: string[];
}

export interface CommandAction {
  id: string;
  label: string;
  category: string;
  shortcut?: string;
  keywords?: string;
  disabled?: boolean;
  run: () => void | Promise<void>;
}

export type CanvasSide = 'top' | 'right' | 'bottom' | 'left';
export type CanvasEdgeEnd = 'none' | 'arrow';
export type CanvasNodeType = 'text' | 'file' | 'link' | 'group';

export interface CanvasBaseNode {
  id: string;
  type: CanvasNodeType;
  x: number;
  y: number;
  width: number;
  height: number;
  color?: string;
}

export interface CanvasTextNode extends CanvasBaseNode {
  type: 'text';
  text: string;
}

export interface CanvasFileNode extends CanvasBaseNode {
  type: 'file';
  file: string;
  subpath?: string;
  mediaKind?: 'note' | 'media';
}

export interface CanvasLinkNode extends CanvasBaseNode {
  type: 'link';
  url: string;
}

export interface CanvasGroupNode extends CanvasBaseNode {
  type: 'group';
  label?: string;
  background?: string;
  backgroundStyle?: 'cover' | 'ratio' | 'repeat';
}

export type CanvasJsonNode =
  | CanvasTextNode
  | CanvasFileNode
  | CanvasLinkNode
  | CanvasGroupNode;

export interface CanvasJsonEdge {
  id: string;
  fromNode: string;
  fromSide?: CanvasSide;
  fromEnd?: CanvasEdgeEnd;
  toNode: string;
  toSide?: CanvasSide;
  toEnd?: CanvasEdgeEnd;
  color?: string;
  label?: string;
}

export interface CanvasJsonData {
  nodes: CanvasJsonNode[];
  edges: CanvasJsonEdge[];
}

export interface KnowledgeNoteEntry {
  path: string;
  stem: string;
  title: string;
  folder: string;
  aliases: string[];
  tags: string[];
  headings: string[];
  frontmatter_keys: string[];
  outlink_count: number;
  inlink_count: number;
  is_orphan: boolean;
}

export interface FrontmatterKeyStat {
  key: string;
  count: number;
  sample_values: string[];
}

export interface VaultKnowledgeMap {
  total_notes: number;
  total_links: number;
  total_tags: number;
  orphan_paths: string[];
  folders: string[];
  tag_taxonomy: TagCount[];
  frontmatter_schema: FrontmatterKeyStat[];
  notes: KnowledgeNoteEntry[];
}

export type ConnectionSignal =
  | {
      kind: 'outgoing_unlinked_mention';
      matched_term: string;
      line: number;
      context: string;
    }
  | {
      kind: 'incoming_unlinked_mention';
      matched_term: string;
      line: number;
      context: string;
    }
  | {
      kind: 'shared_tags';
      tags: string[];
    }
  | {
      kind: 'shared_fields';
      fields: string[];
    }
  | {
      kind: 'lexical_bm25';
      score: number;
      matched_keywords: string[];
    };

export interface WikilinkReplacement {
  target_path: string;
  target_stem: string;
  matched_term: string;
  replacement_wikilink: string;
  line: number;
  context: string;
}

export interface CandidateConnection {
  target_path: string;
  target_title: string;
  target_stem: string;
  target_tags: string[];
  score: number;
  already_linked: boolean;
  suggested_wikilink: string;
  inline_replacements: WikilinkReplacement[];
  signals: ConnectionSignal[];
  reason_summary: string;
}

export interface RelatedConnectionLink {
  target_path: string;
  target_title: string;
  relation: string | null;
}

export interface AiEnrichmentPlan {
  frontmatter_updates: Record<string, FrontmatterValue>;
  overwrite_existing_frontmatter: boolean;
  suggested_tags: string[];
  inline_wikilinks: WikilinkReplacement[];
  related_connections: RelatedConnectionLink[];
  append_related_section: boolean;
}

export interface AiEnrichmentPreview {
  relative_path: string;
  original_content: string;
  enriched_content: string;
  changed: boolean;
  added_frontmatter_keys: string[];
  added_tags: string[];
  injected_wikilinks: WikilinkReplacement[];
  appended_related_links: string[];
  unified_diff_lines: string[];
}

export interface KnowledgeCanvasRequest {
  canvas_path: string;
  title?: string | null;
  seed_note_paths?: string[];
  include_neighbors?: boolean;
  max_nodes?: number;
}

export interface KnowledgeCanvasResult {
  canvas: CanvasDocument;
  node_count: number;
  edge_count: number;
}

export type AiProviderKind =
  | 'deterministic'
  | 'claude_cli'
  | 'gemini_cli'
  | 'gemini_http'
  | 'compatible_http';

export interface AiProviderConfig {
  provider: AiProviderKind;
  model: string;
  endpoint_url: string | null;
  cli_binary_override: string | null;
  max_connections: number;
  append_related_section: boolean;
  fallback_to_deterministic: boolean;
}

export interface AiProviderStatus {
  active_config: AiProviderConfig;
  claude_cli_available: boolean;
  claude_cli_path: string | null;
  gemini_cli_available: boolean;
  gemini_cli_path: string | null;
  has_gemini_api_key: boolean;
  has_compatible_api_key: boolean;
  platform: string;
}

export type AutoConnectMode = 'preview' | 'apply';

export interface AutoConnectRequest {
  path?: string | null;
  paths?: string[] | null;
  folder?: string | null;
  mode?: AutoConnectMode;
  max_connections?: number | null;
  append_related_section?: boolean | null;
  custom_plan?: AiEnrichmentPlan | null;
  provider_override?: AiProviderKind | null;
}

export interface AutoConnectNoteOutcome {
  path: string;
  plan: AiEnrichmentPlan;
  preview: AiEnrichmentPreview;
  applied: boolean;
  provider_used: AiProviderKind;
  warning: string | null;
}

export interface AutoConnectBatchResponse {
  mode: AutoConnectMode;
  total_processed: number;
  changed_count: number;
  applied_count: number;
  provider_used: AiProviderKind;
  items: AutoConnectNoteOutcome[];
  snapshot: VaultSnapshot | null;
}

export interface AiChatMessage {
  role: string;
  content: string;
}

export interface AiChatRequest {
  prompt: string;
  active_note_path?: string | null;
  include_vault_map?: boolean | null;
  provider_override?: AiProviderKind | null;
  history?: AiChatMessage[] | null;
}

export interface AiChatResponse {
  reply_markdown: string;
  provider_used: AiProviderKind;
  model_used: string;
  referenced_notes: string[];
  suggested_actions: string[];
  warning: string | null;
}

export interface GithubRepoInfo {
  full_name: string;
  name: string;
  owner: string;
  private: boolean;
  html_url: string;
  clone_url: string;
  default_branch: string;
  description: string | null;
  updated_at: string;
}


