//! Adaptador multi-proveedor de IA y auto-conexión PKM (`[RF-16]`).
//!
//! Unifica cuatro backends bajo el trait [`AiProvider`]:
//! - `Deterministic`: motor local en Rust de 0 tokens y latencia `< 50 ms`.
//! - `ClaudeCli` / `GeminiCli`: puente `stdio` hacia CLI locales autenticados con la suscripción del usuario.
//! - `GeminiHttp`: cliente HTTPS (`ureq` + `rustls`, compatible Windows y Android) contra Google AI Studio.
//! - `CompatibleHttp`: cliente HTTPS/local compatible con API OpenAI (`/v1/chat/completions`).
//!
//! Las claves API se guardan exclusivamente en [`CredentialStore`] (Windows Credential Manager o
//! almacén privado Android), nunca dentro de la bóveda ni en `.romsidian/ai_config.json`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use romsidian_core::{
    AiEnrichmentPlan, AiEnrichmentPreview, CandidateConnection, FrontmatterValue,
    KnowledgeNoteEntry, RelatedConnectionLink, SearchResult, VaultKnowledgeMap,
    WikilinkReplacement, VAULT_METADATA_DIR,
};
use romsidian_sync::CredentialStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Servicio de llavero aislado para credenciales de proveedores IA en el SO.
pub const AI_KEYRING_SERVICE: &str = "ROMSIDIAN_AI_PROVIDER";
/// Identificador de cuenta en llavero para la clave de Google AI Studio (Gemini HTTP).
pub const GEMINI_API_KEY_ACCOUNT: &str = "gemini_http_api_key";
/// Identificador de cuenta en llavero para la clave de endpoint compatible OpenAI.
pub const COMPATIBLE_API_KEY_ACCOUNT: &str = "compatible_http_api_key";
/// Nombre del archivo de configuración no sensible dentro de `.romsidian/`.
pub const AI_CONFIG_FILE: &str = "ai_config.json";

/// Proveedores de IA soportados por ROMSIDIAN (`[RF-16]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    Deterministic,
    ClaudeCli,
    GeminiCli,
    GeminiHttp,
    CompatibleHttp,
}

impl Default for AiProviderKind {
    fn default() -> Self {
        Self::Deterministic
    }
}

impl AiProviderKind {
    /// Modelo recomendado por defecto según el proveedor elegido.
    pub fn default_model(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic-rust-v1",
            Self::ClaudeCli => "claude-sonnet-4-5",
            Self::GeminiCli => "gemini-2.5-flash",
            Self::GeminiHttp => "gemini-2.5-flash",
            Self::CompatibleHttp => "gpt-4o-mini",
        }
    }
}

/// Configuración persistida en `.romsidian/ai_config.json` (sin credenciales).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderConfig {
    #[serde(default)]
    pub provider: AiProviderKind,
    #[serde(default = "default_model_str")]
    pub model: String,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub cli_binary_override: Option<String>,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_true")]
    pub append_related_section: bool,
    #[serde(default = "default_true")]
    pub fallback_to_deterministic: bool,
}

fn default_model_str() -> String {
    AiProviderKind::Deterministic.default_model().to_string()
}

fn default_max_connections() -> usize {
    8
}

fn default_true() -> bool {
    true
}

impl Default for AiProviderConfig {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::Deterministic,
            model: default_model_str(),
            endpoint_url: None,
            cli_binary_override: None,
            max_connections: default_max_connections(),
            append_related_section: true,
            fallback_to_deterministic: true,
        }
    }
}

impl AiProviderConfig {
    /// Normaliza límites, modelo por defecto y espacios sobrantes sin tocar credenciales.
    pub fn normalized(mut self) -> Self {
        self.max_connections = self.max_connections.clamp(1, 25);
        self.model = self.model.trim().to_string();
        if self.model.is_empty() {
            self.model = self.provider.default_model().to_string();
        }
        self.endpoint_url = self
            .endpoint_url
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self.cli_binary_override = self
            .cli_binary_override
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self
    }
}

/// Estado actual de los conectores de IA y detección de CLI/credenciales.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderStatus {
    pub active_config: AiProviderConfig,
    pub claude_cli_available: bool,
    pub claude_cli_path: Option<String>,
    pub gemini_cli_available: bool,
    pub gemini_cli_path: Option<String>,
    pub has_gemini_api_key: bool,
    pub has_compatible_api_key: bool,
    pub platform: String,
}

/// Modo de ejecución de auto-conexión PKM (`preview` vs `apply`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoConnectMode {
    Preview,
    Apply,
}

impl Default for AutoConnectMode {
    fn default() -> Self {
        Self::Preview
    }
}

/// Parámetros de solicitud para auto-conectar una nota, lista de notas o carpeta (`Inbox/`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutoConnectRequest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub mode: AutoConnectMode,
    #[serde(default)]
    pub max_connections: Option<usize>,
    #[serde(default)]
    pub append_related_section: Option<bool>,
    #[serde(default)]
    pub custom_plan: Option<AiEnrichmentPlan>,
    #[serde(default)]
    pub provider_override: Option<AiProviderKind>,
}

/// Resultado individual de auto-conexión para una nota procesada.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoConnectNoteOutcome {
    pub path: String,
    pub plan: AiEnrichmentPlan,
    pub preview: AiEnrichmentPreview,
    pub applied: bool,
    pub provider_used: AiProviderKind,
    pub warning: Option<String>,
}

/// Mensaje individual dentro del historial de conversación con el Copiloto PKM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatMessage {
    pub role: String,
    pub content: String,
}

/// Solicitud de conversación libre con contexto compacto de la bóveda.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatRequest {
    pub prompt: String,
    #[serde(default)]
    pub active_note_path: Option<String>,
    #[serde(default)]
    pub include_vault_map: Option<bool>,
    #[serde(default)]
    pub provider_override: Option<AiProviderKind>,
    #[serde(default)]
    pub history: Option<Vec<AiChatMessage>>,
}

/// Respuesta del Copiloto PKM con referencias de la bóveda y avisos de fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatResponse {
    pub reply_markdown: String,
    pub provider_used: AiProviderKind,
    pub model_used: String,
    pub referenced_notes: Vec<String>,
    pub suggested_actions: Vec<String>,
    pub warning: Option<String>,
}

/// Contexto compacto pasado a un proveedor IA para proponer un `AiEnrichmentPlan`.
pub struct EnrichmentPromptContext<'a> {
    pub vault_map: &'a VaultKnowledgeMap,
    pub note_path: &'a str,
    pub note_title: &'a str,
    pub note_content: &'a str,
    pub candidates: &'a [CandidateConnection],
    pub deterministic_plan: &'a AiEnrichmentPlan,
    pub max_connections: usize,
    pub append_related_section: bool,
}

/// Nota de la bóveda cuyo contenido se inyecta directamente en el contexto del Chat PKM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatContextNote {
    pub path: String,
    pub title: String,
    pub folder: String,
    pub tags: Vec<String>,
    pub content: String,
    pub match_reason: String,
}

/// Contexto compacto pasado a un proveedor IA para responder preguntas sobre la bóveda.
pub struct ChatPromptContext<'a> {
    pub prompt: &'a str,
    pub vault_root: Option<&'a Path>,
    pub vault_map: Option<&'a VaultKnowledgeMap>,
    pub active_note_path: Option<&'a str>,
    pub active_note_content: Option<&'a str>,
    pub active_candidates: &'a [CandidateConnection],
    pub context_notes: &'a [ChatContextNote],
    pub search_hits: &'a [SearchResult],
    pub history: &'a [AiChatMessage],
}

