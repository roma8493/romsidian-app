//! Puente de comandos IPC de Tauri 2 para ROMSIDIAN (`apps/desktop/src-tauri`).
//!
//! Conecta el frontend Svelte 5 con `romsidian-core` (bóveda, parser e índice SQLite FTS5)
//! y `romsidian-sync` (motor Git `git2` y almacén seguro de credenciales del SO).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;

pub mod ai_provider;

use ai_provider::{
    build_ai_provider_status, load_ai_config, normalize_safe_folder_filter, run_chat_with_provider,
    run_enrichment_with_provider, save_ai_config, update_ai_credentials, AiChatMessage,
    AiChatRequest, AiChatResponse, AiProviderConfig, AiProviderKind, AiProviderStatus,
    AutoConnectMode, AutoConnectNoteOutcome, AutoConnectRequest, ChatContextNote,
    ChatPromptContext, EnrichmentPromptContext, AI_KEYRING_SERVICE,
};
use romsidian_core::{
    apply_ai_enrichment, build_deterministic_enrichment_plan, build_vault_knowledge_map,
    eval_dataview_inline as core_eval_dataview_inline,
    eval_dataview_inline_batch as core_eval_dataview_inline_batch, execute_dataview_query,
    generate_knowledge_canvas, parse_note, preview_ai_enrichment,
    suggest_note_connections as core_suggest_note_connections, BacklinkHit, BlockRef,
    CandidateConnection, CanvasDocument, CanvasSummary, DailyNoteResult, DataviewInlineEvaluation,
    DataviewQueryResult, DataviewValue, FileTreeNode, FrontmatterValue,
    GraphData, HeadingItem, IndexStats, IndexedField, KnowledgeCanvasRequest,
    KnowledgeCanvasResult, KnowledgeNoteEntry, NoteSummary, SearchResult, TagCount,
    TemplateSummary, TransclusionResolution, UnlinkedMentionHit, Vault, VaultIndex, VaultInfo,
    VaultKnowledgeMap, WikiLink,
};
use romsidian_sync::{
    CredentialStore, GitSyncBackend, OsKeyringStore, SyncBackend, SyncConfig, SyncReport,
    SyncStatus,
};
use serde::{Deserialize, Serialize};
use tauri::State;

/// Instantánea completa del estado de la bóveda para poblar la interfaz al abrir o refrescar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultSnapshot {
    pub info: VaultInfo,
    pub tree: Vec<FileTreeNode>,
    pub notes: Vec<NoteSummary>,
    pub canvases: Vec<CanvasSummary>,
    pub tags: Vec<TagCount>,
    pub index_stats: IndexStats,
}

/// Detalle enriquecido de una nota abierta en el editor (contenido, metadatos, enlaces y backlinks).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDetail {
    pub relative_path: String,
    pub title: String,
    pub content: String,
    pub modified_unix_secs: u64,
    pub frontmatter: BTreeMap<String, FrontmatterValue>,
    pub wikilinks: Vec<WikiLink>,
    pub tags: Vec<String>,
    pub backlinks: Vec<BacklinkHit>,
}

/// Estado combinado de sincronización y configuración para la barra de estado y el modal Git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusDto {
    pub status: SyncStatus,
    pub config: SyncConfig,
    pub has_pat_token: bool,
}

/// Respuesta de auto-conexión PKM para una nota o carpeta en lote (`preview` o `apply`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoConnectBatchResponse {
    pub mode: AutoConnectMode,
    pub total_processed: usize,
    pub changed_count: usize,
    pub applied_count: usize,
    pub provider_used: AiProviderKind,
    pub items: Vec<AutoConnectNoteOutcome>,
    pub snapshot: Option<VaultSnapshot>,
}

/// Bóveda local gestionada o detectada por la aplicación.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalVaultEntry {
    pub name: String,
    pub path: String,
    pub is_default: bool,
}

/// Estado compartido del proceso desktop protegido por `Mutex`.
pub struct DesktopCore {
    active_vault_root: Option<PathBuf>,
    ai_credentials: Arc<dyn CredentialStore>,
}

impl Default for DesktopCore {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopCore {
    /// Crea una nueva instancia sin bóveda abierta inicialmente usando el llavero seguro del SO.
    pub fn new() -> Self {
        Self::with_credential_store(Arc::new(OsKeyringStore::with_service(AI_KEYRING_SERVICE)))
    }

    /// Crea una instancia con un almacén de credenciales inyectado (útil para tests aislados).
    pub fn with_credential_store(ai_credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            active_vault_root: None,
            ai_credentials,
        }
    }

    /// Resuelve la ruta por defecto a `test-vault/` si se encuentra en el árbol del proyecto.
    pub fn resolve_default_vault_path() -> Option<PathBuf> {
        if let Ok(cwd) = env::current_dir() {
            for ancestor in cwd.ancestors() {
                let candidate = ancestor.join("test-vault");
                if candidate.exists() && candidate.is_dir() {
                    return Some(candidate);
                }
            }
        }
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for ancestor in manifest_dir.ancestors() {
            let candidate = ancestor.join("test-vault");
            if candidate.exists() && candidate.is_dir() {
                return Some(candidate);
            }
        }
        None
    }

    /// Abre una carpeta como bóveda activa, sincroniza el índice SQLite FTS5 y devuelve su instantánea.
    pub fn open_vault(&mut self, path: impl AsRef<Path>) -> Result<VaultSnapshot, String> {
        let vault = Vault::open(path.as_ref()).map_err(|e| e.to_string())?;
        self.active_vault_root = Some(vault.root_path().to_path_buf());
        Self::build_snapshot(&vault)
    }

    /// Devuelve la instantánea actualizada de la bóveda activa.
    pub fn get_vault_snapshot(&self) -> Result<VaultSnapshot, String> {
        let vault = self.require_vault()?;
        Self::build_snapshot(&vault)
    }

