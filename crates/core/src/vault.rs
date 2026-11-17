//! Gestión de bóveda local y operaciones CRUD sobre archivos Markdown (`.md`).
//!
//! Cumple con `[RF-01]` (gestión de bóveda y árbol de notas en UTF-8) y `[RF-02]`
//! (inicialización automática de `.romsidian/` y su exclusión en `.gitignore`).

use crate::{CoreError, CoreResult, VaultInfo, VAULT_METADATA_DIR};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// JSON Canvas 1.0 vacío por defecto al crear un nuevo lienzo `.canvas`.
pub const DEFAULT_CANVAS_CONTENT: &str = "{\n  \"nodes\": [],\n  \"edges\": []\n}\n";

/// Resumen ligero de una nota dentro de la bóveda (sin cargar todo su contenido).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteSummary {
    /// Ruta relativa normalizada con barras `/` desde la raíz de la bóveda (ej. `subcarpeta/Nota.md`).
    pub relative_path: String,
    /// Título de la nota (nombre de archivo sin la extensión `.md`).
    pub title: String,
    /// Marca de tiempo de última modificación en segundos Unix.
    pub modified_unix_secs: u64,
    /// Tamaño del archivo en bytes.
    pub size_bytes: u64,
}

/// Documento completo de una nota Markdown leída o guardada en disco.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDocument {
    /// Ruta relativa normalizada con barras `/` (ej. `Análisis Macroeconómico.md`).
    pub relative_path: String,
    /// Título de la nota (nombre de archivo sin extensión `.md`).
    pub title: String,
    /// Contenido completo en UTF-8.
    pub content: String,
    /// Marca de tiempo de última modificación en segundos Unix.
    pub modified_unix_secs: u64,
}

/// Resumen ligero de un archivo JSON Canvas (`.canvas`) dentro de la bóveda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasSummary {
    /// Ruta relativa normalizada con barras `/` (ej. `Mapas/Arquitectura.canvas`).
    pub relative_path: String,
    /// Nombre del lienzo sin la extensión `.canvas`.
    pub title: String,
    /// Marca de tiempo de última modificación en segundos Unix.
    pub modified_unix_secs: u64,
    /// Tamaño del archivo en bytes.
    pub size_bytes: u64,
}

/// Documento completo de un lienzo JSON Canvas (`.canvas`) leído o guardado en disco.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasDocument {
    /// Ruta relativa normalizada con barras `/` (ej. `Arquitectura.canvas`).
    pub relative_path: String,
    /// Nombre del lienzo sin la extensión `.canvas`.
    pub title: String,
    /// Contenido JSON Canvas 1.0 en UTF-8.
    pub content: String,
    /// Marca de tiempo de última modificación en segundos Unix.
    pub modified_unix_secs: u64,
}

/// Resumen de una plantilla Markdown ubicada en la carpeta de plantillas (`templates/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSummary {
    /// Ruta relativa normalizada con barras `/` (ej. `templates/Reunión.md`).
    pub relative_path: String,
    /// Nombre legible de la plantilla (sin carpeta base ni `.md`).
    pub name: String,
}

/// Resultado de abrir o crear una nota diaria (`daily/YYYY-MM-DD.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyNoteResult {
    /// Documento de la nota diaria lista para abrir en el editor.
    pub note: NoteDocument,
    /// `true` si la nota diaria se acaba de crear en esta llamada, `false` si ya existía.
    pub created: bool,
}

/// Nodo jerárquico del árbol de archivos de la bóveda para la barra lateral de la UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FileTreeNode {
    Folder {
        name: String,
        relative_path: String,
        children: Vec<FileTreeNode>,
    },
    Note(NoteSummary),
}

/// Representa una bóveda local abierta en el sistema de archivos.
#[derive(Debug, Clone)]
pub struct Vault {
    root_path: PathBuf,
    info: VaultInfo,
}

impl Vault {
    /// Abre una carpeta existente como bóveda, inicializando `.romsidian/` y asegurando
    /// que `.romsidian/` esté presente en el archivo `.gitignore` de la bóveda (`[RF-02]`).
    pub fn open(root_path: impl AsRef<Path>) -> CoreResult<Self> {
        let raw_path = root_path.as_ref();
        if !raw_path.exists() || !raw_path.is_dir() {
            return Err(CoreError::InvalidVaultPath(
                raw_path.display().to_string(),
            ));
        }

        let metadata_dir = raw_path.join(VAULT_METADATA_DIR);
        if !metadata_dir.exists() {
            fs::create_dir_all(&metadata_dir)?;
        }

        ensure_gitignore_excludes_metadata(raw_path)?;

        let vault_name = raw_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Bóveda")
            .to_string();

        let root_str = normalize_slashes(&raw_path.to_string_lossy());
        let info = VaultInfo::new(vault_name, root_str);

        Ok(Self {
            root_path: raw_path.to_path_buf(),
            info,
        })
    }