/// Contrato común de todos los conectores de IA (`[RF-16]`).
pub trait AiProvider {
    fn kind(&self) -> AiProviderKind;
    fn generate_enrichment_plan(
        &self,
        ctx: &EnrichmentPromptContext<'_>,
    ) -> Result<AiEnrichmentPlan, String>;
    fn chat(&self, ctx: &ChatPromptContext<'_>) -> Result<String, String>;
}

/// Carga `.romsidian/ai_config.json` devolviendo configuración por defecto si aún no existe.
pub fn load_ai_config(vault_root: &Path) -> Result<AiProviderConfig, String> {
    let cfg_path = vault_root.join(VAULT_METADATA_DIR).join(AI_CONFIG_FILE);
    if !cfg_path.exists() {
        return Ok(AiProviderConfig::default());
    }
    let raw = fs::read_to_string(&cfg_path)
        .map_err(|e| format!("No se pudo leer configuración IA: {e}"))?;
    let parsed: AiProviderConfig = serde_json::from_str(&raw)
        .map_err(|e| format!("Configuración IA inválida en .romsidian/{AI_CONFIG_FILE}: {e}"))?;
    Ok(parsed.normalized())
}

/// Guarda ajustes no sensibles de IA en `.romsidian/ai_config.json`.
pub fn save_ai_config(vault_root: &Path, config: &AiProviderConfig) -> Result<AiProviderConfig, String> {
    let clean = config.clone().normalized();
    let meta_dir = vault_root.join(VAULT_METADATA_DIR);
    fs::create_dir_all(&meta_dir)
        .map_err(|e| format!("No se pudo crear carpeta {VAULT_METADATA_DIR}: {e}"))?;
    let cfg_path = meta_dir.join(AI_CONFIG_FILE);
    let json = serde_json::to_string_pretty(&clean)
        .map_err(|e| format!("Error al serializar configuración IA: {e}"))?;
    fs::write(&cfg_path, format!("{json}\n"))
        .map_err(|e| format!("No se pudo guardar .romsidian/{AI_CONFIG_FILE}: {e}"))?;
    Ok(clean)
}

/// Actualiza o elimina claves API en el llavero seguro del SO sin tocar archivos de la bóveda.
pub fn update_ai_credentials(
    store: &dyn CredentialStore,
    gemini_api_key: Option<&str>,
    compatible_api_key: Option<&str>,
    clear_gemini: bool,
    clear_compatible: bool,
) -> Result<(), String> {
    if clear_gemini {
        store
            .delete_token(GEMINI_API_KEY_ACCOUNT)
            .map_err(|e| format!("No se pudo limpiar la clave de Gemini en el llavero: {e}"))?;
    } else if let Some(key) = gemini_api_key.map(str::trim).filter(|s| !s.is_empty()) {
        store
            .set_token(GEMINI_API_KEY_ACCOUNT, key)
            .map_err(|e| format!("No se pudo guardar la clave de Gemini en el llavero: {e}"))?;
    }

    if clear_compatible {
        store
            .delete_token(COMPATIBLE_API_KEY_ACCOUNT)
            .map_err(|e| format!("No se pudo limpiar la clave compatible en el llavero: {e}"))?;
    } else if let Some(key) = compatible_api_key.map(str::trim).filter(|s| !s.is_empty()) {
        store
            .set_token(COMPATIBLE_API_KEY_ACCOUNT, key)
            .map_err(|e| format!("No se pudo guardar la clave compatible en el llavero: {e}"))?;
    }
    Ok(())
}

/// Construye el diagnóstico de proveedores disponibles en la plataforma actual.
pub fn build_ai_provider_status(
    vault_root: &Path,
    store: &dyn CredentialStore,
) -> Result<AiProviderStatus, String> {
    let active_config = load_ai_config(vault_root)?;
    let override_str = active_config.cli_binary_override.as_deref();

    let claude_override = (active_config.provider == AiProviderKind::ClaudeCli)
        .then_some(override_str)
        .flatten();
    let gemini_override = (active_config.provider == AiProviderKind::GeminiCli)
        .then_some(override_str)
        .flatten();

    let claude_path = detect_cli_binary("claude", claude_override);
    let gemini_path = detect_cli_binary("gemini", gemini_override);

    let has_gemini_api_key = store
        .get_token(GEMINI_API_KEY_ACCOUNT)
        .ok()
        .flatten()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_compatible_api_key = store
        .get_token(COMPATIBLE_API_KEY_ACCOUNT)
        .ok()
        .flatten()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    Ok(AiProviderStatus {
        active_config,
        claude_cli_available: claude_path.is_some(),
        claude_cli_path: claude_path.map(path_to_forward_string),
        gemini_cli_available: gemini_path.is_some(),
        gemini_cli_path: gemini_path.map(path_to_forward_string),
        has_gemini_api_key,
        has_compatible_api_key,
        platform: current_platform_name().to_string(),
    })
}

/// Valida y normaliza una ruta de carpeta relativa dentro de la bóveda para filtrado en lote.
pub fn normalize_safe_folder_filter(raw: &str) -> Result<String, String> {
    let normalized = raw.trim().replace('\\', "/");
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let mut parts = Vec::new();
    for comp in Path::new(trimmed).components() {
        match comp {
            Component::Normal(os_str) => {
                let s = os_str.to_string_lossy();
                if s.starts_with('.') {
                    return Err(format!("Carpeta oculta o protegida no permitida: '{raw}'"));
                }
                parts.push(s.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "Ruta de carpeta inválida o fuera de la bóveda: '{raw}'"
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

/// Ejecuta el proveedor configurado (o `override_kind`) con fallback automático opcional.
pub fn run_enrichment_with_provider(
    config: &AiProviderConfig,
    override_kind: Option<AiProviderKind>,
    vault_root: Option<&Path>,
    store: &dyn CredentialStore,
    ctx: &EnrichmentPromptContext<'_>,
) -> Result<(AiEnrichmentPlan, AiProviderKind, Option<String>), String> {
    let chosen = override_kind.unwrap_or(config.provider);
    let provider = build_provider_instance(chosen, config, vault_root, store)?;

    match provider.generate_enrichment_plan(ctx) {
        Ok(plan) => Ok((plan, chosen, None)),
        Err(err) => {
            if config.fallback_to_deterministic && chosen != AiProviderKind::Deterministic {
                Ok((
                    ctx.deterministic_plan.clone(),
                    AiProviderKind::Deterministic,
                    Some(format!("Fallback determinista activado ({err})")),
                ))
            } else {
                Err(err)
            }
        }
    }
}

/// Ejecuta una consulta conversacional al Copiloto PKM con fallback opcional offline.
pub fn run_chat_with_provider(
    config: &AiProviderConfig,
    override_kind: Option<AiProviderKind>,
    vault_root: Option<&Path>,
    store: &dyn CredentialStore,
    ctx: &ChatPromptContext<'_>,
) -> Result<(String, AiProviderKind, String, Option<String>), String> {
    let chosen = override_kind.unwrap_or(config.provider);
    let effective_root = vault_root.or(ctx.vault_root);
    let provider = build_provider_instance(chosen, config, effective_root, store)?;
    let model_name = if chosen == config.provider {
        config.model.clone()
    } else {
        chosen.default_model().to_string()
    };

    match provider.chat(ctx) {
        Ok(reply) => Ok((reply, chosen, model_name, None)),
        Err(err) => {
            if config.fallback_to_deterministic && chosen != AiProviderKind::Deterministic {
                let offline = DeterministicRustProvider.chat(ctx)?;
                Ok((
                    offline,
                    AiProviderKind::Deterministic,
                    AiProviderKind::Deterministic.default_model().to_string(),
                    Some(format!("Respuesta generada en modo offline determinista ({err})")),
                ))
            } else {
                Err(err)
            }
        }
    }
}

fn build_provider_instance<'a>(
    kind: AiProviderKind,
    config: &AiProviderConfig,
    vault_root: Option<&Path>,
    store: &'a dyn CredentialStore,
) -> Result<Box<dyn AiProvider + 'a>, String> {
    let model = if kind == config.provider && !config.model.trim().is_empty() {
        config.model.clone()
    } else {
        kind.default_model().to_string()
    };
    let cli_vault_root = vault_root.map(Path::to_path_buf);

    match kind {
        AiProviderKind::Deterministic => Ok(Box::new(DeterministicRustProvider)),
        AiProviderKind::ClaudeCli => Ok(Box::new(LocalCliProvider {
            kind: AiProviderKind::ClaudeCli,
            binary_name: "claude",
            binary_override: config.cli_binary_override.clone(),
            model,
            vault_root: cli_vault_root,
        })),
        AiProviderKind::GeminiCli => Ok(Box::new(LocalCliProvider {
            kind: AiProviderKind::GeminiCli,
            binary_name: "gemini",
            binary_override: config.cli_binary_override.clone(),
            model,
            vault_root: cli_vault_root,
        })),
        AiProviderKind::GeminiHttp => Ok(Box::new(GeminiHttpProvider {
            model,
            endpoint_override: config.endpoint_url.clone(),
            store,
        })),
        AiProviderKind::CompatibleHttp => Ok(Box::new(CompatibleHttpProvider {
            model,
            endpoint_url: config.endpoint_url.clone(),
            store,
        })),
    }
}

// ============================================================================
// Proveedor 1: DeterministicRustProvider (Offline / 0 tokens / <50 ms)
// ============================================================================

/// Proveedor local sin red ni tokens que delega en el motor determinista de `romsidian-core`.
pub struct DeterministicRustProvider;

impl AiProvider for DeterministicRustProvider {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::Deterministic
    }

    fn generate_enrichment_plan(
        &self,
        ctx: &EnrichmentPromptContext<'_>,
    ) -> Result<AiEnrichmentPlan, String> {
        Ok(ctx.deterministic_plan.clone())
    }

    fn chat(&self, ctx: &ChatPromptContext<'_>) -> Result<String, String> {
        let mut out = String::new();
        out.push_str("### Copiloto PKM Offline (Motor Rust Local)\n\n");

        if let Some(map) = ctx.vault_map {
            out.push_str(&format!(
                "- **Resumen de la bóveda**: {} notas, {} enlaces internos, {} etiquetas, {} huérfanas.\n",
                map.total_notes,
                map.total_links,
                map.total_tags,
                map.orphan_paths.len()
            ));
            if !map.tag_taxonomy.is_empty() {
                let top_tags: Vec<String> = map
                    .tag_taxonomy
                    .iter()
                    .take(8)
                    .map(|t| format!("`#{}` ({})", t.tag, t.count))
                    .collect();
                out.push_str(&format!("- **Etiquetas principales**: {}\n", top_tags.join(", ")));
            }
            if !map.orphan_paths.is_empty() {
                let orphans: Vec<String> = map
                    .orphan_paths
                    .iter()
                    .take(5)
                    .map(|p| format!("`{p}`"))
                    .collect();
                out.push_str(&format!(
                    "- **Notas sin enlaces entrantes**: {}\n",
                    orphans.join(", ")
                ));
            }
        }

        if let Some(active) = ctx.active_note_path {
            out.push_str(&format!("\n#### Nota activa: `{active}`\n"));
            if ctx.active_candidates.is_empty() {
                out.push_str("No se detectaron nuevas conexiones candidatas pendientes para esta nota.\n");
            } else {
                out.push_str("Conexiones sugeridas por menciones, `#tags` y similitud FTS5:\n");
                for cand in ctx.active_candidates.iter().take(6) {
                    let status = if cand.already_linked {
                        "ya enlazada"
                    } else {
                        "candidata nueva"
                    };
                    out.push_str(&format!(
                        "- {} (`{}`) — score `{:.0}` ({status}): {}\n",
                        cand.suggested_wikilink, cand.target_path, cand.score, cand.reason_summary
                    ));
                }
            }
        }

        if !ctx.context_notes.is_empty() {
            out.push_str(&format!(
                "\n#### Notas de la bóveda analizadas para esta consulta ({})\n",
                ctx.context_notes.len()
            ));
            for note in ctx.context_notes {
                let excerpt = truncate_chars(note.content.trim(), 240).replace('\n', " ");
                let tags_str = if note.tags.is_empty() {
                    String::new()
                } else {
                    let formatted: Vec<String> =
                        note.tags.iter().map(|t| format!("#{t}")).collect();
                    format!(" ({})", formatted.join(" "))
                };
                out.push_str(&format!(
                    "- **[[{}]]** (`{}`){tags_str} — *{}*: {}\n",
                    note.title, note.path, note.match_reason, excerpt
                ));
            }
        }

        if !ctx.search_hits.is_empty() {
            out.push_str("\n#### Notas relevantes encontradas (FTS5)\n");
            for hit in ctx.search_hits.iter().take(6) {
                let snippet = hit.snippet.replace('\n', " ");
                out.push_str(&format!("- **[[{}]]** (`{}`): {}\n", hit.title, hit.path, snippet));
            }
        } else if !ctx.prompt.trim().is_empty() {
            out.push_str(&format!(
                "\nNo hubo coincidencias léxicas directas en FTS5 para: *\"{}\"*.\n",
                ctx.prompt.trim()
            ));
        }

        out.push_str(
            "\n> **Sugerencia**: Usa **Auto-Conectar** para enriquecer `frontmatter`, `#tags` y `[[wikilinks]]` en Vista Previa, o configura **Claude CLI**, **Gemini CLI** o **Gemini HTTP** en Ajustes para análisis semántico avanzado.",
        );
        Ok(out)
    }
}

// ============================================================================
// Proveedor 2: LocalCliProvider (`claude -p` / `gemini -p` por stdio)
// ============================================================================

/// Proveedor local que invoca `claude` o `gemini` CLI autenticado con la suscripción del usuario.
pub struct LocalCliProvider {
    pub kind: AiProviderKind,
    pub binary_name: &'static str,
    pub binary_override: Option<String>,
    pub model: String,
    pub vault_root: Option<PathBuf>,
}

impl AiProvider for LocalCliProvider {
    fn kind(&self) -> AiProviderKind {
        self.kind
    }

    fn generate_enrichment_plan(
        &self,
        ctx: &EnrichmentPromptContext<'_>,
    ) -> Result<AiEnrichmentPlan, String> {
        let prompt = build_enrichment_prompt(ctx);
        let raw_reply = self.run_cli_prompt(&prompt, false)?;
        parse_and_merge_llm_plan(&raw_reply, ctx)
    }

    fn chat(&self, ctx: &ChatPromptContext<'_>) -> Result<String, String> {
        let prompt = build_chat_prompt(ctx);
        self.run_cli_prompt(&prompt, true)
    }
}

#[cfg(any(not(target_os = "android"), test))]
fn build_cli_args(
    kind: AiProviderKind,
    model: &str,
    vault_root: Option<&Path>,
    allow_read_tools: bool,
) -> Vec<String> {
    let model_trimmed = model.trim();
    let has_model =
        !model_trimmed.is_empty() && model_trimmed != AiProviderKind::Deterministic.default_model();

    match kind {
        AiProviderKind::ClaudeCli => {
            let mut v = vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "text".to_string(),
                "--no-session-persistence".to_string(),
                "--disable-slash-commands".to_string(),
                "--strict-mcp-config".to_string(),
                "--system-prompt".to_string(),
                "Eres el Copiloto PKM de ROMSIDIAN. Responde en español con precisión.".to_string(),
            ];
            if has_model {
                v.push("--model".to_string());
                v.push(model_trimmed.to_string());
            }
            if allow_read_tools {
                v.push("--tools".to_string());
                v.push("Read,Glob,Grep,LS".to_string());
                v.push("--allowedTools".to_string());
                v.push("Read,Glob,Grep,LS".to_string());
                if let Some(root) = vault_root.filter(|p| p.is_dir()) {
                    v.push("--add-dir".to_string());
                    v.push(root.to_string_lossy().to_string());
                }
            } else {
                v.push("--tools".to_string());
                v.push(String::new());
            }
            v
        }
        AiProviderKind::GeminiCli => {
            let mut v = vec!["-p".to_string()];
            if has_model {
                v.push("-m".to_string());
                v.push(model_trimmed.to_string());
            }
            v
        }
        _ => vec!["-p".to_string()],
    }
}

impl LocalCliProvider {
    fn run_cli_prompt(&self, full_prompt: &str, allow_read_tools: bool) -> Result<String, String> {
        #[cfg(target_os = "android")]
        {
            let _ = (full_prompt, allow_read_tools);
            return Err(format!(
                "El proveedor CLI local '{}' solo está disponible en escritorio (Windows/Linux/macOS).",
                self.binary_name
            ));
        }

        #[cfg(not(target_os = "android"))]
        {
            let binary_path = detect_cli_binary(
                self.binary_name,
                self.binary_override.as_deref(),
            )
            .ok_or_else(|| {
                format!(
                    "No se encontró el binario '{}' en PATH ni rutas estándar. Instálalo o configura su ruta en Ajustes.",
                    self.binary_name
                )
            })?;

            let args = build_cli_args(
                self.kind,
                &self.model,
                self.vault_root.as_deref(),
                allow_read_tools,
            );

            let mut cmd = build_cli_command(&binary_path, &args);
            if let Some(root) = self.vault_root.as_deref().filter(|p| p.is_dir()) {
                cmd.current_dir(root);
            }
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            #[cfg(target_os = "windows")]
            cmd.creation_flags(CREATE_NO_WINDOW);

            let mut child = cmd.spawn().map_err(|e| {
                format!(
                    "Error al ejecutar '{}' ({}): {e}",
                    self.binary_name,
                    binary_path.display()
                )
            })?;

            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(full_prompt.as_bytes())
                    .map_err(|e| format!("Error al enviar prompt por stdin a '{}': {e}", self.binary_name))?;
            }

            let output = child
                .wait_with_output()
                .map_err(|e| format!("Error esperando respuesta de '{}': {e}", self.binary_name))?;

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if !output.status.success() {
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                return Err(format!(
                    "El CLI '{}' finalizó con código {:?}: {}",
                    self.binary_name,
                    output.status.code(),
                    truncate_chars(&detail, 280)
                ));
            }

            if stdout.is_empty() {
                return Err(format!(
                    "El CLI '{}' devolvió una respuesta vacía.",
                    self.binary_name
                ));
            }
            Ok(stdout)
        }
    }
}

#[cfg(not(target_os = "android"))]
fn build_cli_command(binary_path: &Path, args: &[String]) -> Command {
    #[cfg(target_os = "windows")]
    {
        let ext = binary_path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("cmd") | Some("bat")) {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(binary_path).args(args);
            return cmd;
        }
    }
    let mut cmd = Command::new(binary_path);
    cmd.args(args);
    cmd
}

