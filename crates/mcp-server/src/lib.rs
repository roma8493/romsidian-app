//! Servidor Model Context Protocol (MCP) nativo para ROMSIDIAN (`[RF-18]`).
//!
//! Expone las capacidades de `romsidian-core` (mapa compacto de conocimiento, búsqueda FTS5,
//! consultas Dataview DQL, lectura/escritura segura, sugerencia y auto-conexión de notas y
//! generación de lienzos `.canvas`) mediante JSON-RPC 2.0 sobre `stdio` sin dependencias pesadas.

use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use romsidian_core::{
    apply_ai_enrichment, build_deterministic_enrichment_plan, build_vault_knowledge_map,
    execute_dataview_query, generate_knowledge_canvas, parse_note, preview_ai_enrichment,
    suggest_note_connections, AiEnrichmentPlan, AiEnrichmentPreview, CoreResult,
    KnowledgeCanvasRequest, Vault, VaultIndex,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Versión del protocolo MCP anunciada por defecto si el cliente no solicita otra.
pub const DEFAULT_MCP_PROTOCOL_VERSION: &str = "2024-11-05";

const JSONRPC_PARSE_ERROR: i64 = -32700;
const JSONRPC_INVALID_REQUEST: i64 = -32600;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;

/// Resuelve la ruta raíz de la bóveda a partir de argumentos CLI (sin ejecutable),
/// la variable de entorno `ROMSIDIAN_VAULT` o el directorio actual (`test-vault` si existe).
pub fn resolve_vault_path(args: &[String], env_vault: Option<&str>, cwd: &Path) -> PathBuf {
    let cli_candidate = extract_cli_vault_arg(args);
    let raw = cli_candidate
        .or_else(|| {
            env_vault
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        })
        .map(PathBuf::from);

    match raw {
        Some(path) if path.is_absolute() => path,
        Some(path) => cwd.join(path),
        None => {
            let default_test_vault = cwd.join("test-vault");
            if default_test_vault.is_dir() {
                default_test_vault
            } else {
                cwd.to_path_buf()
            }
        }
    }
}

fn extract_cli_vault_arg(args: &[String]) -> Option<String> {
    let mut positional: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].trim();
        if arg == "--vault" || arg == "-v" {
            if let Some(next) = args.get(i + 1) {
                let next_trimmed = next.trim();
                if !next_trimmed.is_empty() {
                    return Some(next_trimmed.to_string());
                }
            }
            i += 2;
            continue;
        }
        if let Some(val) = arg.strip_prefix("--vault=") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        } else if !arg.is_empty() && !arg.starts_with('-') && positional.is_none() {
            positional = Some(arg.to_string());
        }
        i += 1;
    }
    positional
}

/// Formato de tramado detectado en la entrada `stdio`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireFraming {
    JsonLine,
    ContentLength,
}

/// Resumen compacto de vista previa/aplicación por nota para ahorrar contexto en clientes MCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutoConnectNoteOutcome {
    relative_path: String,
    changed: bool,
    applied: bool,
    added_frontmatter_keys: Vec<String>,
    added_tags: Vec<String>,
    injected_wikilinks_count: usize,
    injected_wikilinks: Vec<String>,
    appended_related_links: Vec<String>,
    unified_diff_lines: Vec<String>,
}

impl AutoConnectNoteOutcome {
    fn from_preview(preview: AiEnrichmentPreview, applied: bool) -> Self {
        let injected_wikilinks: Vec<String> = preview
            .injected_wikilinks
            .iter()
            .map(|w| format!("L{}: '{}' -> {}", w.line, w.matched_term, w.replacement_wikilink))
            .collect();
        let injected_wikilinks_count = injected_wikilinks.len();
        Self {
            relative_path: preview.relative_path,
            changed: preview.changed,
            applied,
            added_frontmatter_keys: preview.added_frontmatter_keys,
            added_tags: preview.added_tags,
            injected_wikilinks_count,
            injected_wikilinks,
            appended_related_links: preview.appended_related_links,
            unified_diff_lines: preview.unified_diff_lines,
        }
    }
}

/// Instancia del servidor MCP enlazada a una bóveda y su índice SQLite FTS5.
pub struct McpServer {
    vault: Vault,
    index: VaultIndex,
}

impl McpServer {
    /// Abre la bóveda en `vault_root` e inicializa/sincroniza `.romsidian/index.db`.
    pub fn open(vault_root: impl AsRef<Path>) -> CoreResult<Self> {
        let vault = Vault::open(vault_root)?;
        let (index, _) = VaultIndex::open_and_sync(&vault)?;
        Ok(Self { vault, index })
    }

    /// Devuelve la ruta raíz de la bóveda atendida por este servidor.
    pub fn vault_root(&self) -> &Path {
        self.vault.root_path()
    }