    /// Devuelve la información descriptiva de la bóveda.
    pub fn info(&self) -> &VaultInfo {
        &self.info
    }

    /// Devuelve la ruta raíz de la bóveda en el sistema de archivos.
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Devuelve la ruta a la carpeta `.romsidian/` dentro de la bóveda.
    pub fn metadata_dir(&self) -> PathBuf {
        self.root_path.join(VAULT_METADATA_DIR)
    }

    /// Lista recursivamente todas las notas `.md` de la bóveda ignorando carpetas ocultas (`.romsidian`, `.git`, etc.).
    pub fn list_notes(&self) -> CoreResult<Vec<NoteSummary>> {
        let mut notes = Vec::new();
        self.collect_notes_recursive(&self.root_path, &mut notes)?;
        notes.sort_by(|a, b| {
            a.relative_path
                .to_lowercase()
                .cmp(&b.relative_path.to_lowercase())
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        Ok(notes)
    }

    /// Construye el árbol jerárquico de carpetas y notas `.md` ignorando directorios ocultos.
    pub fn file_tree(&self) -> CoreResult<Vec<FileTreeNode>> {
        self.build_tree_recursive(&self.root_path)
    }

    /// Crea una nueva nota `.md` en UTF-8. Falla con `CoreError::NoteAlreadyExists` si ya existe.
    pub fn create_note(&self, relative_path: &str, initial_content: &str) -> CoreResult<NoteDocument> {
        let (clean_rel, full_path) = self.resolve_safe_note_path(relative_path)?;
        if full_path.exists() {
            return Err(CoreError::NoteAlreadyExists(clean_rel));
        }
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, initial_content.as_bytes())?;
        self.read_note(&clean_rel)
    }

    /// Lee el contenido completo en UTF-8 de una nota existente.
    pub fn read_note(&self, relative_path: &str) -> CoreResult<NoteDocument> {
        let (clean_rel, full_path) = self.resolve_safe_note_path(relative_path)?;
        if !full_path.exists() || !full_path.is_file() {
            return Err(CoreError::NoteNotFound(clean_rel));
        }
        let content = fs::read_to_string(&full_path)?;
        let metadata = fs::metadata(&full_path)?;
        let modified_unix_secs = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let title = extract_title_from_rel_path(&clean_rel);

        Ok(NoteDocument {
            relative_path: clean_rel,
            title,
            content,
            modified_unix_secs,
        })
    }

    /// Guarda (sobrescribe o crea) el contenido UTF-8 de una nota en disco.
    pub fn save_note(&self, relative_path: &str, content: &str) -> CoreResult<NoteDocument> {
        let (clean_rel, full_path) = self.resolve_safe_note_path(relative_path)?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, content.as_bytes())?;
        self.read_note(&clean_rel)
    }

    /// Renombra o mueve una nota dentro de la bóveda preservando su contenido.
    pub fn rename_note(
        &self,
        old_relative_path: &str,
        new_relative_path: &str,
    ) -> CoreResult<NoteSummary> {
        let (old_rel, old_full) = self.resolve_safe_note_path(old_relative_path)?;
        let (new_rel, new_full) = self.resolve_safe_note_path(new_relative_path)?;

        if !old_full.exists() || !old_full.is_file() {
            return Err(CoreError::NoteNotFound(old_rel));
        }

        let same_case_insensitive = old_rel.to_lowercase() == new_rel.to_lowercase();
        if new_full.exists() && !same_case_insensitive {
            return Err(CoreError::NoteAlreadyExists(new_rel));
        }

        if let Some(parent) = new_full.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::rename(&old_full, &new_full)?;
        self.build_note_summary(&new_full, &new_rel)
    }

    /// Elimina una nota `.md` de la bóveda.
    pub fn delete_note(&self, relative_path: &str) -> CoreResult<()> {
        let (clean_rel, full_path) = self.resolve_safe_note_path(relative_path)?;
        if !full_path.exists() || !full_path.is_file() {
            return Err(CoreError::NoteNotFound(clean_rel));
        }
        fs::remove_file(full_path)?;
        Ok(())
    }