// ============================================================================
// Proveedor 3: GeminiHttpProvider (`ureq` + `rustls` Windows y Android)
// ============================================================================

/// Proveedor HTTPS compatible con Google AI Studio (Gemini Free Tier) en PC y Android.
pub struct GeminiHttpProvider<'a> {
    pub model: String,
    pub endpoint_override: Option<String>,
    pub store: &'a dyn CredentialStore,
}

impl AiProvider for GeminiHttpProvider<'_> {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::GeminiHttp
    }

    fn generate_enrichment_plan(
        &self,
        ctx: &EnrichmentPromptContext<'_>,
    ) -> Result<AiEnrichmentPlan, String> {
        let prompt = build_enrichment_prompt(ctx);
        let raw_reply = self.call_generate_content(&prompt, true)?;
        parse_and_merge_llm_plan(&raw_reply, ctx)
    }

    fn chat(&self, ctx: &ChatPromptContext<'_>) -> Result<String, String> {
        let prompt = build_chat_prompt(ctx);
        self.call_generate_content(&prompt, false)
    }
}

impl GeminiHttpProvider<'_> {
    fn call_generate_content(&self, prompt: &str, json_mode: bool) -> Result<String, String> {
        let api_key = self
            .store
            .get_token(GEMINI_API_KEY_ACCOUNT)
            .map_err(|e| format!("Error leyendo clave Gemini del llavero seguro: {e}"))?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "No hay clave API de Gemini configurada en el llavero seguro del SO.".to_string()
            })?;

        let model = if self.model.trim().is_empty() {
            AiProviderKind::GeminiHttp.default_model()
        } else {
            self.model.trim()
        };

        let url = match &self.endpoint_override {
            Some(custom) if !custom.trim().is_empty() => custom.trim().to_string(),
            _ => format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
            ),
        };

        let mut gen_cfg = json!({ "temperature": if json_mode { 0.2 } else { 0.4 } });
        if json_mode {
            gen_cfg["responseMimeType"] = json!("application/json");
        }

        let body = json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{ "text": prompt }]
                }
            ],
            "generationConfig": gen_cfg
        });

        let agent = build_http_agent();
        let resp = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .set("x-goog-api-key", &api_key)
            .send_json(&body);

        let payload = handle_ureq_json_response(resp, "Gemini API")?;
        extract_gemini_text(&payload)
    }
}