    /// Procesa un flujo JSON-RPC 2.0 sobre `BufRead`/`Write` (soportando líneas JSON y `Content-Length`).
    pub fn process_stdio_stream<R: BufRead, W: Write>(
        &mut self,
        mut reader: R,
        mut writer: W,
    ) -> io::Result<()> {
        while let Some((raw_json, framing)) = read_next_message(&mut reader)? {
            let reply = match serde_json::from_str::<Value>(&raw_json) {
                Ok(msg) => self.handle_jsonrpc_value(&msg),
                Err(err) => Some(jsonrpc_error(
                    Value::Null,
                    JSONRPC_PARSE_ERROR,
                    &format!("JSON inválido: {err}"),
                )),
            };

            if let Some(resp) = reply {
                write_framed_message(&mut writer, &resp, framing)?;
            }
        }
        Ok(())
    }

    /// Despacha un mensaje JSON-RPC 2.0 individual siguiendo el estándar MCP.
    ///
    /// Devuelve `None` cuando el mensaje es una notificación (sin `id` o método `notifications/*`).
    pub fn handle_jsonrpc_value(&mut self, msg: &Value) -> Option<Value> {
        let obj = match msg.as_object() {
            Some(map) => map,
            None => {
                return Some(jsonrpc_error(
                    Value::Null,
                    JSONRPC_INVALID_REQUEST,
                    "El mensaje JSON-RPC debe ser un objeto JSON",
                ));
            }
        };

        let method = obj.get("method").and_then(Value::as_str);
        let id = obj.get("id").cloned();

        let Some(method_name) = method else {
            return id.map(|req_id| {
                jsonrpc_error(
                    req_id,
                    JSONRPC_INVALID_REQUEST,
                    "Falta el campo string obligatorio 'method'",
                )
            });
        };

        if id.is_none() || method_name.starts_with("notifications/") {
            return None;
        }
        let req_id = id.unwrap_or(Value::Null);
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));

        let response = match method_name {
            "initialize" => jsonrpc_ok(req_id, build_initialize_result(&params)),
            "ping" => jsonrpc_ok(req_id, json!({})),
            "resources/list" => jsonrpc_ok(req_id, json!({ "resources": [] })),
            "prompts/list" => jsonrpc_ok(req_id, json!({ "prompts": [] })),
            "tools/list" => jsonrpc_ok(req_id, json!({ "tools": mcp_tool_definitions() })),
            "tools/call" => self.handle_tools_call(req_id, &params),
            other => jsonrpc_error(
                req_id,
                JSONRPC_METHOD_NOT_FOUND,
                &format!("Método MCP/JSON-RPC no soportado: {other}"),
            ),
        };
        Some(response)
    }

    fn handle_tools_call(&mut self, req_id: Value, params: &Value) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return jsonrpc_error(
                req_id,
                JSONRPC_INVALID_PARAMS,
                "Falta el parámetro obligatorio 'name' en tools/call",
            );
        };
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let outcome = match name {
            "vault_knowledge_map" => self.tool_vault_knowledge_map(&args),
            "search_notes_fts" => self.tool_search_notes_fts(&args),
            "run_dataview_dql" => self.tool_run_dataview_dql(&args),
            "read_note" => self.tool_read_note(&args),
            "write_note" => self.tool_write_note(&args),
            "suggest_note_connections" => self.tool_suggest_note_connections(&args),
            "auto_connect_notes" => self.tool_auto_connect_notes(&args),
            "create_knowledge_canvas" => self.tool_create_knowledge_canvas(&args),
            unknown => {
                return jsonrpc_error(
                    req_id,
                    JSONRPC_INVALID_PARAMS,
                    &format!("Herramienta MCP desconocida: {unknown}"),
                );
            }
        };

        let result = match outcome {
            Ok(data) => mcp_tool_ok(data),
            Err(err_msg) => mcp_tool_error(&err_msg),
        };
        jsonrpc_ok(req_id, result)
    }

    fn tool_vault_knowledge_map(&mut self, args: &Value) -> Result<Value, String> {
        let max_notes = args
            .get("max_notes")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(200);
        let include_compact_text = args
            .get("include_compact_text")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        self.index
            .sync_vault(&self.vault)
            .map_err(|e| format!("No se pudo sincronizar el índice de la bóveda: {e}"))?;
        let map = build_vault_knowledge_map(&self.index)
            .map_err(|e| format!("No se pudo construir el mapa de conocimiento: {e}"))?;

        if include_compact_text {
            let compact = map.to_compact_prompt_context(max_notes);
            Ok(json!({
                "compact_prompt_context": compact,
                "map": map,
            }))
        } else {
            Ok(json!({ "map": map }))
        }
    }

    fn tool_search_notes_fts(&mut self, args: &Value) -> Result<Value, String> {
        let query = required_nonempty_str(args, "query")?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, 100))
            .unwrap_or(20);

        self.index
            .sync_vault(&self.vault)
            .map_err(|e| format!("No se pudo sincronizar el índice SQLite: {e}"))?;
        let results = self
            .index
            .search(query, limit)
            .map_err(|e| format!("Error ejecutando búsqueda FTS5: {e}"))?;

        Ok(json!({
            "query": query,
            "count": results.len(),
            "results": results,
        }))
    }

    fn tool_run_dataview_dql(&mut self, args: &Value) -> Result<Value, String> {
        let query = required_nonempty_str(args, "query")?;
        self.index
            .sync_vault(&self.vault)
            .map_err(|e| format!("No se pudo sincronizar el índice antes de DQL: {e}"))?;
        let result = execute_dataview_query(&self.index, Some(&self.vault), query)
            .map_err(|e| format!("Error en consulta Dataview DQL: {e}"))?;

        Ok(json!({
            "query": query,
            "result": result,
        }))
    }

    fn tool_read_note(&mut self, args: &Value) -> Result<Value, String> {
        let path = required_nonempty_str(args, "path")?;
        let doc = self
            .vault
            .read_note(path)
            .map_err(|e| format!("No se pudo leer la nota '{path}': {e}"))?;
        let parsed = parse_note(&doc.content);
        let backlinks = self
            .index
            .backlinks_for(&doc.relative_path)
            .unwrap_or_default();

        Ok(json!({
            "note": doc,
            "frontmatter": parsed.frontmatter,
            "tags": parsed.tags,
            "headings": parsed.headings,
            "wikilinks": parsed.wikilinks,
            "backlinks": backlinks,
        }))
    }

    fn tool_write_note(&mut self, args: &Value) -> Result<Value, String> {
        let path = required_nonempty_str(args, "path")?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| "Falta el parámetro string obligatorio 'content'".to_string())?;

        let saved = self
            .vault
            .save_note(path, content)
            .map_err(|e| format!("No se pudo guardar la nota '{path}': {e}"))?;
        let reindexed = self
            .index
            .index_note(&self.vault, &saved.relative_path)
            .map_err(|e| format!("Nota guardada pero falló reindexación SQLite: {e}"))?;

        Ok(json!({
            "saved": true,
            "reindexed": reindexed,
            "note": saved,
        }))
    }

    fn tool_suggest_note_connections(&mut self, args: &Value) -> Result<Value, String> {
        let path = required_nonempty_str(args, "path")?;
        let max_candidates = args
            .get("max_candidates")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, 50))
            .unwrap_or(10);

        self.index
            .sync_vault(&self.vault)
            .map_err(|e| format!("No se pudo sincronizar el índice: {e}"))?;
        let candidates = suggest_note_connections(&self.index, path, max_candidates)
            .map_err(|e| format!("No se pudieron sugerir conexiones para '{path}': {e}"))?;

        Ok(json!({
            "path": path,
            "count": candidates.len(),
            "candidates": candidates,
        }))
    }

    fn tool_auto_connect_notes(&mut self, args: &Value) -> Result<Value, String> {
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("preview");
        if mode != "preview" && mode != "apply" {
            return Err(format!(
                "Valor inválido para 'mode': '{mode}' (use 'preview' o 'apply')"
            ));
        }

        let max_connections = args
            .get("max_connections")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, 25))
            .unwrap_or(8);
        let append_related_section = args
            .get("append_related_section")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let custom_plan: Option<AiEnrichmentPlan> = match args.get("plan") {
            None | Some(Value::Null) => None,
            Some(val) => Some(
                serde_json::from_value(val.clone())
                    .map_err(|e| format!("El campo 'plan' no es un AiEnrichmentPlan válido: {e}"))?,
            ),
        };

        let targets = self.collect_auto_connect_targets(args)?;
        self.index
            .sync_vault(&self.vault)
            .map_err(|e| format!("No se pudo sincronizar el índice antes de auto-conectar: {e}"))?;

        let mut items = Vec::with_capacity(targets.len());
        let mut changed_count = 0usize;

        for rel_path in targets {
            let plan = match &custom_plan {
                Some(p) => p.clone(),
                None => build_deterministic_enrichment_plan(
                    &self.vault,
                    &self.index,
                    &rel_path,
                    max_connections,
                    append_related_section,
                )
                .map_err(|e| format!("No se pudo generar el plan para '{rel_path}': {e}"))?,
            };

            let outcome = if mode == "apply" {
                let res = apply_ai_enrichment(&self.vault, &mut self.index, &rel_path, &plan)
                    .map_err(|e| format!("No se pudo aplicar enriquecimiento en '{rel_path}': {e}"))?;
                AutoConnectNoteOutcome::from_preview(res.preview, true)
            } else {
                let doc = self
                    .vault
                    .read_note(&rel_path)
                    .map_err(|e| format!("No se pudo leer '{rel_path}': {e}"))?;
                let prev = preview_ai_enrichment(&doc.relative_path, &doc.content, &plan);
                AutoConnectNoteOutcome::from_preview(prev, false)
            };

            if outcome.changed {
                changed_count += 1;
            }
            items.push(outcome);
        }

        Ok(json!({
            "mode": mode,
            "total_processed": items.len(),
            "changed_count": changed_count,
            "items": items,
        }))
    }

    fn collect_auto_connect_targets(&self, args: &Value) -> Result<Vec<String>, String> {
        let mut targets = Vec::new();
        let mut seen = HashSet::new();

        if let Some(single) = args.get("path").and_then(Value::as_str) {
            let trimmed = single.trim();
            if !trimmed.is_empty() && seen.insert(trimmed.to_lowercase()) {
                targets.push(trimmed.to_string());
            }
        }

        if let Some(arr) = args.get("paths").and_then(Value::as_array) {
            for item in arr {
                if let Some(raw_path) = item.as_str() {
                    let trimmed = raw_path.trim();
                    if !trimmed.is_empty() && seen.insert(trimmed.to_lowercase()) {
                        targets.push(trimmed.to_string());
                    }
                }
            }
        }

        if let Some(raw_folder) = args.get("folder").and_then(Value::as_str) {
            let clean_folder = normalize_safe_folder_filter(raw_folder)?;
            let notes = self
                .vault
                .list_notes()
                .map_err(|e| format!("No se pudo listar las notas de la bóveda: {e}"))?;
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
                "No se encontraron notas objetivo: especifica 'path', 'paths' o una 'folder' válida."
                    .to_string(),
            );
        }
        Ok(targets)
    }

    fn tool_create_knowledge_canvas(&mut self, args: &Value) -> Result<Value, String> {
        let canvas_path = required_nonempty_str(args, "canvas_path")?;
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let seed_note_paths = args
            .get("seed_note_paths")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let include_neighbors = args
            .get("include_neighbors")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let max_nodes = args
            .get("max_nodes")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, 40))
            .unwrap_or(12);

        self.index
            .sync_vault(&self.vault)
            .map_err(|e| format!("No se pudo sincronizar el índice antes de crear el lienzo: {e}"))?;

        let req = KnowledgeCanvasRequest {
            canvas_path: canvas_path.to_string(),
            title,
            seed_note_paths,
            include_neighbors,
            max_nodes,
        };
        let created = generate_knowledge_canvas(&self.vault, &self.index, &req)
            .map_err(|e| format!("No se pudo crear el lienzo '.canvas': {e}"))?;

        Ok(json!({
            "canvas_path": created.canvas.relative_path,
            "title": created.canvas.title,
            "node_count": created.node_count,
            "edge_count": created.edge_count,
        }))
    }
}