    /// Resuelve el destino de un `[[wikilink]]` (por ruta relativa o por título de nota,
    /// sin sensibilidad a mayúsculas/minúsculas y soportando tildes/eñes) hacia la ruta
    /// relativa real dentro de la bóveda, o `None` si la nota aún no existe.
    pub fn resolve_wikilink_path(&self, target: &str) -> CoreResult<Option<String>> {
        let trimmed = target.trim().trim_matches(['/', '\\']);
        if trimmed.is_empty() {
            return Ok(None);
        }

        let normalized_target = normalize_slashes(trimmed);
        let target_with_ext = if normalized_target.to_lowercase().ends_with(".md") {
            normalized_target.clone()
        } else {
            format!("{}.md", normalized_target)
        };
        let target_stem = normalized_target
            .strip_suffix(".md")
            .or_else(|| normalized_target.strip_suffix(".MD"))
            .unwrap_or(&normalized_target)
            .to_lowercase();
        let target_rel_lower = target_with_ext.to_lowercase();

        let notes = self.list_notes()?;

        // 1. Coincidencia exacta por ruta relativa (case-insensitive)
        if let Some(found) = notes
            .iter()
            .find(|n| n.relative_path.to_lowercase() == target_rel_lower)
        {
            return Ok(Some(found.relative_path.clone()));
        }

        // 2. Coincidencia por título / nombre de archivo sin importar subcarpeta
        if let Some(found) = notes
            .iter()
            .find(|n| n.title.to_lowercase() == target_stem)
        {
            return Ok(Some(found.relative_path.clone()));
        }

        Ok(None)
    }

    /// Lista recursivamente todos los archivos `.canvas` de la bóveda ignorando carpetas ocultas.
    pub fn list_canvases(&self) -> CoreResult<Vec<CanvasSummary>> {
        let mut raw_entries = Vec::new();
        self.collect_files_with_ext_recursive(&self.root_path, "canvas", &mut raw_entries)?;
        let mut canvases: Vec<CanvasSummary> = raw_entries
            .into_iter()
            .map(|(rel_path, modified_unix_secs, size_bytes)| CanvasSummary {
                title: extract_stem_from_rel_path(&rel_path, "canvas"),
                relative_path: rel_path,
                modified_unix_secs,
                size_bytes,
            })
            .collect();
        canvases.sort_by(|a, b| {
            a.relative_path
                .to_lowercase()
                .cmp(&b.relative_path.to_lowercase())
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        Ok(canvases)
    }

    /// Crea un nuevo lienzo JSON Canvas (`.canvas`) validando su JSON inicial.
    pub fn create_canvas(
        &self,
        relative_path: &str,
        initial_content: &str,
    ) -> CoreResult<CanvasDocument> {
        let (clean_rel, full_path) = self.resolve_safe_canvas_path(relative_path)?;
        if full_path.exists() {
            return Err(CoreError::NoteAlreadyExists(clean_rel));
        }
        let payload = if initial_content.trim().is_empty() {
            DEFAULT_CANVAS_CONTENT
        } else {
            initial_content
        };
        validate_canvas_json(payload)?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, payload.as_bytes())?;
        self.read_canvas(&clean_rel)
    }

    /// Lee el contenido JSON completo en UTF-8 de un lienzo `.canvas` existente.
    pub fn read_canvas(&self, relative_path: &str) -> CoreResult<CanvasDocument> {
        let (clean_rel, full_path) = self.resolve_safe_canvas_path(relative_path)?;
        if !full_path.exists() || !full_path.is_file() {
            return Err(CoreError::NoteNotFound(clean_rel));
        }
        let content = fs::read_to_string(&full_path)?;
        let metadata = fs::metadata(&full_path)?;
        let modified_unix_secs = metadata_modified_unix_secs(&metadata);
        let title = extract_stem_from_rel_path(&clean_rel, "canvas");

        Ok(CanvasDocument {
            relative_path: clean_rel,
            title,
            content,
            modified_unix_secs,
        })
    }

    /// Guarda (sobrescribe o crea) un lienzo `.canvas` tras validar que contiene un objeto JSON válido.
    pub fn save_canvas(&self, relative_path: &str, content: &str) -> CoreResult<CanvasDocument> {
        let (clean_rel, full_path) = self.resolve_safe_canvas_path(relative_path)?;
        validate_canvas_json(content)?;
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full_path, content.as_bytes())?;
        self.read_canvas(&clean_rel)
    }

    /// Renombra o mueve un archivo `.canvas` dentro de la bóveda.
    pub fn rename_canvas(
        &self,
        old_relative_path: &str,
        new_relative_path: &str,
    ) -> CoreResult<CanvasSummary> {
        let (old_rel, old_full) = self.resolve_safe_canvas_path(old_relative_path)?;
        let (new_rel, new_full) = self.resolve_safe_canvas_path(new_relative_path)?;

        if !old_full.exists() || !old_full.is_file() {
            return Err(CoreError::NoteNotFound(old_rel));
        }

        let same_case_insensitive = old_rel.to_lowercase() == new_rel.to_lowercase();
        if new_full.exists() && !same_case_insensitive {
            return Err(CoreError::NoteAlreadyExists(new_rel));
        }

        if let Some(parent) = new_full.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::rename(&old_full, &new_full)?;
        let metadata = fs::metadata(&new_full)?;
        Ok(CanvasSummary {
            title: extract_stem_from_rel_path(&new_rel, "canvas"),
            relative_path: new_rel,
            modified_unix_secs: metadata_modified_unix_secs(&metadata),
            size_bytes: metadata.len(),
        })
    }