    /// Lee una nota por su ruta relativa y adjunta su `frontmatter`, `wikilinks`, `tags` y `backlinks`.
    pub fn read_note(&self, relative_path: &str) -> Result<NoteDetail, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        Self::build_note_detail(&vault, &index, relative_path)
    }

    /// Crea una nueva nota `.md`, actualiza el índice incremental y devuelve su detalle.
    pub fn create_note(&self, relative_path: &str, content: &str) -> Result<NoteDetail, String> {
        let vault = self.require_vault()?;
        let doc = vault
            .create_note(relative_path, content)
            .map_err(|e| e.to_string())?;
        let (index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;
        Self::build_note_detail(&vault, &index, &doc.relative_path)
    }

    /// Guarda (crea o sobrescribe) una nota `.md`, actualiza el índice incremental y devuelve su detalle.
    pub fn save_note(&self, relative_path: &str, content: &str) -> Result<NoteDetail, String> {
        let vault = self.require_vault()?;
        let doc = vault
            .save_note(relative_path, content)
            .map_err(|e| e.to_string())?;
        let (index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;
        Self::build_note_detail(&vault, &index, &doc.relative_path)
    }

    /// Renombra o mueve una nota `.md`, actualiza el índice y devuelve el detalle en su nueva ruta.
    pub fn rename_note(&self, old_path: &str, new_path: &str) -> Result<NoteDetail, String> {
        let vault = self.require_vault()?;
        let summary = vault
            .rename_note(old_path, new_path)
            .map_err(|e| e.to_string())?;
        let (index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;
        Self::build_note_detail(&vault, &index, &summary.relative_path)
    }

    /// Elimina una nota `.md`, actualiza el índice y devuelve la nueva instantánea de la bóveda.
    pub fn delete_note(&self, relative_path: &str) -> Result<VaultSnapshot, String> {
        let vault = self.require_vault()?;
        vault
            .delete_note(relative_path)
            .map_err(|e| e.to_string())?;
        Self::build_snapshot(&vault)
    }

    /// Resuelve un `[[wikilink]]` a la ruta relativa canónica de la nota destino (si existe).
    pub fn resolve_wikilink(&self, target: &str) -> Result<Option<String>, String> {
        let vault = self.require_vault()?;
        vault
            .resolve_wikilink_path(target)
            .map_err(|e| e.to_string())
    }

    /// Obtiene los enlaces entrantes (*backlinks*) para una nota.
    pub fn get_backlinks(&self, relative_path: &str) -> Result<Vec<BacklinkHit>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index
            .backlinks_for(relative_path)
            .map_err(|e| e.to_string())
    }

    /// Ejecuta una búsqueda de texto completo (SQLite FTS5) o filtro por `#etiquetas`.
    pub fn search_notes(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<SearchResult>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index
            .search(query, limit.unwrap_or(30))
            .map_err(|e| e.to_string())
    }

    /// Lista todas las etiquetas de la bóveda con su conteo de notas.
    pub fn get_all_tags(&self) -> Result<Vec<TagCount>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index.list_all_tags().map_err(|e| e.to_string())
    }

    /// Lista las notas que contienen una `#etiqueta` (incluyendo sub-etiquetas jerárquicas).
    pub fn get_notes_by_tag(&self, tag: &str) -> Result<Vec<NoteSummary>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index.notes_by_tag(tag).map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------------------
    // Operaciones v0.2: Dataview DQL, Transclusiones, Menciones, Canvas y Diarias
    // ------------------------------------------------------------------------

    /// Ejecuta una consulta declarativa DQL (`TABLE`, `LIST`, `TASK`) sobre el índice SQLite.
    pub fn run_dataview_query(&self, query: &str) -> Result<DataviewQueryResult, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        execute_dataview_query(&index, Some(&vault), query).map_err(|e| e.to_string())
    }

    /// Evalúa una expresión inline `` `= this.*` `` en el contexto de una nota.
    pub fn eval_dataview_inline_expr(
        &self,
        note_path: &str,
        expression: &str,
    ) -> Result<DataviewValue, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        core_eval_dataview_inline(&index, Some(&vault), note_path, expression)
            .map_err(|e| e.to_string())
    }

    /// Evalúa un lote de expresiones inline `` `= this.*` `` para la nota indicada.
    pub fn eval_dataview_inline_batch_exprs(
        &self,
        note_path: &str,
        expressions: &[String],
    ) -> Result<Vec<DataviewInlineEvaluation>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        core_eval_dataview_inline_batch(&index, Some(&vault), note_path, expressions)
            .map_err(|e| e.to_string())
    }

    /// Devuelve todos los campos estructurados (frontmatter + inline `Clave:: Valor`) de una nota.
    pub fn get_note_fields(&self, relative_path: &str) -> Result<Vec<IndexedField>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index
            .fields_for_note(relative_path)
            .map_err(|e| e.to_string())
    }

    /// Devuelve los encabezados indexados (`# ...`) de una nota.
    pub fn get_note_headings(&self, relative_path: &str) -> Result<Vec<HeadingItem>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index
            .headings_for_note(relative_path)
            .map_err(|e| e.to_string())
    }

    /// Devuelve los bloques con ID explícito (`^block-id`) de una nota.
    pub fn get_note_blocks(&self, relative_path: &str) -> Result<Vec<BlockRef>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index
            .blocks_for_note(relative_path)
            .map_err(|e| e.to_string())
    }

    /// Resuelve una transclusión `![[Nota]]`, `![[Nota#Sección]]` o `![[Nota#^bloque]]`.
    pub fn resolve_transclusion(
        &self,
        raw_target: &str,
        subtarget: Option<String>,
    ) -> Result<Option<TransclusionResolution>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index
            .resolve_transclusion(&vault, raw_target, subtarget.as_deref())
            .map_err(|e| e.to_string())
    }

    /// Busca menciones no enlazadas del título de `relative_path` excluyendo los patrones indicados.
    pub fn get_unlinked_mentions(
        &self,
        relative_path: &str,
        excluded_patterns: Option<Vec<String>>,
    ) -> Result<Vec<UnlinkedMentionHit>, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        let patterns = excluded_patterns.unwrap_or_default();
        index
            .unlinked_mentions_for(relative_path, &patterns)
            .map_err(|e| e.to_string())
    }

    /// Convierte una mención no enlazada en `[[wikilink]]` y re-indexa la nota modificada.
    pub fn link_unlinked_mention(
        &self,
        source_path: &str,
        line: usize,
        matched_term: &str,
        target_note_path: &str,
    ) -> Result<NoteDetail, String> {
        let vault = self.require_vault()?;
        let mut index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        let doc = index
            .link_unlinked_mention(&vault, source_path, line, matched_term, target_note_path)
            .map_err(|e| e.to_string())?;
        Self::build_note_detail(&vault, &index, &doc.relative_path)
    }

    /// Lista todos los archivos `.canvas` de la bóveda activa.
    pub fn list_canvas_files(&self) -> Result<Vec<CanvasSummary>, String> {
        let vault = self.require_vault()?;
        vault.list_canvases().map_err(|e| e.to_string())
    }

    /// Crea un archivo JSON Canvas (`.canvas`) vacío o con contenido JSON inicial.
    pub fn create_canvas_file(
        &self,
        relative_path: &str,
        initial_content: Option<String>,
    ) -> Result<CanvasDocument, String> {
        let vault = self.require_vault()?;
        vault
            .create_canvas(relative_path, initial_content.as_deref().unwrap_or(""))
            .map_err(|e| e.to_string())
    }

    /// Lee un lienzo `.canvas` existente.
    pub fn read_canvas_file(&self, relative_path: &str) -> Result<CanvasDocument, String> {
        let vault = self.require_vault()?;
        vault.read_canvas(relative_path).map_err(|e| e.to_string())
    }

    /// Guarda un lienzo `.canvas` tras validar su estructura JSON.
    pub fn save_canvas_file(
        &self,
        relative_path: &str,
        content: &str,
    ) -> Result<CanvasDocument, String> {
        let vault = self.require_vault()?;
        vault
            .save_canvas(relative_path, content)
            .map_err(|e| e.to_string())
    }

    /// Renombra o mueve un archivo `.canvas` dentro de la bóveda.
    pub fn rename_canvas_file(
        &self,
        old_path: &str,
        new_path: &str,
    ) -> Result<CanvasSummary, String> {
        let vault = self.require_vault()?;
        vault
            .rename_canvas(old_path, new_path)
            .map_err(|e| e.to_string())
    }

    /// Elimina un archivo `.canvas` y devuelve la instantánea actualizada.
    pub fn delete_canvas_file(&self, relative_path: &str) -> Result<VaultSnapshot, String> {
        let vault = self.require_vault()?;
        vault.delete_canvas(relative_path).map_err(|e| e.to_string())?;
        Self::build_snapshot(&vault)
    }

    /// Abre o crea la nota diaria (`daily/YYYY-MM-DD.md`) e indexa la nueva nota si se creó.
    pub fn get_or_create_daily_note(
        &self,
        date_str: Option<String>,
        time_str: Option<String>,
        daily_folder: Option<String>,
        template_path: Option<String>,
    ) -> Result<DailyNoteResult, String> {
        let vault = self.require_vault()?;
        let res = vault
            .get_or_create_daily_note(
                date_str.as_deref(),
                time_str.as_deref(),
                daily_folder.as_deref(),
                template_path.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        if res.created {
            let _ = VaultIndex::open_and_sync(&vault);
        }
        Ok(res)
    }

    /// Lista las plantillas Markdown disponibles en la bóveda (`templates/`).
    pub fn list_templates(
        &self,
        templates_folder: Option<String>,
    ) -> Result<Vec<TemplateSummary>, String> {
        let vault = self.require_vault()?;
        vault
            .list_templates(templates_folder.as_deref())
            .map_err(|e| e.to_string())
    }

    /// Aplica una plantilla sustituyendo `{{date}}`, `{{time}}` y `{{title}}`.
    pub fn apply_template_content(
        &self,
        template_path: &str,
        target_title: &str,
        date_str: Option<String>,
        time_str: Option<String>,
    ) -> Result<String, String> {
        let vault = self.require_vault()?;
        vault
            .apply_template(
                template_path,
                target_title,
                date_str.as_deref(),
                time_str.as_deref(),
            )
            .map_err(|e| e.to_string())
    }

    /// Devuelve nodos y aristas del grafo de conocimiento 2D de la bóveda activa.
    pub fn get_graph_data(&self) -> Result<GraphData, String> {
        let vault = self.require_vault()?;
        let index = VaultIndex::open(&vault).map_err(|e| e.to_string())?;
        index.build_graph_data().map_err(|e| e.to_string())
    }

    /// Consulta el estado actual de sincronización Git y configuración de la bóveda activa.
    pub fn get_sync_status(&self) -> Result<SyncStatusDto, String> {
        let root = self.require_vault_root()?;
        let cfg = GitSyncBackend::load_config(&root).map_err(|e| e.to_string())?;
        let backend = GitSyncBackend::open_or_init(&root, cfg).map_err(|e| e.to_string())?;
        let status = backend.status().map_err(|e| e.to_string())?;
        let has_pat_token = backend.has_pat_token().unwrap_or(false);
        Ok(SyncStatusDto {
            status,
            config: backend.config().clone(),
            has_pat_token,
        })
    }

    /// Guarda la configuración Git en `.romsidian/sync.json` y actualiza el token PAT en el llavero del SO.
    pub fn save_sync_config(
        &self,
        config: SyncConfig,
        pat_token: Option<String>,
        clear_token: Option<bool>,
    ) -> Result<SyncStatusDto, String> {
        let root = self.require_vault_root()?;
        let mut backend =
            GitSyncBackend::open_or_init(&root, config.clone()).map_err(|e| e.to_string())?;
        backend.update_config(config).map_err(|e| e.to_string())?;

        if clear_token.unwrap_or(false) {
            backend.clear_pat_token().map_err(|e| e.to_string())?;
        } else if let Some(token) = pat_token {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                backend.set_pat_token(trimmed).map_err(|e| e.to_string())?;
            }
        }

        let status = backend.status().map_err(|e| e.to_string())?;
        let has_pat_token = backend.has_pat_token().unwrap_or(false);
        Ok(SyncStatusDto {
            status,
            config: backend.config().clone(),
            has_pat_token,
        })
    }

    /// Ejecuta `sync_now` (`commit_local` -> `pull_and_merge` -> `push`) y re-indexa la bóveda en SQLite.
    pub fn sync_vault_now(&self, timestamp_label: Option<String>) -> Result<SyncReport, String> {
        let root = self.require_vault_root()?;
        let cfg = GitSyncBackend::load_config(&root).map_err(|e| e.to_string())?;
        let mut backend = GitSyncBackend::open_or_init(&root, cfg).map_err(|e| e.to_string())?;

        let label = timestamp_label
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "2026-11-11".to_string());

        let report_res = backend.sync_now(&label);

        // Siempre re-sincronizamos el índice SQLite FTS5 por si se descargaron notas o se crearon copias en conflicto
        if let Ok(vault) = Vault::open(&root) {
            let _ = VaultIndex::open_and_sync(&vault);
        }

        report_res.map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------------------
    // Operaciones v0.5: Agente IA, Mapa de Conocimiento y Auto-Conexión PKM
    // ------------------------------------------------------------------------

    /// Devuelve configuración activa de IA y disponibilidad de CLIs locales y claves seguras.
    pub fn get_ai_provider_status(&self) -> Result<AiProviderStatus, String> {
        let root = self.require_vault_root()?;
        build_ai_provider_status(&root, self.ai_credentials.as_ref())
    }

    /// Guarda configuración de proveedor IA en `.romsidian/ai_config.json` y claves en llavero del SO.
    pub fn save_ai_provider_config(
        &self,
        config: AiProviderConfig,
        gemini_api_key: Option<String>,
        compatible_api_key: Option<String>,
        clear_gemini_api_key: Option<bool>,
        clear_compatible_api_key: Option<bool>,
    ) -> Result<AiProviderStatus, String> {
        let root = self.require_vault_root()?;
        save_ai_config(&root, &config)?;
        update_ai_credentials(
            self.ai_credentials.as_ref(),
            gemini_api_key.as_deref(),
            compatible_api_key.as_deref(),
            clear_gemini_api_key.unwrap_or(false),
            clear_compatible_api_key.unwrap_or(false),
        )?;
        build_ai_provider_status(&root, self.ai_credentials.as_ref())
    }

    /// Genera el mapa compacto de conocimiento de toda la bóveda (0 tokens, desde SQLite).
    pub fn get_vault_knowledge_map(&self) -> Result<VaultKnowledgeMap, String> {
        let vault = self.require_vault()?;
        let (index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;
        build_vault_knowledge_map(&index).map_err(|e| e.to_string())
    }

    /// Sugiere conexiones candidatas para una nota combinando menciones, tags, campos y FTS5 BM25.
    pub fn suggest_note_connections(
        &self,
        relative_path: &str,
        max_candidates: Option<usize>,
    ) -> Result<Vec<CandidateConnection>, String> {
        let vault = self.require_vault()?;
        let (index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;
        let limit = max_candidates.unwrap_or(10).clamp(1, 50);
        core_suggest_note_connections(&index, relative_path, limit).map_err(|e| e.to_string())
    }

    /// Previsualiza o aplica auto-conexión PKM (`frontmatter`, `#tags`, `[[wikilinks]]`)
    /// sobre una nota, una selección o toda una carpeta (`Inbox/`).
    pub fn auto_connect_notes(
        &self,
        request: AutoConnectRequest,
    ) -> Result<AutoConnectBatchResponse, String> {
        let vault = self.require_vault()?;
        let root = vault.root_path().to_path_buf();
        let cfg = load_ai_config(&root)?;
        let targets = Self::collect_auto_connect_targets(&vault, &request)?;
        let max_connections = request.max_connections.unwrap_or(cfg.max_connections).clamp(1, 25);
        let append_related_section = request
            .append_related_section
            .unwrap_or(cfg.append_related_section);

        let (mut index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;
        let vault_map = build_vault_knowledge_map(&index).map_err(|e| e.to_string())?;

        let mut items = Vec::with_capacity(targets.len());
        let mut changed_count = 0usize;
        let mut applied_count = 0usize;
        let mut last_provider = request.provider_override.unwrap_or(cfg.provider);

        for rel_path in targets {
            let doc = vault.read_note(&rel_path).map_err(|e| e.to_string())?;
            let parsed = parse_note(&doc.content);
            let effective_title = parsed.title.unwrap_or_else(|| doc.title.clone());

            let (plan, provider_used, warning) = match &request.custom_plan {
                Some(custom) => (
                    custom.clone(),
                    request.provider_override.unwrap_or(cfg.provider),
                    None,
                ),
                None => {
                    let candidates = core_suggest_note_connections(
                        &index,
                        &doc.relative_path,
                        max_connections.saturating_mul(2),
                    )
                    .map_err(|e| e.to_string())?;
                    let deterministic_plan = build_deterministic_enrichment_plan(
                        &vault,
                        &index,
                        &doc.relative_path,
                        max_connections,
                        append_related_section,
                    )
                    .map_err(|e| e.to_string())?;

                    let ctx = EnrichmentPromptContext {
                        vault_map: &vault_map,
                        note_path: &doc.relative_path,
                        note_title: &effective_title,
                        note_content: &doc.content,
                        candidates: &candidates,
                        deterministic_plan: &deterministic_plan,
                        max_connections,
                        append_related_section,
                    };
                    run_enrichment_with_provider(
                        &cfg,
                        request.provider_override,
                        Some(root.as_path()),
                        self.ai_credentials.as_ref(),
                        &ctx,
                    )?
                }
            };

            last_provider = provider_used;
            let (preview, applied) = match request.mode {
                AutoConnectMode::Apply => {
                    let res =
                        apply_ai_enrichment(&vault, &mut index, &doc.relative_path, &plan)
                            .map_err(|e| e.to_string())?;
                    let did_apply = res.preview.changed;
                    (res.preview, did_apply)
                }
                AutoConnectMode::Preview => (
                    preview_ai_enrichment(&doc.relative_path, &doc.content, &plan),
                    false,
                ),
            };

            if preview.changed {
                changed_count += 1;
            }
            if applied {
                applied_count += 1;
            }
            items.push(AutoConnectNoteOutcome {
                path: doc.relative_path,
                plan,
                preview,
                applied,
                provider_used,
                warning,
            });
        }

        let snapshot = if applied_count > 0 {
            Some(Self::build_snapshot(&vault)?)
        } else {
            None
        };

        Ok(AutoConnectBatchResponse {
            mode: request.mode,
            total_processed: items.len(),
            changed_count,
            applied_count,
            provider_used: last_provider,
            items,
            snapshot,
        })
    }

    /// Genera o actualiza un lienzo visual `.canvas` conectando notas semilla y sugeridas.
    pub fn create_knowledge_canvas(
        &self,
        request: KnowledgeCanvasRequest,
    ) -> Result<KnowledgeCanvasResult, String> {
        let vault = self.require_vault()?;
        let (index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;
        generate_knowledge_canvas(&vault, &index, &request).map_err(|e| e.to_string())
    }

    /// Responde una consulta libre al Copiloto PKM inyectando mapa compacto, nota activa y FTS5.
    pub fn ask_ai_copilot(&self, request: AiChatRequest) -> Result<AiChatResponse, String> {
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err("Escribe una pregunta o instrucción para el Copiloto PKM.".to_string());
        }

        let vault = self.require_vault()?;
        let root = vault.root_path().to_path_buf();
        let cfg = load_ai_config(&root)?;
        let (index, _) = VaultIndex::open_and_sync(&vault).map_err(|e| e.to_string())?;

        let vault_map = if request.include_vault_map.unwrap_or(true) {
            Some(build_vault_knowledge_map(&index).map_err(|e| e.to_string())?)
        } else {
            None
        };

        let mut active_note_resolved: Option<String> = None;
        let mut active_note_content: Option<String> = None;
        let mut active_candidates: Vec<CandidateConnection> = Vec::new();
        if let Some(raw_active) = request
            .active_note_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Ok(doc) = vault.read_note(raw_active) {
                active_candidates =
                    core_suggest_note_connections(&index, &doc.relative_path, 8)
                        .unwrap_or_default();
                active_note_resolved = Some(doc.relative_path);
                active_note_content = Some(doc.content);
            }
        }

        let search_hits = Self::gather_chat_search_hits(&index, prompt);
        let history = request.history.unwrap_or_default();
        let context_notes = Self::gather_chat_context_notes(
            &vault,
            vault_map.as_ref(),
            prompt,
            &history,
            active_note_resolved.as_deref(),
            active_note_content.is_some(),
            &search_hits,
        );
        let ctx = ChatPromptContext {
            prompt,
            vault_root: Some(root.as_path()),
            vault_map: vault_map.as_ref(),
            active_note_path: active_note_resolved.as_deref(),
            active_note_content: active_note_content.as_deref(),
            active_candidates: &active_candidates,
            context_notes: &context_notes,
            search_hits: &search_hits,
            history: &history,
        };

        let (reply_markdown, provider_used, model_used, warning) = run_chat_with_provider(
            &cfg,
            request.provider_override,
            Some(root.as_path()),
            self.ai_credentials.as_ref(),
            &ctx,
        )?;

        let mut referenced_notes = Vec::new();
        let mut seen_refs = HashSet::new();
        for p in active_note_resolved
            .iter()
            .chain(context_notes.iter().map(|n| &n.path))
            .chain(search_hits.iter().map(|h| &h.path))
            .chain(active_candidates.iter().map(|c| &c.target_path))
        {
            if seen_refs.insert(p.to_lowercase()) {
                referenced_notes.push(p.clone());
                if referenced_notes.len() >= 12 {
                    break;
                }
            }
        }

        let mut suggested_actions = Vec::new();
        if active_note_resolved.is_some()
            && active_candidates.iter().any(|c| !c.already_linked)
        {
            suggested_actions.push("Auto-Conectar Nota Activa (Vista Previa)".to_string());
        }
        if vault_map
            .as_ref()
            .map(|m| !m.orphan_paths.is_empty())
            .unwrap_or(false)
        {
            suggested_actions.push("Conectar notas huérfanas o carpeta Inbox/".to_string());
        }
        suggested_actions.push("Generar Mapa .canvas de Conexiones".to_string());

        Ok(AiChatResponse {
            reply_markdown,
            provider_used,
            model_used,
            referenced_notes,
            suggested_actions,
            warning,
        })
    }

    fn collect_auto_connect_targets(
        vault: &Vault,
        request: &AutoConnectRequest,
    ) -> Result<Vec<String>, String> {
        let mut targets = Vec::new();
        let mut seen = HashSet::new();

        if let Some(single) = request.path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if seen.insert(single.to_lowercase()) {
                targets.push(single.to_string());
            }
        }

        if let Some(list) = &request.paths {
            for item in list {
                let trimmed = item.trim();
                if !trimmed.is_empty() && seen.insert(trimmed.to_lowercase()) {
                    targets.push(trimmed.to_string());
                }
            }
        }

        if let Some(raw_folder) = request.folder.as_deref() {
            let clean_folder = normalize_safe_folder_filter(raw_folder)?;
            let notes = vault.list_notes().map_err(|e| e.to_string())?;
            let folder_prefix = if clean_folder.is_empty() {
                String::new()
            } else {
                format!("{}/", clean_folder.to_lowercase())
            };

            for note in notes {
                let rel_lower = note.relative_path.to_lowercase();
                let matches = clean_folder.is_empty()
                    || rel_lower.starts_with(&folder_prefix)
                    || rel_lower == format!("{}.md", clean_folder.to_lowercase());
                if matches && seen.insert(rel_lower) {
                    targets.push(note.relative_path);
                }
            }
        }

        if targets.is_empty() {
            return Err(
                "No se encontraron notas objetivo: especifica 'path', 'paths' o una carpeta válida."
                    .to_string(),
            );
        }
        Ok(targets)
    }

    fn gather_chat_search_hits(index: &VaultIndex, prompt: &str) -> Vec<SearchResult> {
        let keywords = extract_chat_keywords(prompt);
        if keywords.is_empty() {
            return Vec::new();
        }
        let combined = keywords.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
        let direct = index.search(&combined, 6).unwrap_or_default();
        if !direct.is_empty() || keywords.len() == 1 {
            return direct;
        }

        let mut merged = Vec::new();
        let mut seen = HashSet::new();
        for kw in keywords.iter().take(4) {
            if let Ok(hits) = index.search(kw, 3) {
                for hit in hits {
                    if seen.insert(hit.path.to_lowercase()) {
                        merged.push(hit);
                        if merged.len() >= 6 {
                            return merged;
                        }
                    }
                }
            }
        }
        merged
    }

    /// Selecciona y lee el contenido de las notas relevantes para responder la consulta en el chat PKM.
    fn gather_chat_context_notes(
        vault: &Vault,
        vault_map: Option<&VaultKnowledgeMap>,
        prompt: &str,
        history: &[AiChatMessage],
        active_note_path: Option<&str>,
        active_note_has_content: bool,
        search_hits: &[SearchResult],
    ) -> Vec<ChatContextNote> {
        const MAX_CONTEXT_NOTES: usize = 10;

        let mut query_parts = vec![prompt.to_lowercase()];
        for msg in history.iter().rev().take(4) {
            query_parts.push(msg.content.to_lowercase());
        }
        let combined_query_lower = query_parts.join("\n");

        let is_active_loaded = |rel_path: &str| {
            active_note_has_content
                && active_note_path
                    .map(|p| p.eq_ignore_ascii_case(rel_path))
                    .unwrap_or(false)
        };

        let mut out = Vec::new();
        let Some(map) = vault_map else {
            let mut seen = HashSet::new();
            for hit in search_hits {
                if out.len() >= MAX_CONTEXT_NOTES {
                    break;
                }
                if is_active_loaded(&hit.path) || !seen.insert(hit.path.to_lowercase()) {
                    continue;
                }
                let folder = hit.path.rsplit_once('/').map(|(f, _)| f).unwrap_or("");
                if let Some(note) = build_chat_context_note(
                    vault,
                    &hit.path,
                    &hit.title,
                    folder,
                    &[],
                    "Coincidencia FTS5".to_string(),
                ) {
                    out.push(note);
                }
            }
            return out;
        };

        let mut selected: Vec<(&KnowledgeNoteEntry, String)> = Vec::new();
        let mut seen_paths = HashSet::new();

        // 1. Coincidencia por carpeta (ej. /inbox, inbox/, "carpeta inbox").
        let matched_folders = collect_matched_folders(&map.folders, &combined_query_lower);
        if !matched_folders.is_empty() {
            for entry in &map.notes {
                let entry_folder = entry.folder.to_lowercase();
                let in_folder = matched_folders.iter().any(|mf| {
                    entry_folder == *mf || entry_folder.starts_with(&format!("{mf}/"))
                });
                if in_folder {
                    let reason = if entry.folder.is_empty() {
                        "Carpeta raíz".to_string()
                    } else {
                        format!("Carpeta '{}'", entry.folder)
                    };
                    if !try_select_context_note(
                        &mut selected,
                        &mut seen_paths,
                        active_note_path,
                        active_note_has_content,
                        MAX_CONTEXT_NOTES,
                        entry,
                        reason,
                    ) {
                        break;
                    }
                }
            }
        }

        // 2. Coincidencia por ruta, título, stem, alias o #etiqueta.
        if selected.len() < MAX_CONTEXT_NOTES {
            for entry in &map.notes {
                if let Some(reason) = find_note_mention_reason(&combined_query_lower, entry) {
                    if !try_select_context_note(
                        &mut selected,
                        &mut seen_paths,
                        active_note_path,
                        active_note_has_content,
                        MAX_CONTEXT_NOTES,
                        entry,
                        reason,
                    ) {
                        break;
                    }
                }
            }
        }

        // 3. Coincidencia por búsqueda léxica FTS5.
        if selected.len() < MAX_CONTEXT_NOTES {
            let entry_by_path: HashMap<String, &KnowledgeNoteEntry> = map
                .notes
                .iter()
                .map(|e| (e.path.to_lowercase(), e))
                .collect();
            for hit in search_hits {
                if let Some(entry) = entry_by_path.get(&hit.path.to_lowercase()) {
                    if !try_select_context_note(
                        &mut selected,
                        &mut seen_paths,
                        active_note_path,
                        active_note_has_content,
                        MAX_CONTEXT_NOTES,
                        entry,
                        "Coincidencia FTS5".to_string(),
                    ) {
                        break;
                    }
                }
            }
        }

        // 4. Consulta global o bóveda pequeña (<= 12 notas): completar contexto útil.
        let asks_all = [
            "todas mis notas",
            "todas las notas",
            "toda la boveda",
            "toda la bóveda",
            "resumen de la boveda",
            "resumen de la bóveda",
            "resumen general",
        ]
        .iter()
        .any(|phrase| combined_query_lower.contains(phrase));

        if selected.len() < MAX_CONTEXT_NOTES
            && ((asks_all && matched_folders.is_empty()) || selected.is_empty() || map.notes.len() <= 12)
        {
            for entry in &map.notes {
                if !try_select_context_note(
                    &mut selected,
                    &mut seen_paths,
                    active_note_path,
                    active_note_has_content,
                    MAX_CONTEXT_NOTES,
                    entry,
                    "Contexto general de la bóveda".to_string(),
                ) {
                    break;
                }
            }
        }

        for (entry, reason) in selected {
            if let Some(note) = build_chat_context_note(
                vault,
                &entry.path,
                &entry.title,
                &entry.folder,
                &entry.tags,
                reason,
            ) {
                out.push(note);
            }
        }
        out
    }

    fn require_vault_root(&self) -> Result<PathBuf, String> {
        self.active_vault_root
            .clone()
            .ok_or_else(|| "No hay ninguna bóveda abierta actualmente.".to_string())
    }

    fn require_vault(&self) -> Result<Vault, String> {
        let root = self.require_vault_root()?;
        Vault::open(root).map_err(|e| e.to_string())
    }

    fn build_snapshot(vault: &Vault) -> Result<VaultSnapshot, String> {
        let (index, index_stats) = VaultIndex::open_and_sync(vault).map_err(|e| e.to_string())?;
        let tree = vault.file_tree().map_err(|e| e.to_string())?;
        let notes = vault.list_notes().map_err(|e| e.to_string())?;
        let canvases = vault.list_canvases().map_err(|e| e.to_string())?;
        let tags = index.list_all_tags().map_err(|e| e.to_string())?;
        Ok(VaultSnapshot {
            info: vault.info().clone(),
            tree,
            notes,
            canvases,
            tags,
            index_stats,
        })
    }

    fn build_note_detail(
        vault: &Vault,
        index: &VaultIndex,
        relative_path: &str,
    ) -> Result<NoteDetail, String> {
        let doc = vault.read_note(relative_path).map_err(|e| e.to_string())?;
        let parsed = parse_note(&doc.content);
        let backlinks = index
            .backlinks_for(&doc.relative_path)
            .map_err(|e| e.to_string())?;
        let effective_title = parsed.title.unwrap_or_else(|| doc.title.clone());

        Ok(NoteDetail {
            relative_path: doc.relative_path,
            title: effective_title,
            content: doc.content,
            modified_unix_secs: doc.modified_unix_secs,
            frontmatter: parsed.frontmatter,
            wikilinks: parsed.wikilinks,
            tags: parsed.tags,
            backlinks,
        })
    }
}