// ============================================================================
// Proveedor 4: CompatibleHttpProvider (OpenAI / Ollama / OpenRouter)
// ============================================================================

/// Proveedor HTTPS/local para endpoints compatibles con `/v1/chat/completions`.
pub struct CompatibleHttpProvider<'a> {
    pub model: String,
    pub endpoint_url: Option<String>,
    pub store: &'a dyn CredentialStore,
}

impl AiProvider for CompatibleHttpProvider<'_> {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::CompatibleHttp
    }

    fn generate_enrichment_plan(
        &self,
        ctx: &EnrichmentPromptContext<'_>,
    ) -> Result<AiEnrichmentPlan, String> {
        let prompt = build_enrichment_prompt(ctx);
        let raw_reply = self.call_chat_completions(&prompt, true)?;
        parse_and_merge_llm_plan(&raw_reply, ctx)
    }

    fn chat(&self, ctx: &ChatPromptContext<'_>) -> Result<String, String> {
        let prompt = build_chat_prompt(ctx);
        self.call_chat_completions(&prompt, false)
    }
}

impl CompatibleHttpProvider<'_> {
    fn call_chat_completions(&self, prompt: &str, json_mode: bool) -> Result<String, String> {
        let url = resolve_compatible_endpoint(self.endpoint_url.as_deref());
        let is_local = url.contains("localhost") || url.contains("127.0.0.1");

        let api_key = self
            .store
            .get_token(COMPATIBLE_API_KEY_ACCOUNT)
            .map_err(|e| format!("Error leyendo clave compatible del llavero seguro: {e}"))?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if api_key.is_none() && !is_local {
            return Err(
                "No hay clave API configurada en el llavero seguro para el endpoint compatible."
                    .to_string(),
            );
        }

        let model = if self.model.trim().is_empty() {
            AiProviderKind::CompatibleHttp.default_model()
        } else {
            self.model.trim()
        };

        let body = json!({
            "model": model,
            "temperature": if json_mode { 0.2 } else { 0.4 },
            "messages": [
                {
                    "role": "system",
                    "content": "Eres el Copiloto PKM experto de ROMSIDIAN. Responde en español con precisión."
                },
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        });

        let agent = build_http_agent();
        let mut req = agent.post(&url).set("Content-Type", "application/json");
        if let Some(token) = api_key.as_deref() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }

        let resp = req.send_json(&body);
        let payload = handle_ureq_json_response(resp, "Endpoint compatible")?;
        extract_compatible_text(&payload)
    }
}