    /// Elimina un archivo `.canvas` de la bóveda.
    pub fn delete_canvas(&self, relative_path: &str) -> CoreResult<()> {
        let (clean_rel, full_path) = self.resolve_safe_canvas_path(relative_path)?;
        if !full_path.exists() || !full_path.is_file() {
            return Err(CoreError::NoteNotFound(clean_rel));
        }
        fs::remove_file(full_path)?;
        Ok(())
    }

    /// Lista las plantillas Markdown disponibles dentro de `templates/` (o la carpeta indicada).
    /// Devuelve una lista vacía si la carpeta todavía no existe en la bóveda.
    pub fn list_templates(&self, templates_folder: Option<&str>) -> CoreResult<Vec<TemplateSummary>> {
        let folder = templates_folder.unwrap_or("templates");
        let (clean_folder, full_folder) = self.resolve_safe_folder_path(folder)?;
        if !full_folder.exists() || !full_folder.is_dir() {
            return Ok(Vec::new());
        }

        let mut raw_entries = Vec::new();
        self.collect_files_with_ext_recursive(&full_folder, "md", &mut raw_entries)?;

        let prefix = if clean_folder.is_empty() {
            String::new()
        } else {
            format!("{clean_folder}/")
        };

        let mut templates: Vec<TemplateSummary> = raw_entries
            .into_iter()
            .map(|(rel_path, _, _)| {
                let without_prefix = rel_path.strip_prefix(&prefix).unwrap_or(&rel_path);
                let name = without_prefix
                    .strip_suffix(".md")
                    .or_else(|| without_prefix.strip_suffix(".MD"))
                    .unwrap_or(without_prefix)
                    .to_string();
                TemplateSummary {
                    relative_path: rel_path,
                    name,
                }
            })
            .collect();

        templates.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(templates)
    }