// ============================================================================
// Comandos IPC expuestos a Tauri 2 (`#[tauri::command]`)
// ============================================================================

#[tauri::command]
fn get_default_vault_path(app: tauri::AppHandle) -> Result<Option<String>, String> {
    if let Some(p) = DesktopCore::resolve_default_vault_path() {
        return Ok(Some(p.to_string_lossy().replace('\\', "/")));
    }
    #[cfg(target_os = "android")]
    {
        if let Ok(data_dir) = app.path().app_data_dir() {
            let vault_dir = data_dir.join("vault");
            let _ = std::fs::create_dir_all(&vault_dir);
            return Ok(Some(vault_dir.to_string_lossy().replace('\\', "/")));
        }
    }
    let _ = app;
    Ok(None)
}

fn sanitize_vault_folder_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return Err("Ingresá un nombre válido para la bóveda.".to_string());
    }
    let cleaned: String = trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            _ => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let final_name = collapsed.trim_matches(|c| c == '.' || c == '-').trim().to_string();
    if final_name.is_empty() {
        return Err("El nombre de la bóveda no puede estar vacío.".to_string());
    }
    Ok(final_name)
}

#[tauri::command]
fn list_local_vaults(app: tauri::AppHandle) -> Result<Vec<LocalVaultEntry>, String> {
    let mut entries = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    if let Some(default_desktop) = DesktopCore::resolve_default_vault_path() {
        let normalized = default_desktop.to_string_lossy().replace('\\', "/");
        if seen_paths.insert(normalized.clone()) {
            let name = default_desktop
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("test-vault")
                .to_string();
            entries.push(LocalVaultEntry {
                name,
                path: normalized,
                is_default: true,
            });
        }
    }

    if let Ok(data_dir) = app.path().app_data_dir() {
        let primary_vault = data_dir.join("vault");
        if primary_vault.exists() && primary_vault.is_dir() {
            let normalized = primary_vault.to_string_lossy().replace('\\', "/");
            if seen_paths.insert(normalized.clone()) {
                entries.push(LocalVaultEntry {
                    name: "Principal".to_string(),
                    path: normalized,
                    is_default: cfg!(target_os = "android"),
                });
            }
        }

        let vaults_dir = data_dir.join("vaults");
        if vaults_dir.exists() && vaults_dir.is_dir() {
            if let Ok(read_dir) = std::fs::read_dir(&vaults_dir) {
                let mut sub_entries = Vec::new();
                for item in read_dir.flatten() {
                    let path = item.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if name.starts_with('.') {
                        continue;
                    }
                    let normalized = path.to_string_lossy().replace('\\', "/");
                    if seen_paths.insert(normalized.clone()) {
                        sub_entries.push(LocalVaultEntry {
                            name: name.to_string(),
                            path: normalized,
                            is_default: false,
                        });
                    }
                }
                sub_entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                entries.extend(sub_entries);
            }
        }
    }

    Ok(entries)
}