fn required_nonempty_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Falta el parámetro string obligatorio '{key}'"))
}

fn normalize_safe_folder_filter(raw_folder: &str) -> Result<String, String> {
    let normalized = raw_folder
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    if normalized.is_empty() {
        return Ok(String::new());
    }
    let mut clean_segments = Vec::new();
    for component in Path::new(&normalized).components() {
        match component {
            std::path::Component::Normal(os_seg) => {
                let seg = os_seg.to_str().unwrap_or_default();
                if seg.is_empty() || seg.starts_with('.') || seg.contains(':') {
                    return Err(format!(
                        "Ruta de carpeta inválida o insegura en 'folder': '{raw_folder}'"
                    ));
                }
                clean_segments.push(seg.to_string());
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "Ruta de carpeta inválida o insegura en 'folder': '{raw_folder}'"
                ));
            }
        }
    }
    Ok(clean_segments.join("/"))
}

fn build_initialize_result(params: &Value) -> Value {
    let protocol_version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MCP_PROTOCOL_VERSION);

    json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "romsidian-mcp",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Servidor MCP nativo para bóvedas Markdown/Canvas de ROMSIDIAN. Usa `vault_knowledge_map` para mapear notas, tags y claves de frontmatter con consumo mínimo de tokens; `search_notes_fts` y `run_dataview_dql` para consultar; `suggest_note_connections` y `auto_connect_notes` (preview/apply) para tejer [[wikilinks]], #tags y frontmatter de forma idempotente; y `create_knowledge_canvas` para generar mapas visuales .canvas."
    })
}