    /// Lee una plantilla de la bóveda y sustituye `{{date}}`, `{{time}}` y `{{title}}`.
    pub fn apply_template(
        &self,
        template_relative_path: &str,
        target_title: &str,
        date_str: Option<&str>,
        time_str: Option<&str>,
    ) -> CoreResult<String> {
        let doc = self.read_note(template_relative_path)?;
        let (default_date, default_time) = current_utc_date_time_strings();
        let effective_date = date_str
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&default_date);
        let effective_time = time_str
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&default_time);
        Ok(substitute_template_variables(
            &doc.content,
            target_title,
            effective_date,
            effective_time,
        ))
    }

    /// Abre la nota diaria correspondiente a `date_str` (`YYYY-MM-DD`, por defecto hoy) o la crea
    /// bajo `daily/` aplicando una plantilla si existe.
    pub fn get_or_create_daily_note(
        &self,
        date_str: Option<&str>,
        time_str: Option<&str>,
        daily_folder: Option<&str>,
        template_path: Option<&str>,
    ) -> CoreResult<DailyNoteResult> {
        let (default_date, default_time) = current_utc_date_time_strings();
        let effective_date = date_str
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&default_date);
        let effective_time = time_str
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&default_time);

        if effective_date.contains('/') || effective_date.contains('\\') || effective_date.contains("..") {
            return Err(CoreError::InvalidNotePath(effective_date.to_string()));
        }

        let folder = daily_folder.unwrap_or("daily");
        let (clean_folder, _) = self.resolve_safe_folder_path(folder)?;
        let target_rel = if clean_folder.is_empty() {
            format!("{effective_date}.md")
        } else {
            format!("{clean_folder}/{effective_date}.md")
        };

        let (resolved_rel, resolved_full) = self.resolve_safe_note_path(&target_rel)?;
        if resolved_full.exists() && resolved_full.is_file() {
            let note = self.read_note(&resolved_rel)?;
            return Ok(DailyNoteResult {
                note,
                created: false,
            });
        }

        let initial_content = if let Some(explicit_tpl) = template_path.map(str::trim).filter(|s| !s.is_empty()) {
            self.apply_template(
                explicit_tpl,
                effective_date,
                Some(effective_date),
                Some(effective_time),
            )?
        } else {
            let fallback_candidates = ["templates/daily.md", "templates/Daily.md"];
            let found_tpl = fallback_candidates
                .iter()
                .find(|cand| self.read_note(cand).is_ok())
                .copied();
            if let Some(tpl) = found_tpl {
                self.apply_template(
                    tpl,
                    effective_date,
                    Some(effective_date),
                    Some(effective_time),
                )?
            } else {
                substitute_template_variables(
                    "# {{date}}\n\n## Notas del día\n\n- [ ] \n",
                    effective_date,
                    effective_date,
                    effective_time,
                )
            }
        };

        let note = self.create_note(&resolved_rel, &initial_content)?;
        Ok(DailyNoteResult {
            note,
            created: true,
        })
    }

    fn collect_notes_recursive(&self, dir: &Path, acc: &mut Vec<NoteSummary>) -> CoreResult<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => continue,
            };

            // Ignorar archivos y carpetas ocultas (ej. .romsidian, .git, .claude)
            if file_name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                self.collect_notes_recursive(&path, acc)?;
            } else if is_markdown_file(&path) {
                let rel_path = self.to_relative_slash_path(&path)?;
                let summary = self.build_note_summary(&path, &rel_path)?;
                acc.push(summary);
            }
        }
        Ok(())
    }

    fn build_tree_recursive(&self, dir: &Path) -> CoreResult<Vec<FileTreeNode>> {
        let mut folders = Vec::new();
        let mut notes = Vec::new();

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            if file_name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                let children = self.build_tree_recursive(&path)?;
                let rel_path = self.to_relative_slash_path(&path)?;
                folders.push(FileTreeNode::Folder {
                    name: file_name,
                    relative_path: rel_path,
                    children,
                });
            } else if is_markdown_file(&path) {
                let rel_path = self.to_relative_slash_path(&path)?;
                let summary = self.build_note_summary(&path, &rel_path)?;
                notes.push(FileTreeNode::Note(summary));
            }
        }

        folders.sort_by(|a, b| match (a, b) {
            (FileTreeNode::Folder { name: na, .. }, FileTreeNode::Folder { name: nb, .. }) => {
                na.to_lowercase().cmp(&nb.to_lowercase())
            }
            _ => std::cmp::Ordering::Equal,
        });

        notes.sort_by(|a, b| match (a, b) {
            (FileTreeNode::Note(na), FileTreeNode::Note(nb)) => {
                na.title.to_lowercase().cmp(&nb.title.to_lowercase())
            }
            _ => std::cmp::Ordering::Equal,
        });

        folders.extend(notes);
        Ok(folders)
    }

    fn build_note_summary(&self, full_path: &Path, relative_path: &str) -> CoreResult<NoteSummary> {
        let metadata = fs::metadata(full_path)?;
        let modified_unix_secs = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let title = extract_title_from_rel_path(relative_path);

        Ok(NoteSummary {
            relative_path: relative_path.to_string(),
            title,
            modified_unix_secs,
            size_bytes: metadata.len(),
        })
    }

    fn to_relative_slash_path(&self, full_path: &Path) -> CoreResult<String> {
        let rel = full_path
            .strip_prefix(&self.root_path)
            .map_err(|_| CoreError::InvalidNotePath(full_path.display().to_string()))?;
        Ok(normalize_slashes(&rel.to_string_lossy()))
    }

    /// Valida que la ruta relativa sea segura (sin `..`, sin rutas absolutas ni carpetas ocultas)
    /// y asegura la extensión `.md`.
    fn resolve_safe_note_path(&self, relative_path: &str) -> CoreResult<(String, PathBuf)> {
        self.resolve_safe_path_with_ext(
            relative_path,
            "md",
            "La ruta de la nota no puede estar vacía",
        )
    }

    /// Valida que la ruta relativa de un `.canvas` sea segura y asegura extensión `.canvas`.
    fn resolve_safe_canvas_path(&self, relative_path: &str) -> CoreResult<(String, PathBuf)> {
        self.resolve_safe_path_with_ext(
            relative_path,
            "canvas",
            "La ruta del lienzo no puede estar vacía",
        )
    }

    fn resolve_safe_path_with_ext(
        &self,
        relative_path: &str,
        ext: &str,
        empty_msg: &str,
    ) -> CoreResult<(String, PathBuf)> {
        let trimmed = relative_path.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidNotePath(empty_msg.to_string()));
        }

        let normalized = normalize_slashes(trimmed);
        let suffix_lower = format!(".{}", ext.to_lowercase());
        let with_ext = if normalized.to_lowercase().ends_with(&suffix_lower) {
            let stem = &normalized[..normalized.len() - suffix_lower.len()];
            format!("{stem}{suffix_lower}")
        } else {
            format!("{normalized}{suffix_lower}")
        };

        let rel_path = Path::new(&with_ext);
        let clean_parts = validate_clean_relative_components(rel_path, relative_path)?;
        if clean_parts.is_empty() {
            return Err(CoreError::InvalidNotePath(relative_path.to_string()));
        }

        let clean_rel = clean_parts.join("/");
        let mut full_path = self.root_path.clone();
        for part in &clean_parts {
            full_path.push(part);
        }

        Ok((clean_rel, full_path))
    }

    fn resolve_safe_folder_path(&self, relative_folder: &str) -> CoreResult<(String, PathBuf)> {
        let trimmed = relative_folder.trim().trim_matches(['/', '\\']);
        if trimmed.is_empty() {
            return Ok((String::new(), self.root_path.clone()));
        }
        let normalized = normalize_slashes(trimmed);
        let rel_path = Path::new(&normalized);
        let clean_parts = validate_clean_relative_components(rel_path, relative_folder)?;
        let clean_rel = clean_parts.join("/");
        let mut full_path = self.root_path.clone();
        for part in &clean_parts {
            full_path.push(part);
        }
        Ok((clean_rel, full_path))
    }

    fn collect_files_with_ext_recursive(
        &self,
        dir: &Path,
        ext: &str,
        acc: &mut Vec<(String, u64, u64)>,
    ) -> CoreResult<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => continue,
            };
            if file_name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                self.collect_files_with_ext_recursive(&path, ext, acc)?;
            } else if has_extension_ascii_case(&path, ext) {
                let rel_path = self.to_relative_slash_path(&path)?;
                let metadata = fs::metadata(&path)?;
                acc.push((
                    rel_path,
                    metadata_modified_unix_secs(&metadata),
                    metadata.len(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_clean_relative_components(rel_path: &Path, raw_input: &str) -> CoreResult<Vec<String>> {
    let mut clean_parts = Vec::new();
    for component in rel_path.components() {
        match component {
            Component::Normal(os_str) => {
                let part = os_str
                    .to_str()
                    .ok_or_else(|| CoreError::InvalidNotePath(raw_input.to_string()))?;
                let part_trimmed = part.trim();
                if part_trimmed.is_empty() || part_trimmed.starts_with('.') {
                    return Err(CoreError::InvalidNotePath(raw_input.to_string()));
                }
                clean_parts.push(part_trimmed.to_string());
            }
            _ => {
                return Err(CoreError::InvalidNotePath(raw_input.to_string()));
            }
        }
    }
    Ok(clean_parts)
}

fn validate_canvas_json(raw: &str) -> CoreResult<()> {
    let parsed: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("JSON Canvas inválido: {e}"),
        )
    })?;
    if !parsed.is_object() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "JSON Canvas inválido: el documento raíz debe ser un objeto JSON",
        )
        .into());
    }
    Ok(())
}