#[tauri::command]
fn create_local_vault(name: String, app: tauri::AppHandle) -> Result<LocalVaultEntry, String> {
    let clean_name = sanitize_vault_folder_name(&name)?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("No se pudo obtener el directorio de datos de la app: {e}"))?;
    let target = data_dir.join("vaults").join(&clean_name);
    std::fs::create_dir_all(&target)
        .map_err(|e| format!("Error al crear la carpeta de la bóveda: {e}"))?;

    Ok(LocalVaultEntry {
        name: clean_name,
        path: target.to_string_lossy().replace('\\', "/"),
        is_default: false,
    })
}

#[tauri::command]
fn open_vault(
    path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<VaultSnapshot, String> {
    let mut core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.open_vault(path)
}

#[tauri::command]
fn get_vault_snapshot(state: State<'_, Mutex<DesktopCore>>) -> Result<VaultSnapshot, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_vault_snapshot()
}

#[tauri::command]
fn read_note(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<NoteDetail, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.read_note(&relative_path)
}

#[tauri::command]
fn create_note(
    relative_path: String,
    content: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<NoteDetail, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.create_note(&relative_path, &content)
}

#[tauri::command]
fn save_note(
    relative_path: String,
    content: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<NoteDetail, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.save_note(&relative_path, &content)
}

#[tauri::command]
fn rename_note(
    old_path: String,
    new_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<NoteDetail, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.rename_note(&old_path, &new_path)
}

#[tauri::command]
fn delete_note(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<VaultSnapshot, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.delete_note(&relative_path)
}

#[tauri::command]
fn resolve_wikilink(
    target: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Option<String>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.resolve_wikilink(&target)
}

#[tauri::command]
fn get_backlinks(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<BacklinkHit>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_backlinks(&relative_path)
}

#[tauri::command]
fn search_notes(
    query: String,
    limit: Option<usize>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<SearchResult>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.search_notes(&query, limit)
}

#[tauri::command]
fn get_all_tags(state: State<'_, Mutex<DesktopCore>>) -> Result<Vec<TagCount>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_all_tags()
}

#[tauri::command]
fn get_notes_by_tag(
    tag: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<NoteSummary>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_notes_by_tag(&tag)
}

#[tauri::command]
fn get_sync_status(state: State<'_, Mutex<DesktopCore>>) -> Result<SyncStatusDto, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_sync_status()
}

#[tauri::command]
fn save_sync_config(
    config: SyncConfig,
    pat_token: Option<String>,
    clear_token: Option<bool>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<SyncStatusDto, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.save_sync_config(config, pat_token, clear_token)
}

#[tauri::command]
fn sync_vault_now(
    timestamp_label: Option<String>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<SyncReport, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.sync_vault_now(timestamp_label)
}

#[tauri::command]
fn run_dataview_query(
    query: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<DataviewQueryResult, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.run_dataview_query(&query)
}

#[tauri::command]
fn eval_dataview_inline(
    note_path: String,
    expression: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<DataviewValue, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.eval_dataview_inline_expr(&note_path, &expression)
}

#[tauri::command]
fn eval_dataview_inline_batch(
    note_path: String,
    expressions: Vec<String>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<DataviewInlineEvaluation>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.eval_dataview_inline_batch_exprs(&note_path, &expressions)
}

#[tauri::command]
fn get_note_fields(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<IndexedField>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_note_fields(&relative_path)
}

#[tauri::command]
fn get_note_headings(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<HeadingItem>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_note_headings(&relative_path)
}

#[tauri::command]
fn get_note_blocks(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<BlockRef>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_note_blocks(&relative_path)
}

#[tauri::command]
fn resolve_transclusion(
    raw_target: String,
    subtarget: Option<String>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Option<TransclusionResolution>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.resolve_transclusion(&raw_target, subtarget)
}

#[tauri::command]
fn get_unlinked_mentions(
    relative_path: String,
    excluded_patterns: Option<Vec<String>>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<UnlinkedMentionHit>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_unlinked_mentions(&relative_path, excluded_patterns)
}

#[tauri::command]
fn link_unlinked_mention(
    source_path: String,
    line: usize,
    matched_term: String,
    target_note_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<NoteDetail, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.link_unlinked_mention(&source_path, line, &matched_term, &target_note_path)
}

#[tauri::command]
fn list_canvas_files(state: State<'_, Mutex<DesktopCore>>) -> Result<Vec<CanvasSummary>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.list_canvas_files()
}

#[tauri::command]
fn create_canvas_file(
    relative_path: String,
    initial_content: Option<String>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<CanvasDocument, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.create_canvas_file(&relative_path, initial_content)
}

#[tauri::command]
fn read_canvas_file(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<CanvasDocument, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.read_canvas_file(&relative_path)
}

#[tauri::command]
fn save_canvas_file(
    relative_path: String,
    content: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<CanvasDocument, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.save_canvas_file(&relative_path, &content)
}

#[tauri::command]
fn rename_canvas_file(
    old_path: String,
    new_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<CanvasSummary, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.rename_canvas_file(&old_path, &new_path)
}

#[tauri::command]
fn delete_canvas_file(
    relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<VaultSnapshot, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.delete_canvas_file(&relative_path)
}

#[tauri::command]
fn get_or_create_daily_note(
    date_str: Option<String>,
    time_str: Option<String>,
    daily_folder: Option<String>,
    template_path: Option<String>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<DailyNoteResult, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_or_create_daily_note(date_str, time_str, daily_folder, template_path)
}

#[tauri::command]
fn list_templates(
    templates_folder: Option<String>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<TemplateSummary>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.list_templates(templates_folder)
}

#[tauri::command]
fn apply_template_content(
    template_path: String,
    target_title: String,
    date_str: Option<String>,
    time_str: Option<String>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<String, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.apply_template_content(&template_path, &target_title, date_str, time_str)
}

#[tauri::command]
fn get_graph_data(state: State<'_, Mutex<DesktopCore>>) -> Result<GraphData, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_graph_data()
}

#[tauri::command]
fn pick_vault_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let script = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = "Seleccionar carpeta de bóveda para ROMSIDIAN"
$dialog.ShowNewFolderButton = $true
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output $dialog.SelectedPath
}
"#;

        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Error al abrir explorador de carpetas: {e}"))?;

        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Ok(Some(path_str));
            }
        }
        Ok(None)
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(data_dir) = app.path().app_data_dir() {
            let vault_dir = data_dir.join("vault");
            let _ = std::fs::create_dir_all(&vault_dir);
            return Ok(Some(vault_dir.to_string_lossy().replace('\\', "/")));
        }
        Ok(None)
    }
}