// ============================================================================
// Prompts Compactos y Validación Segura de Respuestas LLM
// ============================================================================

fn build_enrichment_prompt(ctx: &EnrichmentPromptContext<'_>) -> String {
    let map_summary = ctx.vault_map.to_compact_prompt_context(120);
    let candidates_text = if ctx.candidates.is_empty() {
        "Ninguna detectada localmente.".to_string()
    } else {
        ctx.candidates
            .iter()
            .take(ctx.max_connections.saturating_mul(2))
            .map(|c| {
                format!(
                    "- [[{}]] ({}) | score:{:.0} | linked:{} | {}",
                    c.target_stem, c.target_path, c.score, c.already_linked, c.reason_summary
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let truncated_body = truncate_chars(ctx.note_content, 4500);
    let append_flag = ctx.append_related_section;

    format!(
        r#"Eres el motor PKM de ROMSIDIAN. Analiza la nota objetivo y devuelve EXCLUSIVAMENTE un objeto JSON válido (sin texto adicional) que proponga metadatos frontmatter coherentes, etiquetas #tags de la taxonomía existente y conexiones [[wikilinks]] reales hacia otras notas de la bóveda.

REGLAS ESTRICTAS:
1. Solo usa destinos existentes en el Mapa de la Bóveda. Nunca inventes rutas ni enlaces rotos.
2. Respeta claves existentes del frontmatter del usuario.
3. No modifiques URLs ni bloques de código.
4. Usa exactamente este esquema JSON:
{{
  "frontmatter_updates": {{"tema": "valor", "fuentes": ["..."]}},
  "suggested_tags": ["tag1", "area/subtag"],
  "inline_wikilinks": [
    {{"target_path": "Nota.md", "matched_term": "texto exacto en nota", "replacement_wikilink": "[[Nota|texto exacto en nota]]"}}
  ],
  "related_connections": [
    {{"target_path": "Nota.md", "target_title": "Título de Nota", "relation": "Por qué se relaciona"}}
  ],
  "append_related_section": {append_flag}
}}

--- MAPA COMPACTO DE LA BÓVEDA ---
{map_summary}

--- CANDIDATAS PRECALCULADAS (FTS5 + MENCIONES) ---
{candidates_text}

--- NOTA OBJETIVO: {path} (Título: {title}) ---
{truncated_body}"#,
        path = ctx.note_path,
        title = ctx.note_title,
    )
}

fn build_chat_prompt(ctx: &ChatPromptContext<'_>) -> String {
    let mut sections = Vec::new();
    sections.push(
        "Eres el Copiloto PKM de ROMSIDIAN. Ayudas a analizar fuentes, sintetizar notas, redactar documentos completos, conectar ideas con [[wikilinks]], proponer #tags y organizar frontmatter YAML. Responde en español en Markdown claro y bien estructurado.\n\
REGLAS DE ACCESO A LA BÓVEDA (SOLO LECTURA):\n\
- Ya tienes acceso autorizado y directo para leer la bóveda abierta del usuario y el contenido de las notas inyectadas más abajo.\n\
- Nunca pidas permiso para leer notas ni preguntes al usuario dónde está ubicada la bóveda.\n\
- Si necesitas consultar archivos adicionales de la bóveda con tus herramientas de lectura, tu directorio de trabajo actual ya es la raíz de la bóveda.\n\
REGLAS DE ENTREGA Y ESCRITURA (CRÍTICO):\n\
- NO tienes herramientas para crear, escribir ni modificar archivos en disco. NUNCA intentes escribir archivos, NUNCA digas que ya guardaste o creaste una nota en la bóveda, NUNCA digas que el sistema pedirá confirmación para escribir, y NUNCA preguntes \"¿quieres que lo haga?\" o \"¿quieres que lo guarde en...?\" antes de entregar el contenido.\n\
- Cuando el usuario pida un resumen, síntesis, manual paso a paso, guía, blueprint, plan, plantilla o nueva nota, redacta y entrega el CONTENIDO COMPLETO Y DESARROLLADO de principio a fin directamente en tu respuesta (PROHIBIDO devolver solo un índice, esquema o lista de secciones prometidas).\n\
- Si el resultado está pensado para guardarse como nota en la bóveda, comienza directamente con un título `# Título claro` (y opcionalmente `#tags` y `[[wikilinks]]` a notas existentes), sin preámbulos innecesarios: el usuario puede guardarlo en 1 clic con el botón \"Guardar como nota\" debajo de tu respuesta.\n\
- Cuando el usuario pida un resumen o análisis de varias notas o de una carpeta completa, elabora una síntesis única e integrada que cubra TODAS las notas incluidas y explique las relaciones conceptuales entre ellas."
            .to_string(),
    );

    if let Some(map) = ctx.vault_map {
        let root_line = ctx
            .vault_root
            .map(|p| {
                format!(
                    "Ruta de la bóveda abierta: {}\n",
                    path_to_forward_string(p.to_path_buf())
                )
            })
            .unwrap_or_default();
        sections.push(format!(
            "--- MAPA COMPACTO DE LA BÓVEDA ---\n{}{}",
            root_line,
            map.to_compact_prompt_context(100)
        ));
    } else if let Some(root) = ctx.vault_root {
        sections.push(format!(
            "--- RUTA DE LA BÓVEDA ABIERTA ---\n{}",
            path_to_forward_string(root.to_path_buf())
        ));
    }

    if let Some(active_path) = ctx.active_note_path {
        let mut active_sec = format!("--- NOTA ACTIVA: {active_path} ---\n");
        if let Some(content) = ctx.active_note_content {
            active_sec.push_str(&truncate_chars(content, 2500));
            active_sec.push('\n');
        }
        if !ctx.active_candidates.is_empty() {
            active_sec.push_str("Candidatas locales:\n");
            for c in ctx.active_candidates.iter().take(6) {
                active_sec.push_str(&format!(
                    "- {} ({}) score={:.0} ({})\n",
                    c.suggested_wikilink, c.target_path, c.score, c.reason_summary
                ));
            }
        }
        sections.push(active_sec);
    }

    if !ctx.context_notes.is_empty() {
        let mut notes_sec = format!(
            "--- CONTENIDO DE NOTAS DE LA BÓVEDA RELEVANTES PARA LA CONSULTA ({} notas) ---\n",
            ctx.context_notes.len()
        );
        for note in ctx.context_notes {
            let folder_display = if note.folder.is_empty() { "/" } else { &note.folder };
            let tags_display = if note.tags.is_empty() {
                "ninguno".to_string()
            } else {
                note.tags
                    .iter()
                    .map(|t| format!("#{t}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            notes_sec.push_str(&format!(
                "### Nota: [[{}]] (`{}`) | carpeta: `{}` | tags: {} | motivo: {}\n{}\n\n",
                note.title,
                note.path,
                folder_display,
                tags_display,
                note.match_reason,
                truncate_chars(&note.content, 3500)
            ));
        }
        sections.push(notes_sec.trim_end().to_string());
    }

    if !ctx.search_hits.is_empty() {
        let mut hits_sec = String::from("--- FRAGMENTOS RELEVANTES POR BÚSQUEDA FTS5 ---\n");
        for hit in ctx.search_hits.iter().take(6) {
            hits_sec.push_str(&format!(
                "- [[{}]] ({}): {}\n",
                hit.title,
                hit.path,
                truncate_chars(&hit.snippet, 220)
            ));
        }
        sections.push(hits_sec);
    }

    if !ctx.history.is_empty() {
        let mut hist_sec = String::from("--- HISTORIAL RECIENTE ---\n");
        for msg in ctx.history.iter().rev().take(6).collect::<Vec<_>>().into_iter().rev() {
            hist_sec.push_str(&format!("{}: {}\n", msg.role, truncate_chars(&msg.content, 500)));
        }
        sections.push(hist_sec);
    }

    sections.push(format!("--- CONSULTA DEL USUARIO ---\n{}", ctx.prompt.trim()));
    sections.join("\n\n")
}

#[derive(Debug, Default, Deserialize)]
struct RawLlmEnrichmentPlan {
    #[serde(default)]
    frontmatter_updates: BTreeMap<String, Value>,
    #[serde(default)]
    overwrite_existing_frontmatter: Option<bool>,
    #[serde(default)]
    suggested_tags: Vec<String>,
    #[serde(default)]
    inline_wikilinks: Vec<RawWikilinkItem>,
    #[serde(default)]
    related_connections: Vec<RawRelatedItem>,
    #[serde(default)]
    append_related_section: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWikilinkItem {
    #[serde(default)]
    target_path: String,
    #[serde(default)]
    target_stem: Option<String>,
    #[serde(default)]
    matched_term: String,
    #[serde(default)]
    replacement_wikilink: Option<String>,
    #[serde(default)]
    line: Option<usize>,
    #[serde(default)]
    context: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRelatedItem {
    #[serde(default)]
    target_path: String,
    #[serde(default)]
    target_title: Option<String>,
    #[serde(default)]
    relation: Option<String>,
}

/// Parsea el JSON devuelto por un LLM (tolerando fences Markdown o campos parciales),
/// valida destinos contra el mapa real de la bóveda y fusiona señales deterministas.
pub fn parse_and_merge_llm_plan(
    raw_output: &str,
    ctx: &EnrichmentPromptContext<'_>,
) -> Result<AiEnrichmentPlan, String> {
    let json_slice = extract_json_object(raw_output).ok_or_else(|| {
        "La respuesta del modelo no incluyó un objeto JSON válido de AiEnrichmentPlan.".to_string()
    })?;
    let raw: RawLlmEnrichmentPlan = serde_json::from_str(json_slice)
        .map_err(|e| format!("JSON de enriquecimiento inválido: {e}"))?;

    let lookup = build_note_lookup(&ctx.vault_map.notes);
    let note_lines: Vec<&str> = ctx.note_content.lines().collect();

    let mut frontmatter_updates = ctx.deterministic_plan.frontmatter_updates.clone();
    for (k, v) in raw.frontmatter_updates {
        let key = k.trim();
        if key.is_empty() || key.eq_ignore_ascii_case("tags") {
            continue;
        }
        if let Some(fm_val) = json_value_to_frontmatter(&v) {
            frontmatter_updates.insert(key.to_string(), fm_val);
        }
    }

    let mut tag_set = HashSet::new();
    let mut suggested_tags = Vec::new();
    for tag in raw
        .suggested_tags
        .into_iter()
        .chain(ctx.deterministic_plan.suggested_tags.clone())
    {
        let clean = tag.trim().trim_start_matches('#').to_lowercase();
        if !clean.is_empty() && tag_set.insert(clean.clone()) {
            suggested_tags.push(clean);
        }
    }

    let mut inline_wikilinks = ctx.deterministic_plan.inline_wikilinks.clone();
    let mut seen_inline: HashSet<(usize, String)> = inline_wikilinks
        .iter()
        .map(|w| (w.line, w.matched_term.to_lowercase()))
        .collect();

    for item in raw.inline_wikilinks {
        let term = item.matched_term.trim();
        if term.is_empty() {
            continue;
        }
        let target_key = if !item.target_path.trim().is_empty() {
            item.target_path.trim()
        } else {
            item.target_stem.as_deref().unwrap_or(term).trim()
        };
        let Some(entry) = resolve_note_entry(&lookup, target_key) else {
            continue;
        };
        if entry.path.eq_ignore_ascii_case(ctx.note_path) {
            continue;
        }

        let Some((line_no, line_ctx)) = resolve_term_line(&note_lines, term, item.line) else {
            continue;
        };
        if !seen_inline.insert((line_no, term.to_lowercase())) {
            continue;
        }
        let replacement_wikilink = item
            .replacement_wikilink
            .as_deref()
            .map(str::trim)
            .filter(|s| s.starts_with("[[") && s.ends_with("]]"))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                if term.eq_ignore_ascii_case(&entry.stem) {
                    format!("[[{}]]", entry.stem)
                } else {
                    format!("[[{}|{}]]", entry.stem, term)
                }
            });

        inline_wikilinks.push(WikilinkReplacement {
            target_path: entry.path.clone(),
            target_stem: entry.stem.clone(),
            matched_term: term.to_string(),
            replacement_wikilink,
            line: line_no,
            context: item.context.unwrap_or(line_ctx),
        });
    }

    let mut related_connections = Vec::new();
    let mut seen_related = HashSet::new();
    for rel in raw.related_connections {
        let Some(entry) = resolve_note_entry(&lookup, rel.target_path.trim()) else {
            continue;
        };
        if entry.path.eq_ignore_ascii_case(ctx.note_path) {
            continue;
        }
        if seen_related.insert(entry.path.to_lowercase()) {
            related_connections.push(RelatedConnectionLink {
                target_path: entry.path.clone(),
                target_title: rel
                    .target_title
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&entry.title)
                    .to_string(),
                relation: rel.relation.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            });
        }
    }
    for rel in &ctx.deterministic_plan.related_connections {
        if related_connections.len() >= ctx.max_connections {
            break;
        }
        if seen_related.insert(rel.target_path.to_lowercase()) {
            related_connections.push(rel.clone());
        }
    }

    Ok(AiEnrichmentPlan {
        frontmatter_updates,
        overwrite_existing_frontmatter: raw.overwrite_existing_frontmatter.unwrap_or(false),
        suggested_tags,
        inline_wikilinks,
        related_connections,
        append_related_section: raw
            .append_related_section
            .unwrap_or(ctx.append_related_section),
    })
}

/// Extrae el primer objeto JSON balanceado (`{ ... }`) respetando strings e ignora fences Markdown.
pub fn extract_json_object(raw: &str) -> Option<&str> {
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    let mut start_idx: Option<usize> = None;

    for (idx, ch) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' if start_idx.is_some() => in_string = true,
            '{' => {
                if depth == 0 {
                    start_idx = Some(idx);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = start_idx {
                        return Some(&raw[start..=idx]);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn json_value_to_frontmatter(val: &Value) -> Option<FrontmatterValue> {
    match val {
        Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| FrontmatterValue::Scalar(trimmed.to_string()))
        }
        Value::Number(n) => Some(FrontmatterValue::Scalar(n.to_string())),
        Value::Bool(b) => Some(FrontmatterValue::Scalar(b.to_string())),
        Value::Array(arr) => {
            let items: Vec<String> = arr
                .iter()
                .filter_map(|item| match item {
                    Value::String(s) => (!s.trim().is_empty()).then(|| s.trim().to_string()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
                .collect();
            (!items.is_empty()).then_some(FrontmatterValue::List(items))
        }
        Value::Null | Value::Object(_) => None,
    }
}

fn build_note_lookup(notes: &[KnowledgeNoteEntry]) -> HashMap<String, &KnowledgeNoteEntry> {
    let mut map = HashMap::new();
    for entry in notes {
        map.entry(entry.path.to_lowercase()).or_insert(entry);
        map.entry(entry.stem.to_lowercase()).or_insert(entry);
        map.entry(entry.title.to_lowercase()).or_insert(entry);
        for alias in &entry.aliases {
            map.entry(alias.to_lowercase()).or_insert(entry);
        }
    }
    map
}

fn resolve_note_entry<'a>(
    lookup: &HashMap<String, &'a KnowledgeNoteEntry>,
    raw_key: &str,
) -> Option<&'a KnowledgeNoteEntry> {
    let clean = raw_key
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .split('|')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if clean.is_empty() {
        return None;
    }
    lookup
        .get(&clean)
        .copied()
        .or_else(|| {
            clean
                .strip_suffix(".md")
                .and_then(|stem| lookup.get(stem).copied())
        })
}

fn resolve_term_line(
    note_lines: &[&str],
    term: &str,
    hint_line: Option<usize>,
) -> Option<(usize, String)> {
    let term_lower = term.to_lowercase();
    if let Some(line_no) = hint_line.filter(|&n| n >= 1 && n <= note_lines.len()) {
        let line_text = note_lines[line_no - 1];
        if line_text.to_lowercase().contains(&term_lower) {
            return Some((line_no, line_text.trim().to_string()));
        }
    }
    for (idx, line_text) in note_lines.iter().enumerate() {
        if line_text.to_lowercase().contains(&term_lower) {
            return Some((idx + 1, line_text.trim().to_string()));
        }
    }
    None
}

// ============================================================================
// Helpers HTTP (`ureq 2.10` + `rustls`) y Descubrimiento de CLI Local
// ============================================================================

fn build_http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .build()
}

fn handle_ureq_json_response(
    resp: Result<ureq::Response, ureq::Error>,
    label: &str,
) -> Result<Value, String> {
    match resp {
        Ok(r) => r
            .into_json::<Value>()
            .map_err(|e| format!("Respuesta JSON inválida de {label}: {e}")),
        Err(ureq::Error::Status(code, r)) => {
            let msg = r.into_string().unwrap_or_default();
            Err(format!("{label} HTTP {code}: {}", truncate_chars(&msg, 260)))
        }
        Err(e) => Err(format!("Error de conexión con {label}: {e}")),
    }
}

/// Extrae el texto generado de la estructura estándar `candidates[0].content.parts[*].text` de Gemini.
pub fn extract_gemini_text(payload: &Value) -> Result<String, String> {
    let parts = payload
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array);

    if let Some(parts_arr) = parts {
        let joined: String = parts_arr
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        let trimmed = joined.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err("La respuesta de Gemini API no contiene texto en candidates[0].".to_string())
}

/// Extrae el contenido de `choices[0].message.content` de un endpoint compatible OpenAI.
pub fn extract_compatible_text(payload: &Value) -> Result<String, String> {
    let text = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    text.map(ToOwned::to_owned).ok_or_else(|| {
        "La respuesta del endpoint compatible no contiene texto en choices[0].message.content."
            .to_string()
    })
}

fn resolve_compatible_endpoint(raw: Option<&str>) -> String {
    let base = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("https://api.openai.com/v1/chat/completions");
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

/// Busca ejecutables CLI locales (`claude`, `gemini`) en `override`, `PATH` y carpetas estándar.
pub fn detect_cli_binary(binary_name: &str, override_path: Option<&str>) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        let _ = (binary_name, override_path);
        None
    }

    #[cfg(not(target_os = "android"))]
    {
        if let Some(custom) = override_path.map(str::trim).filter(|s| !s.is_empty()) {
            let p = PathBuf::from(custom);
            if p.is_file() {
                return Some(p);
            }
            if let Some(found) = find_in_path_dirs(custom) {
                return Some(found);
            }
        }

        if let Some(found) = find_in_path_dirs(binary_name) {
            return Some(found);
        }

        #[cfg(target_os = "windows")]
        {
            let mut extra_dirs = Vec::new();
            if let Some(appdata) = env::var_os("APPDATA") {
                extra_dirs.push(PathBuf::from(appdata).join("npm"));
            }
            if let Some(local_appdata) = env::var_os("LOCALAPPDATA") {
                extra_dirs.push(PathBuf::from(local_appdata).join("Programs"));
            }
            if let Some(profile) = env::var_os("USERPROFILE") {
                extra_dirs.push(PathBuf::from(&profile).join(".local").join("bin"));
                extra_dirs.push(PathBuf::from(&profile).join(".claude").join("local"));
            }
            for dir in extra_dirs {
                if let Some(found) = probe_binary_in_dir(&dir, binary_name) {
                    return Some(found);
                }
            }
        }

        None
    }
}

#[cfg(not(target_os = "android"))]
fn find_in_path_dirs(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        if let Some(found) = probe_binary_in_dir(&dir, name) {
            return Some(found);
        }
    }
    None
}

#[cfg(not(target_os = "android"))]
fn probe_binary_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    #[cfg(target_os = "windows")]
    {
        for ext in ["exe", "cmd", "bat"] {
            let candidate = dir.join(format!("{name}.{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn current_platform_name() -> &'static str {
    #[cfg(target_os = "android")]
    {
        "android"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(all(not(target_os = "android"), not(target_os = "windows")))]
    {
        env::consts::OS
    }
}

fn path_to_forward_string(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max_chars).collect();
    format!("{prefix}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use romsidian_core::TagCount;

    fn sample_context<'a>(
        vault_map: &'a VaultKnowledgeMap,
        det_plan: &'a AiEnrichmentPlan,
        note_content: &'a str,
    ) -> EnrichmentPromptContext<'a> {
        EnrichmentPromptContext {
            vault_map,
            note_path: "Inbox/Video.md",
            note_title: "Video",
            note_content,
            candidates: &[],
            deterministic_plan: det_plan,
            max_connections: 8,
            append_related_section: true,
        }
    }

    #[test]
    fn extract_json_object_handles_fences_and_string_braces() {
        let raw = r#"
Aquí tienes el plan sugerido:
```json
{
  "frontmatter_updates": {"resumen": "Contiene {llaves} dentro"},
  "suggested_tags": ["macro"]
}
```
Fin."#;
        let json_str = extract_json_object(raw).expect("Debe extraer el JSON");
        let parsed: Value = serde_json::from_str(json_str).expect("JSON válido");
        assert_eq!(parsed["suggested_tags"][0], "macro");
        assert_eq!(
            parsed["frontmatter_updates"]["resumen"],
            "Contiene {llaves} dentro"
        );
    }

    #[test]
    fn parse_and_merge_llm_plan_validates_targets_and_resolves_lines() {
        let vault_map = VaultKnowledgeMap {
            total_notes: 2,
            total_links: 0,
            total_tags: 1,
            orphan_paths: vec![],
            folders: vec![],
            tag_taxonomy: vec![TagCount {
                tag: "economia".to_string(),
                count: 1,
            }],
            frontmatter_schema: vec![],
            notes: vec![
                KnowledgeNoteEntry {
                    path: "Inbox/Video.md".to_string(),
                    stem: "Video".to_string(),
                    title: "Video".to_string(),
                    folder: "Inbox".to_string(),
                    aliases: vec![],
                    tags: vec![],
                    headings: vec![],
                    frontmatter_keys: vec![],
                    outlink_count: 0,
                    inlink_count: 0,
                    is_orphan: true,
                },
                KnowledgeNoteEntry {
                    path: "Politica Monetaria.md".to_string(),
                    stem: "Politica Monetaria".to_string(),
                    title: "Política Monetaria".to_string(),
                    folder: "".to_string(),
                    aliases: vec![],
                    tags: vec!["economia".to_string()],
                    headings: vec![],
                    frontmatter_keys: vec![],
                    outlink_count: 0,
                    inlink_count: 0,
                    is_orphan: true,
                },
            ],
        };
        let note_content = "# Video\n\nAnalizamos política monetaria y tasas.\n";
        let det_plan = AiEnrichmentPlan::default();
        let ctx = sample_context(&vault_map, &det_plan, note_content);

        let llm_reply = r##"```json
{
  "frontmatter_updates": {"revisado": true, "puntaje": 9, "tema": "Macro"},
  "suggested_tags": ["#Economia", "YouTube"],
  "inline_wikilinks": [
    {"target_path": "Politica Monetaria.md", "matched_term": "política monetaria"},
    {"target_path": "NoExiste.md", "matched_term": "fantasma"}
  ],
  "related_connections": [
    {"target_path": "Politica Monetaria.md", "relation": "Base teórica"}
  ],
  "append_related_section": true
}
```"##;

        let plan = parse_and_merge_llm_plan(llm_reply, &ctx).expect("Parseo debe tener éxito");
        assert_eq!(
            plan.frontmatter_updates.get("revisado"),
            Some(&FrontmatterValue::Scalar("true".to_string()))
        );
        assert_eq!(
            plan.frontmatter_updates.get("tema"),
            Some(&FrontmatterValue::Scalar("Macro".to_string()))
        );
        assert!(plan.suggested_tags.contains(&"economia".to_string()));
        assert!(plan.suggested_tags.contains(&"youtube".to_string()));
        assert_eq!(plan.inline_wikilinks.len(), 1);
        assert_eq!(plan.inline_wikilinks[0].line, 3);
        assert_eq!(
            plan.inline_wikilinks[0].replacement_wikilink,
            "[[Politica Monetaria|política monetaria]]"
        );
        assert_eq!(plan.related_connections.len(), 1);
    }

    #[test]
    fn normalize_safe_folder_filter_rejects_traversal_and_hidden() {
        assert_eq!(normalize_safe_folder_filter(" Inbox\\ ").unwrap(), "Inbox");
        assert_eq!(normalize_safe_folder_filter("").unwrap(), "");
        assert!(normalize_safe_folder_filter("../secret").is_err());
        assert!(normalize_safe_folder_filter(".romsidian").is_err());
        assert!(normalize_safe_folder_filter("C:/Users").is_err());
    }

    #[test]
    fn http_extractors_parse_gemini_and_compatible_payloads() {
        let gemini_json = json!({
            "candidates": [{
                "content": { "parts": [{ "text": "{\"suggested_tags\": [\"ia\"]}" }] }
            }]
        });
        assert_eq!(
            extract_gemini_text(&gemini_json).unwrap(),
            "{\"suggested_tags\": [\"ia\"]}"
        );

        let compat_json = json!({
            "choices": [{
                "message": { "content": "Respuesta PKM" }
            }]
        });
        assert_eq!(extract_compatible_text(&compat_json).unwrap(), "Respuesta PKM");
    }

    #[test]
    fn claude_cli_args_and_chat_prompt_include_vault_access_and_context_notes() {
        let temp_dir = env::temp_dir();
        let chat_args = build_cli_args(
            AiProviderKind::ClaudeCli,
            "claude-sonnet-4-5",
            Some(&temp_dir),
            true,
        );
        assert!(chat_args.contains(&"-p".to_string()));
        assert!(chat_args.contains(&"--no-session-persistence".to_string()));
        assert!(chat_args.contains(&"--disable-slash-commands".to_string()));
        assert!(chat_args.contains(&"--strict-mcp-config".to_string()));
        assert!(chat_args.contains(&"--system-prompt".to_string()));
        assert!(chat_args.contains(&"--tools".to_string()));
        assert!(chat_args.contains(&"--allowedTools".to_string()));
        assert!(chat_args.contains(&"Read,Glob,Grep,LS".to_string()));
        assert!(chat_args.contains(&"--add-dir".to_string()));
        assert!(chat_args.contains(&temp_dir.to_string_lossy().to_string()));

        let enrich_args = build_cli_args(
            AiProviderKind::ClaudeCli,
            "claude-sonnet-4-5",
            Some(&temp_dir),
            false,
        );
        assert!(enrich_args.contains(&"--tools".to_string()));
        assert!(enrich_args.contains(&String::new()));
        assert!(!enrich_args.contains(&"--allowedTools".to_string()));

        let context_notes = vec![ChatContextNote {
            path: "inbox/youtube 2.md".to_string(),
            title: "Gemini 4 Hydrogen".to_string(),
            folder: "inbox".to_string(),
            tags: vec!["gemini".to_string()],
            content: "Contenido completo de youtube 2".to_string(),
            match_reason: "Carpeta 'inbox'".to_string(),
        }];
        let ctx = ChatPromptContext {
            prompt: "Resumen de /inbox",
            vault_root: Some(&temp_dir),
            vault_map: None,
            active_note_path: Some("inbox/youtube.md"),
            active_note_content: Some("Contenido de youtube 1"),
            active_candidates: &[],
            context_notes: &context_notes,
            search_hits: &[],
            history: &[],
        };

        let prompt_str = build_chat_prompt(&ctx);
        assert!(prompt_str.contains("Nunca pidas permiso para leer notas"));
        assert!(prompt_str.contains("NO tienes herramientas para crear, escribir ni modificar archivos en disco"));
        assert!(prompt_str.contains("CONTENIDO COMPLETO Y DESARROLLADO"));
        assert!(prompt_str.contains("inbox/youtube 2.md"));
        assert!(prompt_str.contains("Contenido completo de youtube 2"));

        let offline = DeterministicRustProvider.chat(&ctx).unwrap();
        assert!(offline.contains("inbox/youtube 2.md"));
    }
}