fn metadata_modified_unix_secs(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn ensure_gitignore_excludes_metadata(vault_root: &Path) -> CoreResult<()> {
    let gitignore_path = vault_root.join(".gitignore");
    let entry_slash = format!("{}/", VAULT_METADATA_DIR);

    if gitignore_path.exists() {
        let existing = fs::read_to_string(&gitignore_path)?;
        let already_ignored = existing.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == VAULT_METADATA_DIR
                || trimmed == entry_slash
                || trimmed == format!("/{}", VAULT_METADATA_DIR)
                || trimmed == format!("/{}/", VAULT_METADATA_DIR)
        });

        if !already_ignored {
            let mut updated = existing;
            if !updated.ends_with('\n') && !updated.is_empty() {
                updated.push('\n');
            }
            updated.push_str(&format!("{}\n", entry_slash));
            fs::write(&gitignore_path, updated.as_bytes())?;
        }
    } else {
        let default_content = format!(
            "# Metadatos e índice local de ROMSIDIAN (no sincronizar)\n{}\n",
            entry_slash
        );
        fs::write(&gitignore_path, default_content.as_bytes())?;
    }

    Ok(())
}

fn has_extension_ascii_case(path: &Path, expected_ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.eq_ignore_ascii_case(expected_ext))
        .unwrap_or(false)
}

fn is_markdown_file(path: &Path) -> bool {
    has_extension_ascii_case(path, "md")
}

fn normalize_slashes(input: &str) -> String {
    input.replace('\\', "/")
}

fn extract_stem_from_rel_path(rel_path: &str, ext: &str) -> String {
    let file_name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let lower_suffix = format!(".{}", ext.to_lowercase());
    if file_name.len() >= lower_suffix.len()
        && file_name[file_name.len() - lower_suffix.len()..].eq_ignore_ascii_case(&lower_suffix)
    {
        file_name[..file_name.len() - lower_suffix.len()].to_string()
    } else {
        file_name.to_string()
    }
}

fn extract_title_from_rel_path(rel_path: &str) -> String {
    extract_stem_from_rel_path(rel_path, "md")
}

/// Convierte un timestamp Unix en `(YYYY-MM-DD, HH:MM)` UTC usando el algoritmo civil de Hinnant.
pub(crate) fn format_unix_secs_utc(unix_secs: i64) -> (String, String) {
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    (
        format!("{year:04}-{m:02}-{d:02}"),
        format!("{hour:02}:{minute:02}"),
    )
}

fn current_utc_date_time_strings() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_unix_secs_utc(secs)
}