#[tauri::command]
fn create_folder(folder_path: String, state: State<'_, Mutex<DesktopCore>>) -> Result<(), String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    let vault_root = core.active_vault_root.as_ref().ok_or("No hay bóveda abierta")?;
    let clean = folder_path.trim().replace('\\', "/").trim_matches('/').to_string();
    if clean.is_empty() || clean.contains("..") {
        return Err("Ruta de carpeta inválida".to_string());
    }
    let target = vault_root.join(&clean);
    std::fs::create_dir_all(&target).map_err(|e| format!("Error al crear carpeta: {e}"))?;
    Ok(())
}

#[tauri::command]
fn delete_folder(folder_path: String, state: State<'_, Mutex<DesktopCore>>) -> Result<(), String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    let vault_root = core.active_vault_root.as_ref().ok_or("No hay bóveda abierta")?;
    let clean = folder_path.trim().replace('\\', "/").trim_matches('/').to_string();
    if clean.is_empty() || clean.contains("..") {
        return Err("Ruta de carpeta inválida".to_string());
    }
    let target = vault_root.join(&clean);
    if !target.exists() || !target.is_dir() {
        return Err(format!("La carpeta no existe: {clean}"));
    }
    std::fs::remove_dir_all(&target).map_err(|e| format!("Error al eliminar carpeta: {e}"))?;
    if let Ok(vault) = Vault::open(vault_root) {
        let _ = VaultIndex::open_and_sync(&vault);
    }
    Ok(())
}

