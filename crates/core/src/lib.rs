//! Núcleo de dominio de ROMSIDIAN (`romsidian-core`).
//!
//! Provee las estructuras base y módulos para gestión de bóvedas Markdown (`.md`),
//! extracción de `[[wikilinks]]`, `#etiquetas` y `frontmatter`, e indexación SQLite FTS5.

pub mod ai_linker;
pub mod dataview;
pub mod index;
pub mod parser;
pub mod vault;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use ai_linker::{
    apply_ai_enrichment, build_deterministic_enrichment_plan, build_vault_knowledge_map,
    generate_knowledge_canvas, preview_ai_enrichment, suggest_note_connections,
    AiEnrichmentApplyResult, AiEnrichmentPlan, AiEnrichmentPreview, CandidateConnection,
    ConnectionSignal, FrontmatterKeyStat, KnowledgeCanvasRequest, KnowledgeCanvasResult,
    KnowledgeNoteEntry, RelatedConnectionLink, VaultKnowledgeMap, WikilinkReplacement,
};

pub use dataview::{
    eval_dataview_inline, eval_dataview_inline_batch, execute_dataview_query, parse_dataview_query,
    DataviewError, DataviewInlineEvaluation, DataviewListItem, DataviewQuery, DataviewQueryResult,
    DataviewTaskItem, DataviewValue,
};
pub use index::{
    is_path_excluded, BacklinkHit, GraphData, GraphEdge, GraphNode, IndexStats, IndexedField,
    IndexedTask, SearchResult, TagCount, TransclusionResolution, UnlinkedMentionHit, VaultIndex,
    INDEX_DB_FILENAME,
};
pub use parser::{
    normalize_field_key, normalize_tag, parse_note, BlockRef, FrontmatterValue, HeadingItem,
    InlineField, InlineFieldSyntax, ParsedNote, TaskItem, WikiLink,
};
pub use vault::{
    CanvasDocument, CanvasSummary, DailyNoteResult, FileTreeNode, NoteDocument, NoteSummary,
    TemplateSummary, Vault, DEFAULT_CANVAS_CONTENT,
};

/// Nombre de la carpeta oculta de metadatos e índice local dentro de cada bóveda.
pub const VAULT_METADATA_DIR: &str = ".romsidian";

/// Errores principales del núcleo de ROMSIDIAN.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Error de entrada/salida en la bóveda: {0}")]
    Io(#[from] std::io::Error),

    #[error("Error en la base de datos de índice SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Ruta de bóveda inválida: {0}")]
    InvalidVaultPath(String),

    #[error("Ruta de nota inválida o insegura: {0}")]
    InvalidNotePath(String),

    #[error("La nota no existe: {0}")]
    NoteNotFound(String),

    #[error("La nota ya existe: {0}")]
    NoteAlreadyExists(String),
}

/// Resultado estándar para operaciones de `romsidian-core`.
pub type CoreResult<T> = Result<T, CoreError>;

/// Identificador y metadatos básicos de una bóveda abierta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultInfo {
    pub name: String,
    pub root_path: String,
    pub metadata_dir: String,
}

impl VaultInfo {
    /// Construye la información descriptiva de una bóveda a partir de su nombre y ruta raíz.
    pub fn new(name: impl Into<String>, root_path: impl Into<String>) -> Self {
        let root = root_path.into();
        let metadata_dir = format!("{}/{}", root.trim_end_matches(['/', '\\']), VAULT_METADATA_DIR);
        Self {
            name: name.into(),
            root_path: root,
            metadata_dir,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    pub(crate) fn ensure_canonical_seed_notes(vault_dir: &Path) {
        let seed_notes = [
            (
                "Índice General.md",
                "---\ntitle: \"Índice General de la Bóveda\"\ntags:\n  - indice\n  - organizacion/boveda\n  - inicio\n---\n\n# Índice General de la Bóveda\n\nMapa principal de temas en ROMSIDIAN:\n- [[Análisis Macroeconómico]]\n- [[Arquitectura de RomSidian]]\n- [[Sincronización Móvil y Git]]\n- [[Notas de Investigación]]\n",
            ),
            (
                "Análisis Macroeconómico.md",
                "---\ntitle: \"Análisis Macroeconómico\"\ntags:\n  - economía\n  - economía/argentina\n---\n\n# Análisis Macroeconómico\n\nSeguimiento de indicadores clave como la inflación mensual, política monetaria y tipo de cambio real.\nVolver al [[Índice General]].\n",
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
                    fs::create_dir_all(parent).expect("Debe crear subcarpeta semilla");
                }
                fs::write(&target, content.as_bytes())
                    .expect("Debe escribir nota semilla canónica en directorio temporal");
            }
        }
    }

    pub(crate) fn copy_non_hidden_dir(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).expect("Debe crear carpeta temporal de prueba");
        for entry in fs::read_dir(src).expect("Debe leer carpeta origen").flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let target = dst.join(name);
            if path.is_dir() {
                copy_non_hidden_dir(&path, &target);
            } else {
                fs::copy(&path, &target).expect("Debe copiar archivo a directorio temporal");
            }
        }
    }

    #[test]
    fn vault_info_builds_metadata_dir_correctly() {
        let info = VaultInfo::new("Mi Bóveda", "C:/Notas/Mi Bóveda");
        assert_eq!(info.name, "Mi Bóveda");
        assert_eq!(info.metadata_dir, "C:/Notas/Mi Bóveda/.romsidian");
    }

    #[test]
    fn test_vault_has_five_interconnected_utf8_notes() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let test_vault_dir = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("Debe existir la raíz del workspace")
            .join("test-vault");

        assert!(
            test_vault_dir.exists() && test_vault_dir.is_dir(),
            "La carpeta test-vault/ debe existir en la raíz del proyecto"
        );

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Reloj del sistema válido")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "romsidian_lib_seed_{}_{}",
            std::process::id(),
            unique
        ));
        copy_non_hidden_dir(&test_vault_dir, &temp_dir);
        ensure_canonical_seed_notes(&temp_dir);

        let mut md_files = Vec::new();
        collect_md_files(&temp_dir, &mut md_files);

        let canonical_notes = [
            "Índice General.md",
            "Análisis Macroeconómico.md",
            "Arquitectura de RomSidian.md",
            "Sincronización Móvil y Git.md",
            "subcarpeta/Notas de Investigación.md",
        ];

        assert!(
            md_files.len() >= canonical_notes.len(),
            "La bóveda debe contener al menos las 5 notas .md semilla de prueba"
        );

        for rel in canonical_notes {
            let path = temp_dir.join(rel);
            let content = fs::read_to_string(&path)
                .expect("Toda nota semilla debe ser legible en UTF-8 válido");
            assert!(
                content.starts_with("---"),
                "La nota {:?} debe incluir bloque YAML frontmatter",
                path.file_name()
            );
            assert!(
                content.contains("[["),
                "La nota {:?} debe contener al menos un [[wikilink]]",
                path.file_name()
            );
            assert!(
                content.contains('#'),
                "La nota {:?} debe contener etiquetas o encabezados Markdown",
                path.file_name()
            );
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    fn collect_md_files(dir: &PathBuf, acc: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !name.starts_with('.') {
                        collect_md_files(&path, acc);
                    }
                } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    acc.push(path);
                }
            }
        }
    }
}