/// Catálogo de las 8 herramientas MCP expuestas por `romsidian-mcp` (`[RF-18]`).
pub fn mcp_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "vault_knowledge_map",
            "description": "Obtiene el mapa compacto de conocimiento de toda la bóveda desde SQLite (notas, aliases, taxonomía de #tags, claves frontmatter, encabezados y notas huérfanas) sin leer todos los archivos completos.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_notes": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Máximo de notas en `compact_prompt_context` (0 = todas, por defecto 200)."
                    },
                    "include_compact_text": {
                        "type": "boolean",
                        "description": "Si es true (por defecto), incluye resumen compacto tipo llms.txt además del JSON."
                    }
                }
            }
        }),
        json!({
            "name": "search_notes_fts",
            "description": "Búsqueda de texto completo BM25 en SQLite FTS5 sobre títulos, cuerpos y etiquetas (soporta tildes/eñes y filtros `#etiqueta`).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Consulta FTS5 y/o filtros `#etiqueta` (ej. 'inflación #macro')."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Cantidad máxima de resultados (por defecto 20)."
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "run_dataview_dql",
            "description": "Ejecuta una consulta segura Dataview Query Language (TABLE, LIST o TASK con FROM, WHERE, SORT, LIMIT) sobre el índice SQLite de la bóveda.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Consulta DQL completa (ej. 'TABLE estado FROM #proyecto SORT file.mtime DESC')."
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "read_note",
            "description": "Lee el contenido UTF-8 de una nota `.md`, sus propiedades YAML frontmatter, etiquetas, encabezados, enlaces salientes y backlinks entrantes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Ruta relativa segura dentro de la bóveda (ej. 'Inbox/video.md' o 'Macro')."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "write_note",
            "description": "Crea o sobrescribe una nota `.md` validando ruta segura dentro de la bóveda y reindexa SQLite FTS5 al instante.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Ruta relativa `.md` dentro de la bóveda (rechaza traversal o carpetas ocultas)."
                    },
                    "content": {
                        "type": "string",
                        "description": "Contenido Markdown completo en UTF-8."
                    }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "suggest_note_connections",
            "description": "Analiza una nota y propone conexiones candidatas ordenadas por puntaje combinando menciones no enlazadas (título/aliases), #tags compartidos, campos y similitud BM25 en FTS5.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Ruta relativa de la nota a analizar."
                    },
                    "max_candidates": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Máximo de candidatos a devolver (por defecto 10)."
                    }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "auto_connect_notes",
            "description": "Previsualiza ('preview') o aplica ('apply') de forma idempotente mejoras de YAML frontmatter, #tags e inyección de [[wikilinks]] en una nota, lista de notas o carpeta completa (ej. 'Inbox') protegiendo bloques de código y URLs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Ruta relativa de una nota específica."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Lista explícita de rutas relativas a procesar."
                    },
                    "folder": {
                        "type": "string",
                        "description": "Carpeta relativa a procesar en lote (ej. 'Inbox', o '' para toda la bóveda)."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["preview", "apply"],
                        "description": "'preview' calcula diff sin escribir (por defecto); 'apply' guarda en disco y reindexa SQLite."
                    },
                    "max_connections": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 25,
                        "description": "Máximo de conexiones candidatas por nota cuando se usa plan automático (por defecto 8)."
                    },
                    "append_related_section": {
                        "type": "boolean",
                        "description": "Si es true, agrega sección '## Conexiones Relacionadas' cuando haya enlaces fuertes sin mención literal en el texto."
                    },
                    "plan": {
                        "type": "object",
                        "description": "AiEnrichmentPlan explícito propuesto por el agente (frontmatter_updates, suggested_tags, inline_wikilinks, related_connections, append_related_section)."
                    }
                }
            }
        }),
        json!({
            "name": "create_knowledge_canvas",
            "description": "Genera o actualiza un lienzo JSON Canvas 1.0 (`.canvas`) conectando notas semilla, vecinas sugeridas y aristas existentes/sugeridas.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "canvas_path": {
                        "type": "string",
                        "description": "Ruta relativa segura terminada en `.canvas` (ej. 'Mapas/Fuentes.canvas')."
                    },
                    "title": {
                        "type": "string",
                        "description": "Título del nodo cabecera del lienzo."
                    },
                    "seed_note_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Notas iniciales a incluir (vacío = principales de la bóveda)."
                    },
                    "include_neighbors": {
                        "type": "boolean",
                        "description": "Si es true (por defecto), expande con notas conectadas/candidatas."
                    },
                    "max_nodes": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 40,
                        "description": "Máximo de nodos de nota en el lienzo (por defecto 12)."
                    }
                },
                "required": ["canvas_path"]
            }
        }),
    ]
}