#[tauri::command]
fn move_vault_item(
    source_relative_path: String,
    target_folder_relative_path: String,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<String, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    let vault_root = core.active_vault_root.as_ref().ok_or("No hay bóveda abierta")?;
    let src_clean = source_relative_path.trim().replace('\\', "/").trim_matches('/').to_string();
    let target_clean = target_folder_relative_path.trim().replace('\\', "/").trim_matches('/').to_string();
    if src_clean.is_empty() || src_clean.contains("..") || target_clean.contains("..") {
        return Err("Ruta inválida".to_string());
    }
    if target_clean == src_clean || target_clean.starts_with(&format!("{src_clean}/")) {
        return Err("No se puede mover una carpeta dentro de sí misma o de sus subcarpetas".to_string());
    }
    let src_path = vault_root.join(&src_clean);
    if !src_path.exists() {
        return Err(format!("El elemento origen no existe: {src_clean}"));
    }
    let file_name = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Nombre de archivo inválido")?;
    let dest_folder = if target_clean.is_empty() {
        vault_root.clone()
    } else {
        vault_root.join(&target_clean)
    };
    std::fs::create_dir_all(&dest_folder).map_err(|e| format!("Error al crear carpeta destino: {e}"))?;
    let dest_path = dest_folder.join(file_name);
    if dest_path.exists() && dest_path != src_path {
        return Err(format!("Ya existe un elemento con el nombre '{file_name}' en la carpeta destino"));
    }
    if dest_path != src_path {
        std::fs::rename(&src_path, &dest_path).map_err(|e| format!("Error al mover: {e}"))?;
    }
    let new_rel = if target_clean.is_empty() {
        file_name.to_string()
    } else {
        format!("{target_clean}/{file_name}")
    };
    Ok(new_rel)
}

fn try_select_context_note<'a>(
    selected: &mut Vec<(&'a KnowledgeNoteEntry, String)>,
    seen_paths: &mut HashSet<String>,
    active_note_path: Option<&str>,
    active_note_has_content: bool,
    max_notes: usize,
    entry: &'a KnowledgeNoteEntry,
    reason: String,
) -> bool {
    if selected.len() >= max_notes {
        return false;
    }
    if active_note_has_content
        && active_note_path
            .map(|p| p.eq_ignore_ascii_case(&entry.path))
            .unwrap_or(false)
    {
        return true;
    }
    if seen_paths.insert(entry.path.to_lowercase()) {
        selected.push((entry, reason));
    }
    selected.len() < max_notes
}

fn build_chat_context_note(
    vault: &Vault,
    rel_path: &str,
    fallback_title: &str,
    fallback_folder: &str,
    fallback_tags: &[String],
    match_reason: String,
) -> Option<ChatContextNote> {
    let doc = vault.read_note(rel_path).ok()?;
    let parsed = parse_note(&doc.content);
    Some(ChatContextNote {
        path: doc.relative_path,
        title: parsed.title.unwrap_or_else(|| {
            if !doc.title.is_empty() {
                doc.title
            } else {
                fallback_title.to_string()
            }
        }),
        folder: fallback_folder.to_string(),
        tags: if parsed.tags.is_empty() {
            fallback_tags.to_vec()
        } else {
            parsed.tags
        },
        content: doc.content,
        match_reason,
    })
}

fn contains_whole_phrase(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(rel_idx) = haystack[start..].find(needle) {
        let idx = start + rel_idx;
        let end = idx + needle.len();
        let left_ok = haystack[..idx]
            .chars()
            .next_back()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        let right_ok = haystack[end..]
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        if left_ok && right_ok {
            return true;
        }
        let advance = haystack[idx..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
        start = idx + advance;
    }
    false
}

fn mentions_folder(query_lower: &str, folder_lower: &str) -> bool {
    if folder_lower.is_empty() {
        return false;
    }
    if contains_whole_phrase(query_lower, folder_lower) {
        return true;
    }
    if let Some((_, basename)) = folder_lower.rsplit_once('/') {
        if basename.chars().count() >= 3 && contains_whole_phrase(query_lower, basename) {
            return true;
        }
    }
    false
}

fn collect_matched_folders(folders: &[String], query_lower: &str) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    for folder in folders {
        let trimmed = folder.trim_matches('/').to_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        candidates.insert(trimmed.clone());
        let mut prefix = String::new();
        for seg in trimmed.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(seg);
            candidates.insert(prefix.clone());
        }
    }
    candidates
        .into_iter()
        .filter(|f| mentions_folder(query_lower, f))
        .collect()
}

fn find_note_mention_reason(query_lower: &str, entry: &KnowledgeNoteEntry) -> Option<String> {
    if query_lower.contains(&entry.path.to_lowercase()) {
        return Some("Ruta citada en la consulta".to_string());
    }
    if entry.stem.chars().count() >= 3
        && contains_whole_phrase(query_lower, &entry.stem.to_lowercase())
    {
        return Some("Título citado en la consulta".to_string());
    }
    if entry.title.chars().count() >= 3
        && contains_whole_phrase(query_lower, &entry.title.to_lowercase())
    {
        return Some("Título citado en la consulta".to_string());
    }
    if entry.aliases.iter().any(|a| {
        a.chars().count() >= 3 && contains_whole_phrase(query_lower, &a.to_lowercase())
    }) {
        return Some("Alias citado en la consulta".to_string());
    }
    for tag in &entry.tags {
        let tag_lower = tag.to_lowercase();
        if query_lower.contains(&format!("#{tag_lower}"))
            || (tag_lower.chars().count() >= 3
                && contains_whole_phrase(query_lower, &tag_lower))
        {
            return Some(format!("Etiqueta #{tag}"));
        }
    }
    None
}