fn substitute_template_variables(raw: &str, title: &str, date_str: &str, time_str: &str) -> String {
    raw.replace("{{date}}", date_str)
        .replace("{{time}}", time_str)
        .replace("{{title}}", title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    struct TempVaultDir {
        path: PathBuf,
    }

    impl TempVaultDir {
        fn new(prefix: &str) -> Self {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!("romsidian_test_{}_{}_{}", prefix, pid, id));
            if path.exists() {
                let _ = fs::remove_dir_all(&path);
            }
            fs::create_dir_all(&path).expect("Debe crear directorio temporal de prueba");
            Self { path }
        }
    }

    impl Drop for TempVaultDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn vault_open_initializes_romsidian_dir_and_gitignore() {
        let temp = TempVaultDir::new("init");
        let vault = Vault::open(&temp.path).expect("Debe abrir la bóveda temporal");

        assert!(
            vault.metadata_dir().exists() && vault.metadata_dir().is_dir(),
            "Debe haberse creado la carpeta oculta .romsidian/"
        );

        let gitignore_path = temp.path.join(".gitignore");
        assert!(gitignore_path.exists(), "Debe haberse creado .gitignore");
        let gitignore_content = fs::read_to_string(&gitignore_path).unwrap();
        assert!(
            gitignore_content.contains(".romsidian/"),
            ".gitignore debe excluir .romsidian/"
        );

        // Verificar idempotencia al volver a abrir la bóveda
        let _vault_again = Vault::open(&temp.path).expect("Reabrir bóveda debe ser idempotente");
        let gitignore_after = fs::read_to_string(&gitignore_path).unwrap();
        assert_eq!(
            gitignore_content, gitignore_after,
            "Reabrir la bóveda no debe duplicar líneas en .gitignore"
        );
    }

    #[test]
    fn vault_crud_and_utf8_accents_work_correctly() {
        let temp = TempVaultDir::new("crud");
        let vault = Vault::open(&temp.path).unwrap();

        // 1. Crear nota con tildes, eñes, espacios y en subcarpeta
        let created = vault
            .create_note(
                "Economía/Política Monetaria y Diseño",
                "---\ntitle: Política Monetaria\n---\n# Ñandú y Árbol\nEnlace a [[Índice General]].",
            )
            .expect("Debe crear nota con caracteres UTF-8 y subcarpeta");

        assert_eq!(
            created.relative_path,
            "Economía/Política Monetaria y Diseño.md"
        );
        assert_eq!(created.title, "Política Monetaria y Diseño");

        // 2. Intentar crear la misma nota de nuevo debe devolver NoteAlreadyExists
        let dup_err = vault
            .create_note("Economía/Política Monetaria y Diseño.md", "duplicado")
            .unwrap_err();
        assert!(matches!(dup_err, CoreError::NoteAlreadyExists(_)));

        // 3. Leer nota
        let read_doc = vault
            .read_note("Economía/Política Monetaria y Diseño")
            .expect("Debe leer la nota recién creada");
        assert!(read_doc.content.contains("Ñandú y Árbol"));

        // 4. Actualizar nota con save_note
        let updated = vault
            .save_note(
                "Economía/Política Monetaria y Diseño.md",
                "# Contenido Actualizado con Éxito\nNueva línea.",
            )
            .expect("Debe actualizar la nota");
        assert!(updated.content.contains("Actualizado con Éxito"));

        // 5. Verificar que archivos dentro de .romsidian/ son ignorados en list_notes
        fs::write(
            vault.metadata_dir().join("oculta.md"),
            "# No debe aparecer en list_notes",
        )
        .unwrap();

        let notes = vault.list_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].relative_path, "Economía/Política Monetaria y Diseño.md");

        // 6. Resolución de wikilinks por título y por ruta (insensible a mayúsculas)
        let resolved_by_title = vault
            .resolve_wikilink_path("política monetaria y diseño")
            .unwrap();
        assert_eq!(
            resolved_by_title.as_deref(),
            Some("Economía/Política Monetaria y Diseño.md")
        );

        let missing = vault.resolve_wikilink_path("Nota Inexistente").unwrap();
        assert!(missing.is_none());

        // 7. Renombrar nota
        let renamed = vault
            .rename_note(
                "Economía/Política Monetaria y Diseño.md",
                "Economía/Síntesis Macroeconómica.md",
            )
            .expect("Debe renombrar la nota correctamente");
        assert_eq!(renamed.relative_path, "Economía/Síntesis Macroeconómica.md");
        assert_eq!(renamed.title, "Síntesis Macroeconómica");
        assert!(vault
            .read_note("Economía/Política Monetaria y Diseño.md")
            .is_err());

        // 8. Eliminar nota
        vault
            .delete_note("Economía/Síntesis Macroeconómica.md")
            .expect("Debe eliminar la nota");
        assert!(vault.list_notes().unwrap().is_empty());
    }

    #[test]
    fn vault_rejects_unsafe_path_traversal_and_hidden_paths() {
        let temp = TempVaultDir::new("security");
        let vault = Vault::open(&temp.path).unwrap();

        assert!(matches!(
            vault.create_note("../escape.md", "malicioso"),
            Err(CoreError::InvalidNotePath(_))
        ));
        assert!(matches!(
            vault.create_note(".romsidian/secreto.md", "malicioso"),
            Err(CoreError::InvalidNotePath(_))
        ));
        assert!(matches!(
            vault.create_note("", "vacío"),
            Err(CoreError::InvalidNotePath(_))
        ));
    }

    #[test]
    fn vault_reads_workspace_test_vault_and_builds_file_tree() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let test_vault_dir = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .join("test-vault");

        let real_vault = Vault::open(&test_vault_dir).expect("Debe abrir test-vault/");
        let _real_notes = real_vault.list_notes().expect("Debe listar notas de test-vault/");
        let _real_tree = real_vault.file_tree().expect("Debe construir el árbol real de test-vault/");

        let temp = TempVaultDir::new("seed_tree");
        crate::tests::copy_non_hidden_dir(&test_vault_dir, &temp.path);
        crate::tests::ensure_canonical_seed_notes(&temp.path);

        let vault = Vault::open(&temp.path).expect("Debe abrir bóveda temporal poblada");
        let notes = vault.list_notes().expect("Debe listar las notas semilla");
        assert!(notes.len() >= 5);

        let tree = vault.file_tree().expect("Debe construir el árbol jerárquico");
        // En la raíz deben existir la carpeta "subcarpeta" y las notas semilla
        assert!(tree.len() >= 5);
        assert!(tree.iter().any(|node| matches!(node, FileTreeNode::Folder { name, .. } if name == "subcarpeta")));

        // Verificar resolución de wikilink canónico
        let resolved = vault
            .resolve_wikilink_path("Notas de Investigación")
            .unwrap();
        assert_eq!(
            resolved.as_deref(),
            Some("subcarpeta/Notas de Investigación.md")
        );
    }

    #[test]
    fn vault_canvas_crud_and_validation_work() {
        let temp = TempVaultDir::new("canvas");
        let vault = Vault::open(&temp.path).unwrap();

        let created = vault
            .create_canvas("Mapas/Arquitectura", "")
            .expect("Debe crear lienzo vacío válido");
        assert_eq!(created.relative_path, "Mapas/Arquitectura.canvas");
        assert_eq!(created.title, "Arquitectura");
        assert!(created.content.contains("\"nodes\""));

        let updated = vault
            .save_canvas(
                "Mapas/Arquitectura.canvas",
                r#"{"nodes":[{"id":"n1","type":"text","text":"Hola","x":0,"y":0,"width":200,"height":100}],"edges":[]}"#,
            )
            .expect("Debe guardar JSON Canvas válido");
        assert!(updated.content.contains("\"n1\""));

        let invalid = vault.save_canvas("Mapas/Arquitectura.canvas", "{no es json");
        assert!(invalid.is_err());

        let traversal = vault.create_canvas("../malo.canvas", "{}");
        assert!(matches!(traversal, Err(CoreError::InvalidNotePath(_))));

        let list = vault.list_canvases().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].relative_path, "Mapas/Arquitectura.canvas");

        let renamed = vault
            .rename_canvas("Mapas/Arquitectura.canvas", "Mapas/Diseño Final")
            .expect("Debe renombrar lienzo");
        assert_eq!(renamed.relative_path, "Mapas/Diseño Final.canvas");

        vault
            .delete_canvas("Mapas/Diseño Final.canvas")
            .expect("Debe eliminar lienzo");
        assert!(vault.list_canvases().unwrap().is_empty());
    }

    #[test]
    fn vault_daily_notes_and_templates_substitute_variables() {
        let temp = TempVaultDir::new("daily");
        let vault = Vault::open(&temp.path).unwrap();

        assert!(vault.list_templates(None).unwrap().is_empty());

        vault
            .create_note(
                "templates/daily.md",
                "---\nfecha: {{date}}\n---\n# Diario {{title}} ({{time}})\n- [ ] Revisar pendientes\n",
            )
            .unwrap();

        let templates = vault.list_templates(None).unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "daily");

        let first = vault
            .get_or_create_daily_note(Some("2026-11-12"), Some("09:30"), None, None)
            .expect("Debe crear la nota diaria aplicando plantilla");
        assert!(first.created);
        assert_eq!(first.note.relative_path, "daily/2026-11-12.md");
        assert!(first.note.content.contains("fecha: 2026-11-12"));
        assert!(first.note.content.contains("# Diario 2026-11-12 (09:30)"));

        let second = vault
            .get_or_create_daily_note(Some("2026-11-12"), Some("10:00"), None, None)
            .expect("Debe abrir nota existente sin sobrescribir");
        assert!(!second.created);
        assert!(second.note.content.contains("(09:30)"));
    }
}