fn jsonrpc_ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

fn mcp_tool_ok(data: Value) -> Value {
    let text = serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": data,
        "isError": false,
    })
}

fn mcp_tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": format!("Error: {message}") }],
        "isError": true,
    })
}

fn read_next_message<R: BufRead>(reader: &mut R) -> io::Result<Option<(String, WireFraming)>> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(None);
        }

        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() {
            continue;
        }

        if let Some(len) = parse_content_length_header(trimmed) {
            // Consumir cabeceras restantes hasta la línea en blanco que separa el cuerpo.
            loop {
                line.clear();
                let hdr_bytes = reader.read_line(&mut line)?;
                if hdr_bytes == 0 {
                    return Ok(None);
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body)?;
            let text = String::from_utf8(body).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("UTF-8 inválido en MCP: {e}"))
            })?;
            return Ok(Some((text, WireFraming::ContentLength)));
        }

        return Ok(Some((trimmed.to_string(), WireFraming::JsonLine)));
    }
}

fn parse_content_length_header(line: &str) -> Option<usize> {
    let (key, val) = line.split_once(':')?;
    if key.trim().eq_ignore_ascii_case("content-length") {
        val.trim().parse::<usize>().ok()
    } else {
        None
    }
}

fn write_framed_message<W: Write>(
    writer: &mut W,
    message: &Value,
    framing: WireFraming,
) -> io::Result<()> {
    let serialized = serde_json::to_string(message)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    match framing {
        WireFraming::JsonLine => {
            writeln!(writer, "{serialized}")?;
        }
        WireFraming::ContentLength => {
            write!(
                writer,
                "Content-Length: {}\r\n\r\n{}",
                serialized.len(),
                serialized
            )?;
        }
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "romsidian_mcp_{prefix}_{}_{}",
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&dir).expect("crear carpeta temporal de pruebas MCP");
            Self { path: dir }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn seed_test_notes(root: &Path) {
        fs::create_dir_all(root.join("Inbox")).expect("crear Inbox");
        fs::write(
            root.join("Macroeconomia.md"),
            "---\ntitle: Macroeconomía\naliases: [\"Economía Global\"]\ntags: [macroeconomia]\nestado: activo\n---\n\n# Macroeconomía\n\nAnálisis de inflación, política monetaria y tasas de interés.\n",
        )
        .expect("escribir Macroeconomia.md");

        fs::write(
            root.join("Politica Monetaria.md"),
            "---\ntitle: Política Monetaria\ntags: [macroeconomia, bancos-centrales]\nestado: revision\n---\n\n# Política Monetaria\n\nEstudia la [[Macroeconomia|Macroeconomía]] y su impacto en la inflación.\n",
        )
        .expect("escribir Politica Monetaria.md");

        fs::write(
            root.join("Inbox/Video YouTube.md"),
            "---\nsource: youtube\nurl: https://www.youtube.com/watch?v=abc123\ntags: [youtube]\n---\n\n# Charla sobre inflación\n\nHoy analizamos la Economía Global y los efectos de la Política Monetaria.\n\n```md\nEconomía Global dentro de código no se debe tocar.\n```\n",
        )
        .expect("escribir Inbox/Video YouTube.md");
    }

    #[test]
    fn resolve_vault_path_prefers_cli_over_env_and_fallback() {
        let cwd_guard = TempDirGuard::new("resolve");
        let cwd = &cwd_guard.path;
        fs::create_dir_all(cwd.join("test-vault")).expect("crear test-vault");
        fs::create_dir_all(cwd.join("custom-vault")).expect("crear custom-vault");

        let from_flag = resolve_vault_path(
            &["--vault".into(), "custom-vault".into()],
            Some("ignored-env"),
            cwd,
        );
        assert_eq!(from_flag, cwd.join("custom-vault"));

        let from_equals = resolve_vault_path(
            &["--vault=custom-vault".into()],
            Some("ignored-env"),
            cwd,
        );
        assert_eq!(from_equals, cwd.join("custom-vault"));

        let from_env = resolve_vault_path(&[], Some("custom-vault"), cwd);
        assert_eq!(from_env, cwd.join("custom-vault"));

        let from_default = resolve_vault_path(&[], None, cwd);
        assert_eq!(from_default, cwd.join("test-vault"));
    }

    #[test]
    fn mcp_lifecycle_initialize_notification_and_tools_list() {
        let vault_guard = TempDirGuard::new("lifecycle");
        seed_test_notes(&vault_guard.path);
        let mut server = McpServer::open(&vault_guard.path).expect("abrir servidor MCP");

        let init_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "clientInfo": { "name": "test-client", "version": "1.0" }
                }
            }))
            .expect("initialize responde");
        assert_eq!(init_resp["result"]["serverInfo"]["name"], "romsidian-mcp");
        assert_eq!(init_resp["result"]["protocolVersion"], "2024-11-05");

        let notif_resp = server.handle_jsonrpc_value(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
        assert!(notif_resp.is_none(), "notificación no debe responder");

        let ping_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "ping"
            }))
            .expect("ping responde");
        assert_eq!(ping_resp["result"], json!({}));

        let list_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/list"
            }))
            .expect("tools/list responde");
        let tools = list_resp["result"]["tools"]
            .as_array()
            .expect("array tools");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        for expected in [
            "vault_knowledge_map",
            "search_notes_fts",
            "run_dataview_dql",
            "read_note",
            "write_note",
            "suggest_note_connections",
            "auto_connect_notes",
            "create_knowledge_canvas",
        ] {
            assert!(
                names.contains(&expected),
                "falta la herramienta esperada '{expected}'"
            );
        }
        assert_eq!(names.len(), 8);
    }

    #[test]
    fn mcp_tools_execute_end_to_end_and_validate_safe_paths() {
        let vault_guard = TempDirGuard::new("tools_e2e");
        seed_test_notes(&vault_guard.path);
        let mut server = McpServer::open(&vault_guard.path).expect("abrir servidor MCP");

        // 1. vault_knowledge_map
        let map_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "tools/call",
                "params": {
                    "name": "vault_knowledge_map",
                    "arguments": { "max_notes": 50 }
                }
            }))
            .expect("respuesta vault_knowledge_map");
        assert_eq!(map_resp["result"]["isError"], false);
        assert_eq!(map_resp["result"]["structuredContent"]["map"]["total_notes"], 3);
        let compact_text = map_resp["result"]["structuredContent"]["compact_prompt_context"]
            .as_str()
            .expect("compact_prompt_context string");
        assert!(compact_text.contains("Macroeconomia.md"));

        // 2. search_notes_fts (sin tilde encuentra con tilde)
        let search_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "tools/call",
                "params": {
                    "name": "search_notes_fts",
                    "arguments": { "query": "inflacion", "limit": 5 }
                }
            }))
            .expect("respuesta search_notes_fts");
        assert_eq!(search_resp["result"]["isError"], false);
        assert!(
            search_resp["result"]["structuredContent"]["count"]
                .as_u64()
                .unwrap_or(0)
                >= 2
        );

        // 3. write_note segura + rechazo de traversal
        let bad_write = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "tools/call",
                "params": {
                    "name": "write_note",
                    "arguments": { "path": "../escape.md", "content": "hola" }
                }
            }))
            .expect("respuesta write_note insegura");
        assert_eq!(bad_write["result"]["isError"], true);

        let good_write = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 13,
                "method": "tools/call",
                "params": {
                    "name": "write_note",
                    "arguments": {
                        "path": "Nueva Nota.md",
                        "content": "---\nestado: activo\ntags: [macroeconomia]\n---\n\n# Nueva Nota\n\nTexto breve."
                    }
                }
            }))
            .expect("respuesta write_note segura");
        assert_eq!(good_write["result"]["isError"], false);

        // 4. read_note
        let read_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 14,
                "method": "tools/call",
                "params": {
                    "name": "read_note",
                    "arguments": { "path": "Macroeconomia.md" }
                }
            }))
            .expect("respuesta read_note");
        assert_eq!(read_resp["result"]["isError"], false);
        assert!(
            !read_resp["result"]["structuredContent"]["backlinks"]
                .as_array()
                .expect("backlinks array")
                .is_empty(),
            "Macroeconomia.md debe tener backlinks desde Politica Monetaria.md"
        );

        // 5. run_dataview_dql
        let dql_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 15,
                "method": "tools/call",
                "params": {
                    "name": "run_dataview_dql",
                    "arguments": { "query": "TABLE estado FROM #macroeconomia" }
                }
            }))
            .expect("respuesta run_dataview_dql");
        assert_eq!(dql_resp["result"]["isError"], false);
        assert_eq!(dql_resp["result"]["structuredContent"]["result"]["view"], "table");
        assert_eq!(
            dql_resp["result"]["structuredContent"]["result"]["total_count"],
            3
        );

        // 6. suggest_note_connections
        let suggest_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 16,
                "method": "tools/call",
                "params": {
                    "name": "suggest_note_connections",
                    "arguments": { "path": "Inbox/Video YouTube.md", "max_candidates": 5 }
                }
            }))
            .expect("respuesta suggest_note_connections");
        assert_eq!(suggest_resp["result"]["isError"], false);
        assert!(
            suggest_resp["result"]["structuredContent"]["count"]
                .as_u64()
                .unwrap_or(0)
                >= 1
        );

        // 7. auto_connect_notes en preview y luego apply por carpeta Inbox
        let preview_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 17,
                "method": "tools/call",
                "params": {
                    "name": "auto_connect_notes",
                    "arguments": { "folder": "Inbox", "mode": "preview" }
                }
            }))
            .expect("respuesta auto_connect preview");
        assert_eq!(preview_resp["result"]["isError"], false);
        assert_eq!(preview_resp["result"]["structuredContent"]["changed_count"], 1);
        let untouched = fs::read_to_string(vault_guard.path.join("Inbox/Video YouTube.md"))
            .expect("leer nota tras preview");
        assert!(
            !untouched.contains("[[Politica Monetaria"),
            "preview no debe modificar el archivo en disco"
        );

        let apply_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 18,
                "method": "tools/call",
                "params": {
                    "name": "auto_connect_notes",
                    "arguments": { "folder": "Inbox", "mode": "apply" }
                }
            }))
            .expect("respuesta auto_connect apply");
        assert_eq!(apply_resp["result"]["isError"], false);
        assert_eq!(apply_resp["result"]["structuredContent"]["changed_count"], 1);
        let updated = fs::read_to_string(vault_guard.path.join("Inbox/Video YouTube.md"))
            .expect("leer nota tras apply");
        assert!(updated.contains("[[Politica Monetaria"));
        assert!(updated.contains("[[Macroeconomia|Economía Global]]"));
        assert!(
            updated.contains("Economía Global dentro de código no se debe tocar."),
            "el bloque de código cercado debe quedar intacto"
        );

        // Segunda pasada idempotente: no debe volver a modificar ni duplicar enlaces.
        let idempotent_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 180,
                "method": "tools/call",
                "params": {
                    "name": "auto_connect_notes",
                    "arguments": { "folder": "Inbox", "mode": "apply" }
                }
            }))
            .expect("respuesta auto_connect idempotente");
        assert_eq!(idempotent_resp["result"]["isError"], false);
        assert_eq!(
            idempotent_resp["result"]["structuredContent"]["changed_count"],
            0,
            "segunda pasada debe ser idempotente;\napply_resp: {}\nupdated:\n{}\nidempotent_resp: {}",
            serde_json::to_string_pretty(&apply_resp).unwrap_or_default(),
            updated,
            serde_json::to_string_pretty(&idempotent_resp).unwrap_or_default()
        );

        // Plan explícito enviado por agente externo (frontmatter + tags) y bloqueo de rutas inseguras.
        let custom_plan_resp = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 181,
                "method": "tools/call",
                "params": {
                    "name": "auto_connect_notes",
                    "arguments": {
                        "path": "Nueva Nota.md",
                        "mode": "apply",
                        "plan": {
                            "frontmatter_updates": { "tipo": "resumen", "fuente": "youtube" },
                            "suggested_tags": ["ia/copiloto"]
                        }
                    }
                }
            }))
            .expect("respuesta custom plan");
        assert_eq!(custom_plan_resp["result"]["isError"], false);
        assert_eq!(
            custom_plan_resp["result"]["structuredContent"]["changed_count"],
            1
        );
        let nueva_md = fs::read_to_string(vault_guard.path.join("Nueva Nota.md"))
            .expect("leer Nueva Nota.md");
        assert!(nueva_md.contains("tipo: resumen"));
        assert!(nueva_md.contains("ia/copiloto"));

        for bad_folder in ["../otra", ".romsidian", "C:/Windows"] {
            let bad_folder_resp = server
                .handle_jsonrpc_value(&json!({
                    "jsonrpc": "2.0",
                    "id": 182,
                    "method": "tools/call",
                    "params": {
                        "name": "auto_connect_notes",
                        "arguments": { "folder": bad_folder, "mode": "preview" }
                    }
                }))
                .expect("respuesta carpeta insegura");
            assert_eq!(
                bad_folder_resp["result"]["isError"], true,
                "debe rechazar carpeta insegura: {bad_folder}"
            );
        }

        // 8. create_knowledge_canvas + rechazo traversal
        let bad_canvas = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 19,
                "method": "tools/call",
                "params": {
                    "name": "create_knowledge_canvas",
                    "arguments": { "canvas_path": "../malo.canvas" }
                }
            }))
            .expect("respuesta canvas inseguro");
        assert_eq!(bad_canvas["result"]["isError"], true);

        let good_canvas = server
            .handle_jsonrpc_value(&json!({
                "jsonrpc": "2.0",
                "id": 20,
                "method": "tools/call",
                "params": {
                    "name": "create_knowledge_canvas",
                    "arguments": {
                        "canvas_path": "Mapa Macro.canvas",
                        "title": "Mapa Económico",
                        "seed_note_paths": ["Inbox/Video YouTube.md"],
                        "include_neighbors": true,
                        "max_nodes": 8
                    }
                }
            }))
            .expect("respuesta canvas válido");
        assert_eq!(good_canvas["result"]["isError"], false);
        assert!(vault_guard.path.join("Mapa Macro.canvas").is_file());
    }

    #[test]
    fn stdio_stream_handles_newline_and_content_length_framing() {
        let vault_guard = TempDirGuard::new("framing");
        seed_test_notes(&vault_guard.path);
        let mut server = McpServer::open(&vault_guard.path).expect("abrir servidor MCP");

        // 1. Prueba con mensajes delimitados por '\n' (incluido BOM Windows inicial).
        let line_input = "\u{feff}{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
        let mut line_out = Vec::new();
        server
            .process_stdio_stream(Cursor::new(line_input.as_bytes()), &mut line_out)
            .expect("procesar stdio jsonline");
        let out_str = String::from_utf8(line_out).expect("utf8");
        let lines: Vec<&str> = out_str.lines().collect();
        assert_eq!(lines.len(), 2, "solo 2 respuestas (no para notification)");
        let first: Value = serde_json::from_str(lines[0]).expect("parsear resp 1");
        let second: Value = serde_json::from_str(lines[1]).expect("parsear resp 2");
        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
        assert_eq!(second["result"]["tools"].as_array().unwrap().len(), 8);

        // 2. Prueba con cabeceras Content-Length.
        let body = "{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"ping\"}";
        let framed_in = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut framed_out = Vec::new();
        server
            .process_stdio_stream(Cursor::new(framed_in.as_bytes()), &mut framed_out)
            .expect("procesar stdio Content-Length");
        let framed_str = String::from_utf8(framed_out).expect("utf8 framed");
        assert!(framed_str.starts_with("Content-Length: "));
        assert!(framed_str.contains("\"id\":99"));
    }
}