fn extract_chat_keywords(prompt: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "que", "qué", "cual", "cuál", "como", "cómo", "para", "sobre", "entre", "desde", "hacia",
        "donde", "dónde", "con", "sin", "una", "uno", "unos", "unas", "los", "las", "del", "por",
        "nota", "notas", "archivo", "archivos", "boveda", "bóveda", "hola", "podes", "podés",
        "puedes", "armar", "hacer", "hace", "dame", "decime", "quiero", "necesito", "resumen",
        "resumir", "todas", "todos", "toda", "todo", "mis", "carpeta", "carpetas", "favor",
        "explica", "explicar", "analizar", "analiza", "leer", "lee", "the", "and", "for", "with",
        "from", "what", "about", "this", "that",
    ];
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in prompt.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                current.push(lc);
            }
        } else if !current.is_empty() {
            if current.chars().count() >= 3 && !STOPWORDS.contains(&current.as_str()) {
                words.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if !current.is_empty()
        && current.chars().count() >= 3
        && !STOPWORDS.contains(&current.as_str())
    {
        words.push(current);
    }

    words.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    words.dedup();
    words
}

// ============================================================================
// Comandos IPC v0.5 (Agente IA, Mapa de Conocimiento y Auto-Conexión)
// ============================================================================

#[tauri::command]
fn get_ai_provider_status(state: State<'_, Mutex<DesktopCore>>) -> Result<AiProviderStatus, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_ai_provider_status()
}

#[tauri::command]
fn save_ai_provider_config(
    config: AiProviderConfig,
    gemini_api_key: Option<String>,
    compatible_api_key: Option<String>,
    clear_gemini_api_key: Option<bool>,
    clear_compatible_api_key: Option<bool>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<AiProviderStatus, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.save_ai_provider_config(
        config,
        gemini_api_key,
        compatible_api_key,
        clear_gemini_api_key,
        clear_compatible_api_key,
    )
}

#[tauri::command]
fn get_vault_knowledge_map(state: State<'_, Mutex<DesktopCore>>) -> Result<VaultKnowledgeMap, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.get_vault_knowledge_map()
}

#[tauri::command]
fn suggest_note_connections_cmd(
    relative_path: String,
    max_candidates: Option<usize>,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<Vec<CandidateConnection>, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.suggest_note_connections(&relative_path, max_candidates)
}

#[tauri::command]
fn auto_connect_notes_cmd(
    request: AutoConnectRequest,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<AutoConnectBatchResponse, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.auto_connect_notes(request)
}

#[tauri::command]
fn create_knowledge_canvas_cmd(
    request: KnowledgeCanvasRequest,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<KnowledgeCanvasResult, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.create_knowledge_canvas(request)
}

#[tauri::command]
fn ask_ai_copilot(
    request: AiChatRequest,
    state: State<'_, Mutex<DesktopCore>>,
) -> Result<AiChatResponse, String> {
    let core = state.lock().map_err(|_| "Estado bloqueado".to_string())?;
    core.ask_ai_copilot(request)
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let clean = url.trim();
    if !clean.starts_with("http://") && !clean.starts_with("https://") && !clean.starts_with("mailto:") {
        return Err("Protocolo no permitido para enlace externo".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", clean])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Error abriendo URL externa en Windows: {e}"))?;
        Ok(())
    }

    #[cfg(target_os = "android")]
    {
        let _ = clean;
        Ok(())
    }

    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        std::process::Command::new("xdg-open")
            .arg(clean)
            .spawn()
            .map_err(|e| format!("Error abriendo URL externa: {e}"))?;
        Ok(())
    }
}

/// Punto de entrada principal del runtime Tauri 2.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut initial_core = DesktopCore::new();
    if let Some(default_vault) = DesktopCore::resolve_default_vault_path() {
        let _ = initial_core.open_vault(default_vault);
    }

    tauri::Builder::default()
        .manage(Mutex::new(initial_core))
        .setup(|app| {
            #[cfg(target_os = "android")]
            {
                if let Ok(data_dir) = app.path().app_data_dir() {
                    romsidian_sync::set_android_secrets_dir(data_dir.join(".secrets"));
                    let vault_dir = data_dir.join("vault");
                    let _ = std::fs::create_dir_all(&vault_dir);
                    if let Ok(mut core) = app.state::<Mutex<DesktopCore>>().lock() {
                        if core.active_vault_root.is_none() {
                            let _ = core.open_vault(&vault_dir);
                        }
                    }
                }
            }
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_default_vault_path,
            list_local_vaults,
            create_local_vault,
            open_vault,
            get_vault_snapshot,
            read_note,
            create_note,
            save_note,
            rename_note,
            delete_note,
            resolve_wikilink,
            get_backlinks,
            search_notes,
            get_all_tags,
            get_notes_by_tag,
            get_sync_status,
            save_sync_config,
            sync_vault_now,
            run_dataview_query,
            eval_dataview_inline,
            eval_dataview_inline_batch,
            get_note_fields,
            get_note_headings,
            get_note_blocks,
            resolve_transclusion,
            get_unlinked_mentions,
            link_unlinked_mention,
            list_canvas_files,
            create_canvas_file,
            read_canvas_file,
            save_canvas_file,
            rename_canvas_file,
            delete_canvas_file,
            get_or_create_daily_note,
            list_templates,
            apply_template_content,
            get_graph_data,
            pick_vault_folder,
            create_folder,
            delete_folder,
            move_vault_item,
            get_ai_provider_status,
            save_ai_provider_config,
            get_vault_knowledge_map,
            suggest_note_connections_cmd,
            auto_connect_notes_cmd,
            create_knowledge_canvas_cmd,
            ask_ai_copilot,
            open_external_url,
        ])
        .run(tauri::generate_context!())
        .expect("Error al iniciar la aplicación de escritorio ROMSIDIAN");
}

#[cfg(test)]
mod tests {
    use super::*;
    use romsidian_sync::MemoryCredentialStore;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempVaultGuard {
        path: PathBuf,
    }

    impl TempVaultGuard {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let dir = env::temp_dir().join(format!(
                "romsidian_desktop_{prefix}_{}_{}",
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&dir).expect("crear carpeta temporal de test");
            Self { path: dir }
        }
    }

    impl Drop for TempVaultGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn copy_non_hidden_dir(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).expect("crear carpeta destino");
        for entry in fs::read_dir(src).expect("leer carpeta origen").flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let target = dst.join(name);
            if path.is_dir() {
                copy_non_hidden_dir(&path, &target);
            } else {
                fs::copy(&path, &target).expect("copiar archivo de prueba");
            }
        }
    }

    fn seed_canonical_test_notes(vault_dir: &Path) {
        let seed_notes = [
            (
                "Índice General.md",
                "---\ntitle: \"Índice General de la Bóveda\"\ntags:\n  - indice\n  - organizacion/boveda\n  - inicio\n---\n\n# Índice General de la Bóveda\n\nMapa principal de temas en ROMSIDIAN:\n- [[Análisis Macroeconómico]]\n- [[Arquitectura de RomSidian]]\n- [[Sincronización Móvil y Git]]\n- [[Notas de Investigación]]\n",
            ),
            (
                "Análisis Macroeconómico.md",
                "---\ntitle: \"Análisis Macroeconómico\"\ntags:\n  - economía\n  - economía/argentina\n---\n\n# Análisis Macroeconómico\n\nSeguimiento de indicadores clave como la inflación mensual, política monetaria y relación técnica con [[Arquitectura de RomSidian]].\nVolver al [[Índice General]].\n",
            ),
            (
                "Arquitectura de RomSidian.md",
                "---\ntitle: \"Arquitectura de RomSidian\"\ntags:\n  - arquitectura\n  - rust\n---\n\n# Arquitectura de RomSidian\n\nEl núcleo escrito en Rust se conecta con [[Sincronización Móvil y Git]] y se lista en el [[Índice General]].\n",
            ),
            (
                "Sincronización Móvil y Git.md",
                "---\ntitle: \"Sincronización Móvil y Git\"\ntags:\n  - git\n  - android\n---\n\n# Sincronización Móvil y Git\n\nFlujo de sincronización con [[Arquitectura de RomSidian]] y GitHub.\n",
            ),
            (
                "subcarpeta/Notas de Investigación.md",
                "---\ntitle: \"Notas de Investigación\"\ntags:\n  - investigacion\n---\n\n# Notas de Investigación\n\nSíntesis vinculada a [[Análisis Macroeconómico]] y al [[Índice General]].\n",
            ),
        ];

        for (rel, content) in seed_notes {
            let target = vault_dir.join(rel);
            if !target.exists() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).expect("crear subcarpeta semilla");
                }
                fs::write(&target, content.as_bytes()).expect("escribir nota semilla");
            }
        }
    }

    #[test]
    fn desktop_core_opens_test_vault_and_serves_ipc_queries() {
        let test_vault_path = DesktopCore::resolve_default_vault_path()
            .expect("Debe encontrar test-vault en el workspace");

        let temp = TempVaultGuard::new("seed_copy");
        copy_non_hidden_dir(&test_vault_path, &temp.path);
        seed_canonical_test_notes(&temp.path);

        let mut core = DesktopCore::with_credential_store(Arc::new(MemoryCredentialStore::new()));
        let snapshot = core
            .open_vault(&temp.path)
            .expect("Abrir copia temporal de test-vault desde DesktopCore");

        assert!(snapshot.notes.len() >= 5);
        assert!(!snapshot.tags.is_empty());

        let detail = core
            .read_note("Análisis Macroeconómico.md")
            .expect("Leer nota con tildes");
        assert!(detail.content.contains("[[Arquitectura de RomSidian]]"));
        assert!(!detail.backlinks.is_empty());

        let search_hits = core
            .search_notes("inflacion", Some(10))
            .expect("Buscar FTS5 sin tilde");
        assert!(
            search_hits
                .iter()
                .any(|h| h.path == "Análisis Macroeconómico.md"),
            "FTS5 debe encontrar Análisis Macroeconómico.md al buscar 'inflacion'"
        );

        let resolved = core
            .resolve_wikilink("análisis macroeconómico")
            .expect("Resolver wikilink en minúsculas");
        assert_eq!(resolved.as_deref(), Some("Análisis Macroeconómico.md"));

        let graph = core.get_graph_data().expect("Construir grafo 2D");
        assert!(graph.nodes.len() >= 5);
        assert!(!graph.edges.is_empty());
    }

    #[test]
    fn desktop_core_serves_v02_dataview_canvas_daily_and_graph_commands() {
        let temp = TempVaultGuard::new("v02");
        let mut core = DesktopCore::with_credential_store(Arc::new(MemoryCredentialStore::new()));
        core.open_vault(&temp.path).unwrap();

        core.create_note(
            "templates/daily.md",
            "# Diario {{title}}\nfecha:: {{date}}\n- [ ] Tarea diaria\n",
        )
        .unwrap();

        let daily = core
            .get_or_create_daily_note(
                Some("2026-11-12".to_string()),
                Some("08:15".to_string()),
                None,
                None,
            )
            .unwrap();
        assert!(daily.created);
        assert_eq!(daily.note.relative_path, "daily/2026-11-12.md");

        let dv = core
            .run_dataview_query("TABLE fecha FROM \"daily\"")
            .expect("Ejecutar consulta TABLE desde DesktopCore");
        match dv {
            DataviewQueryResult::Table { rows, total_count, .. } => {
                assert_eq!(total_count, 1);
                assert_eq!(
                    rows[0][1],
                    DataviewValue::Text {
                        value: "2026-11-12".to_string()
                    }
                );
            }
            other => panic!("Se esperaba Table, obtuvo {other:?}"),
        }

        let inline = core
            .eval_dataview_inline_expr("daily/2026-11-12.md", "`= this.file.name`")
            .unwrap();
        assert_eq!(
            inline,
            DataviewValue::Text {
                value: "2026-11-12".to_string()
            }
        );

        let canvas = core
            .create_canvas_file("Diagrama.canvas", None)
            .expect("Crear archivo .canvas desde DesktopCore");
        assert_eq!(canvas.relative_path, "Diagrama.canvas");
        assert_eq!(core.list_canvas_files().unwrap().len(), 1);
    }

    #[test]
    fn desktop_core_serves_v05_ai_provider_knowledge_map_and_auto_connect() {
        let temp = TempVaultGuard::new("v05_ai");
        fs::create_dir_all(temp.path.join("Inbox")).unwrap();
        fs::write(
            temp.path.join("Macroeconomia.md"),
            "---\ntitle: Macroeconomía\naliases: [\"Economía Global\"]\ntags: [macroeconomia]\nestado: activo\n---\n\n# Macroeconomía\n\nAnálisis de inflación y tasas de interés.\n",
        )
        .unwrap();
        fs::write(
            temp.path.join("Politica Monetaria.md"),
            "---\ntitle: Política Monetaria\ntags: [macroeconomia, bancos-centrales]\n---\n\n# Política Monetaria\n\nImpacto de [[Macroeconomia|Macroeconomía]] en el crédito.\n",
        )
        .unwrap();
        fs::write(
            temp.path.join("Inbox/Video YouTube.md"),
            "---\nsource: youtube\ntags: [youtube]\n---\n\n# Video sobre Economía Global\n\nResumen sobre la Economía Global y decisiones de Política Monetaria.\n\n```\nEconomía Global dentro de código no se debe modificar.\n```\n",
        )
        .unwrap();

        let store = Arc::new(MemoryCredentialStore::new());
        let mut core = DesktopCore::with_credential_store(store.clone());
        core.open_vault(&temp.path).unwrap();

        // 1. Configuración y resguardo de API Key fuera de .romsidian/ai_config.json
        let saved_status = core
            .save_ai_provider_config(
                AiProviderConfig {
                    provider: AiProviderKind::Deterministic,
                    model: "gemini-2.5-flash".to_string(),
                    ..Default::default()
                },
                Some("AIza-secret-test-key".to_string()),
                None,
                None,
                None,
            )
            .expect("Guardar configuración IA");
        assert!(saved_status.has_gemini_api_key);
        assert!(!saved_status.has_compatible_api_key);

        let cfg_raw = fs::read_to_string(temp.path.join(".romsidian/ai_config.json"))
            .expect("Debe crearse .romsidian/ai_config.json");
        assert!(
            !cfg_raw.contains("AIza-secret-test-key"),
            "La API Key jamás debe guardarse dentro de la bóveda"
        );
        assert_eq!(
            store
                .get_token(ai_provider::GEMINI_API_KEY_ACCOUNT)
                .unwrap()
                .as_deref(),
            Some("AIza-secret-test-key")
        );

        // 2. Mapa de conocimiento y sugerencias
        let map = core
            .get_vault_knowledge_map()
            .expect("Generar mapa de conocimiento");
        assert_eq!(map.total_notes, 3);

        let suggestions = core
            .suggest_note_connections("Inbox/Video YouTube.md", Some(5))
            .expect("Sugerir conexiones");
        assert!(!suggestions.is_empty());

        // 3. Auto-conexión en lote (Preview -> Apply -> Re-Apply idempotente)
        let preview_resp = core
            .auto_connect_notes(AutoConnectRequest {
                folder: Some("Inbox/".to_string()),
                mode: AutoConnectMode::Preview,
                ..Default::default()
            })
            .expect("Previsualizar lote Inbox/");
        assert_eq!(preview_resp.total_processed, 1);
        assert_eq!(preview_resp.changed_count, 1);
        assert_eq!(preview_resp.applied_count, 0);
        assert!(preview_resp.snapshot.is_none());

        let apply_resp = core
            .auto_connect_notes(AutoConnectRequest {
                folder: Some("Inbox".to_string()),
                mode: AutoConnectMode::Apply,
                ..Default::default()
            })
            .expect("Aplicar lote Inbox/");
        assert_eq!(apply_resp.total_processed, 1);
        assert_eq!(apply_resp.changed_count, 1);
        assert_eq!(apply_resp.applied_count, 1);
        assert!(apply_resp.snapshot.is_some());

        let updated = core.read_note("Inbox/Video YouTube.md").unwrap();
        assert!(updated.content.contains("[[Macroeconomia|Economía Global]]"));
        assert!(
            updated
                .content
                .contains("```\nEconomía Global dentro de código no se debe modificar.\n```")
        );

        let second_pass = core
            .auto_connect_notes(AutoConnectRequest {
                folder: Some("Inbox".to_string()),
                mode: AutoConnectMode::Apply,
                ..Default::default()
            })
            .expect("Segunda pasada debe ser idempotente");
        assert_eq!(second_pass.changed_count, 0);
        assert_eq!(second_pass.applied_count, 0);

        // Rechazo de rutas inseguras en lote
        assert!(core
            .auto_connect_notes(AutoConnectRequest {
                folder: Some("../".to_string()),
                mode: AutoConnectMode::Preview,
                ..Default::default()
            })
            .is_err());

        // 4. Generación de lienzo .canvas de conocimiento
        let canvas_res = core
            .create_knowledge_canvas(KnowledgeCanvasRequest {
                canvas_path: "Mapas/Conocimiento.canvas".to_string(),
                title: Some("Fuentes Macro".to_string()),
                seed_note_paths: vec!["Inbox/Video YouTube.md".to_string()],
                include_neighbors: true,
                max_nodes: 10,
            })
            .expect("Crear mapa .canvas");
        assert!(canvas_res.node_count >= 2);
        assert!(temp.path.join("Mapas/Conocimiento.canvas").exists());

        // 5. Chat libre con contexto de carpeta completa (/inbox) + fallback limpio
        core.create_note(
            "Inbox/Segundo Video.md",
            "---\nsource: youtube\ntags: [youtube, gemini]\n---\n\n# Segundo Video\n\nAnálisis sobre Gemini 4 Hydrogen y automatización.\n",
        )
        .unwrap();

        let inbox_chat = core
            .ask_ai_copilot(AiChatRequest {
                prompt: "Hola, podes armar un resumen de todas mis notas de la carpeta /inbox ?".to_string(),
                active_note_path: Some("Inbox/Video YouTube.md".to_string()),
                include_vault_map: Some(true),
                provider_override: Some(AiProviderKind::Deterministic),
                history: None,
            })
            .expect("Responder consulta de carpeta /inbox sin pedir permisos");
        assert!(inbox_chat
            .referenced_notes
            .iter()
            .any(|p| p == "Inbox/Video YouTube.md"));
        assert!(inbox_chat
            .referenced_notes
            .iter()
            .any(|p| p == "Inbox/Segundo Video.md"));
        assert!(inbox_chat.reply_markdown.contains("Inbox/Segundo Video.md"));

        let chat_resp = core
            .ask_ai_copilot(AiChatRequest {
                prompt: "¿Qué notas hablan sobre inflación?".to_string(),
                active_note_path: Some("Inbox/Video YouTube.md".to_string()),
                include_vault_map: Some(true),
                provider_override: Some(AiProviderKind::Deterministic),
                history: None,
            })
            .expect("Responder consulta al Copiloto PKM");
        assert_eq!(chat_resp.provider_used, AiProviderKind::Deterministic);
        assert!(chat_resp.reply_markdown.contains("Macroeconomia") || !chat_resp.referenced_notes.is_empty());

        // Limpiar API Key del llavero
        let cleared_status = core
            .save_ai_provider_config(saved_status.active_config, None, None, Some(true), None)
            .unwrap();
        assert!(!cleared_status.has_gemini_api_key);
    }

    #[test]
    fn test_sanitize_vault_folder_name() {
        assert_eq!(sanitize_vault_folder_name("  Trabajo 2026  ").unwrap(), "Trabajo 2026");
        assert_eq!(sanitize_vault_folder_name("Proyecto/Alpha:Beta").unwrap(), "Proyecto-Alpha-Beta");
        assert!(sanitize_vault_folder_name("").is_err());
        assert!(sanitize_vault_folder_name("..").is_err());
        assert!(sanitize_vault_folder_name("///").is_err());
    }
}
