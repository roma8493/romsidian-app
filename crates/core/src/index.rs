//! Índice local SQLite FTS5, resolución de backlinks y filtrado por etiquetas (`[RF-04]`, `[RF-05]`).
//!
//! Mantiene `.romsidian/index.db` sincronizado de forma incremental usando hashes SHA-256
//! de cada nota `.md`. Si el archivo SQLite no existe, se borra o se corrompe, se reconstruye
//! íntegramente a partir de los archivos `.md` de la bóveda (única fuente de la verdad).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::parser::{
    normalize_field_key, normalize_tag, parse_note, BlockRef, FrontmatterValue, HeadingItem,
};
use crate::vault::{NoteDocument, NoteSummary, Vault};
use crate::CoreResult;

/// Nombre del archivo SQLite dentro de `.romsidian/`.
pub const INDEX_DB_FILENAME: &str = "index.db";

/// Estadísticas devueltas tras sincronizar el índice SQLite con la bóveda en disco.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStats {
    /// Cantidad total de notas `.md` presentes en la bóveda.
    pub total_notes: usize,
    /// Notas nuevas o modificadas que fueron parseadas e indexadas.
    pub indexed_or_updated: usize,
    /// Notas cuyo hash SHA-256 no cambió y fueron omitidas.
    pub skipped_unchanged: usize,
    /// Notas que habían sido indexadas antes pero ya no existen en disco.
    pub removed_deleted: usize,
}

/// Resultado individual de una búsqueda de texto completo (FTS5) o por etiquetas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Ruta relativa de la nota dentro de la bóveda (con barras `/`).
    pub path: String,
    /// Título efectivo de la nota.
    pub title: String,
    /// Fragmento de contexto con coincidencias resaltadas (`<mark>...</mark>`).
    pub snippet: String,
    /// Puntuación de relevancia (menor valor BM25 o mayor relevancia normalizada).
    pub score: f64,
    /// Etiquetas normalizadas asociadas a la nota.
    pub tags: Vec<String>,
}

/// Conteo de uso de una etiqueta en toda la bóveda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagCount {
    /// Nombre normalizado de la etiqueta (sin `#`, en minúsculas).
    pub tag: String,
    /// Cantidad de notas que contienen esta etiqueta.
    pub count: usize,
}

/// Enlace entrante (*backlink*) desde otra nota hacia la nota consultada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklinkHit {
    /// Ruta relativa de la nota origen que contiene el `[[wikilink]]`.
    pub source_path: String,
    /// Título de la nota origen.
    pub source_title: String,
    /// Destino crudo escrito dentro de `[[...]]`.
    pub target_raw: String,
    /// Texto visible alternativo (`[[destino|alias]]`), si existe.
    pub alias: Option<String>,
    /// Sub-encabezado (`[[destino#sección]]`), si existe.
    pub heading: Option<String>,
    /// Número de línea (1-based) donde aparece el enlace en la nota origen.
    pub line: usize,
    /// Línea de texto de contexto donde se ubica el enlace.
    pub context: String,
    /// Indica si fue una incrustación (`![[recurso]]`).
    pub is_embed: bool,
}

/// Campo indexado de una nota (proveniente tanto de `frontmatter` YAML como de campos inline Dataview).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedField {
    /// Ruta relativa de la nota.
    pub path: String,
    /// Clave limpia original.
    pub key: String,
    /// Clave normalizada para consultas Dataview (`key_norm`).
    pub key_norm: String,
    /// Valor textual del campo.
    pub value: String,
    /// Origen del campo (`"frontmatter"`, `"bare"`, `"bracket"`, `"paren"`).
    pub source: String,
    /// Línea (1-based) dentro de la nota, o `0` si proviene del `frontmatter`.
    pub line: usize,
}

/// Tarea Markdown indexada en SQLite (`- [ ]` / `- [x]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedTask {
    /// Ruta relativa de la nota donde se encuentra la tarea.
    pub path: String,
    /// Título efectivo de la nota.
    pub note_title: String,
    /// Número de línea (1-based) en la nota.
    pub line: usize,
    /// Indica si la tarea está completada (`[x]`).
    pub completed: bool,
    /// Texto limpio de la tarea.
    pub text: String,
    /// Etiquetas normalizadas presentes en la tarea.
    pub tags: Vec<String>,
}

/// Mención no enlazada (*Unlinked Mention*) del título o alias de una nota en otra nota de la bóveda (`[RF-11]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlinkedMentionHit {
    /// Ruta relativa de la nota donde aparece el texto sin enlazar.
    pub source_path: String,
    /// Título efectivo de la nota origen.
    pub source_title: String,
    /// Fragmento exacto que coincidió con el título o alias de la nota destino.
    pub matched_term: String,
    /// Número de línea (1-based) en el archivo original.
    pub line: usize,
    /// Byte inicial (0-based) de la coincidencia dentro de `context`.
    pub column_start: usize,
    /// Byte final (0-based) de la coincidencia dentro de `context`.
    pub column_end: usize,
    /// Línea de contexto recortada donde se halló la mención.
    pub context: String,
}

/// Resultado de resolver una transclusión `![[Nota]]`, `![[Nota#Encabezado]]` o `![[Nota#^bloque]]` (`[RF-10]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransclusionResolution {
    /// Ruta relativa canónica de la nota destino en la bóveda.
    pub resolved_path: String,
    /// Título efectivo de la nota destino.
    pub title: String,
    /// Sub-objetivo solicitado (`#Encabezado` o `#^bloque`), si aplica.
    pub subtarget: Option<String>,
    /// Contenido Markdown extraído listo para incrustar.
    pub content: String,
}

/// Mapa en memoria para resolver `[[wikilinks]]` en O(1) durante la indexación.
#[derive(Default)]
struct LinkResolverMap {
    /// Mapa `ruta_sin_extension_en_minusculas` -> `ruta_relativa_canonica`.
    by_path_no_ext: HashMap<String, String>,
    /// Mapa `nombre_archivo_sin_extension_en_minusculas` -> `ruta_relativa_canonica`.
    by_stem: HashMap<String, String>,
}

impl LinkResolverMap {
    fn from_summaries(summaries: &[NoteSummary]) -> Self {
        let mut by_path_no_ext = HashMap::with_capacity(summaries.len());
        let mut by_stem = HashMap::with_capacity(summaries.len());

        for note in summaries {
            let (path_no_ext, stem) = path_lookup_keys(&note.relative_path);
            by_path_no_ext.insert(path_no_ext, note.relative_path.clone());
            // Si hay colisión de nombre base en distintas subcarpetas, preservamos la primera
            // en orden lexicográfico (que coincide con el orden de list_notes()).
            by_stem
                .entry(stem)
                .or_insert_with(|| note.relative_path.clone());
        }

        Self {
            by_path_no_ext,
            by_stem,
        }
    }

    fn resolve(&self, raw_target: &str) -> Option<String> {
        let norm = normalize_link_target(raw_target)?;
        if let Some(found) = self.by_path_no_ext.get(&norm) {
            return Some(found.clone());
        }
        self.by_stem.get(&norm).cloned()
    }
}

/// Gestiona la conexión e índice SQLite FTS5 de una bóveda en `.romsidian/index.db`.
pub struct VaultIndex {
    db_path: PathBuf,
    pub(crate) conn: Connection,
}

impl VaultIndex {
    /// Abre o inicializa la base de datos `.romsidian/index.db` de la bóveda.
    ///
    /// Si el archivo SQLite está corrupto o no es una base de datos válida, lo elimina
    /// y recrea automáticamente el esquema limpio.
    pub fn open(vault: &Vault) -> CoreResult<Self> {
        fs::create_dir_all(vault.metadata_dir())?;
        let db_path = vault.metadata_dir().join(INDEX_DB_FILENAME);
        let conn = Self::open_or_recreate_connection(&db_path)?;
        Ok(Self { db_path, conn })
    }

    /// Abre el índice SQLite y ejecuta inmediatamente la sincronización incremental con la bóveda.
    pub fn open_and_sync(vault: &Vault) -> CoreResult<(Self, IndexStats)> {
        let mut index = Self::open(vault)?;
        let stats = index.sync_vault(vault)?;
        Ok((index, stats))
    }

    /// Devuelve la ruta absoluta al archivo `.romsidian/index.db`.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Abre la conexión SQLite y verifica/crea el esquema. Si detecta corrupción, recrea el archivo.
    fn open_or_recreate_connection(db_path: &Path) -> CoreResult<Connection> {
        match Self::try_open_and_init(db_path) {
            Ok(conn) => Ok(conn),
            Err(_) => {
                Self::remove_db_files(db_path);
                Self::try_open_and_init(db_path)
            }
        }
    }

    fn try_open_and_init(db_path: &Path) -> CoreResult<Connection> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        Self::init_schema(&conn)?;
        Ok(conn)
    }

    fn remove_db_files(db_path: &Path) {
        let _ = fs::remove_file(db_path);
        let wal_path = db_path.with_extension("db-wal");
        let shm_path = db_path.with_extension("db-shm");
        let _ = fs::remove_file(wal_path);
        let _ = fs::remove_file(shm_path);
    }

    fn init_schema(conn: &Connection) -> CoreResult<()> {
        let user_version: i64 =
            conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if user_version < 2 {
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS note_links;
                DROP TABLE IF EXISTS note_tags;
                DROP TABLE IF EXISTS note_fields;
                DROP TABLE IF EXISTS note_tasks;
                DROP TABLE IF EXISTS note_headings;
                DROP TABLE IF EXISTS note_blocks;
                DROP TABLE IF EXISTS notes_fts;
                DROP TABLE IF EXISTS notes;
                ",
            )?;
        }

        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA user_version = 2;

            CREATE TABLE IF NOT EXISTS notes (
                path TEXT PRIMARY KEY,
                stem_lower TEXT NOT NULL,
                path_no_ext_lower TEXT NOT NULL,
                title TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                modified_epoch_ms INTEGER NOT NULL,
                frontmatter_json TEXT NOT NULL,
                body_start_line INTEGER NOT NULL DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_notes_stem_lower ON notes(stem_lower);
            CREATE INDEX IF NOT EXISTS idx_notes_path_no_ext_lower ON notes(path_no_ext_lower);

            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                path UNINDEXED,
                title,
                body,
                tags,
                tokenize = 'unicode61 remove_diacritics 2'
            );

            CREATE TABLE IF NOT EXISTS note_tags (
                path TEXT NOT NULL,
                tag TEXT NOT NULL,
                tag_folded TEXT NOT NULL,
                PRIMARY KEY (path, tag),
                FOREIGN KEY (path) REFERENCES notes(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_note_tags_tag ON note_tags(tag);
            CREATE INDEX IF NOT EXISTS idx_note_tags_tag_folded ON note_tags(tag_folded);

            CREATE TABLE IF NOT EXISTS note_links (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_path TEXT NOT NULL,
                target_raw TEXT NOT NULL,
                target_norm_lower TEXT NOT NULL,
                target_resolved_path TEXT,
                heading TEXT,
                block_id TEXT,
                alias TEXT,
                line INTEGER NOT NULL,
                context TEXT NOT NULL,
                is_embed INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (source_path) REFERENCES notes(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_note_links_source ON note_links(source_path);
            CREATE INDEX IF NOT EXISTS idx_note_links_resolved ON note_links(target_resolved_path);
            CREATE INDEX IF NOT EXISTS idx_note_links_target_norm ON note_links(target_norm_lower);

            CREATE TABLE IF NOT EXISTS note_fields (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                key TEXT NOT NULL,
                key_norm TEXT NOT NULL,
                value TEXT NOT NULL,
                source TEXT NOT NULL,
                line INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (path) REFERENCES notes(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_note_fields_path ON note_fields(path);
            CREATE INDEX IF NOT EXISTS idx_note_fields_key_norm ON note_fields(key_norm);

            CREATE TABLE IF NOT EXISTS note_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                line INTEGER NOT NULL,
                completed INTEGER NOT NULL DEFAULT 0,
                text TEXT NOT NULL,
                tags_json TEXT NOT NULL DEFAULT '[]',
                FOREIGN KEY (path) REFERENCES notes(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_note_tasks_path ON note_tasks(path);

            CREATE TABLE IF NOT EXISTS note_headings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                level INTEGER NOT NULL,
                text TEXT NOT NULL,
                text_lower TEXT NOT NULL,
                line INTEGER NOT NULL,
                FOREIGN KEY (path) REFERENCES notes(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_note_headings_path ON note_headings(path);

            CREATE TABLE IF NOT EXISTS note_blocks (
                path TEXT NOT NULL,
                block_id TEXT NOT NULL,
                block_id_lower TEXT NOT NULL,
                line INTEGER NOT NULL,
                text TEXT NOT NULL,
                PRIMARY KEY (path, block_id_lower),
                FOREIGN KEY (path) REFERENCES notes(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_note_blocks_path ON note_blocks(path);
            ",
        )?;

        // Verifica que la tabla principal y FTS5 sean consultables (detecta corrupción latente).
        let _: i64 = conn.query_row("SELECT count(*) FROM notes", [], |row| row.get(0))?;
        Ok(())
    }

    /// Si `.romsidian/index.db` fue eliminado o corrompido en disco mientras la instancia estaba abierta,
    /// reabre y reconstruye el esquema antes de sincronizar.
    fn ensure_healthy_connection(&mut self) -> CoreResult<()> {
        let needs_reopen = !self.db_path.exists()
            || self
                .conn
                .query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, i64>(0))
                .is_err();

        if needs_reopen {
            // Reemplazamos la conexión previa por una en memoria temporalmente para liberar
            // los descriptores de archivo en Windows antes de borrar/recrear index.db.
            if let Ok(mem_conn) = Connection::open_in_memory() {
                self.conn = mem_conn;
            }
            Self::remove_db_files(&self.db_path);
            self.conn = Self::open_or_recreate_connection(&self.db_path)?;
        }
        Ok(())
    }

    /// Sincroniza incrementalmente todas las notas de la bóveda contra `.romsidian/index.db`.
    ///
    /// - Calcula el hash SHA-256 de cada nota (optimizando el camino rápido por `modified_epoch_ms` + hash).
    /// - Omite las notas cuyo contenido no haya cambiado.
    /// - Actualiza en una sola transacción SQLite las notas nuevas o modificadas.
    /// - Elimina del índice las notas que hayan sido borradas del disco.
    pub fn sync_vault(&mut self, vault: &Vault) -> CoreResult<IndexStats> {
        self.ensure_healthy_connection()?;

        let summaries = vault.list_notes()?;
        let resolver = LinkResolverMap::from_summaries(&summaries);

        // Cargamos el estado actual del índice: path -> (content_hash, modified_epoch_ms)
        let mut existing: HashMap<String, (String, u64)> = HashMap::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT path, content_hash, modified_epoch_ms FROM notes")?;
            let rows = stmt.query_map([], |row| {
                let path: String = row.get(0)?;
                let hash: String = row.get(1)?;
                let mtime: i64 = row.get(2)?;
                Ok((path, (hash, mtime.max(0) as u64)))
            })?;
            for row in rows {
                let (path, meta) = row?;
                existing.insert(path, meta);
            }
        }

        let mut stats = IndexStats {
            total_notes: summaries.len(),
            ..Default::default()
        };

        let mut seen_paths = HashSet::with_capacity(summaries.len());
        let tx = self.conn.transaction()?;

        for summary in &summaries {
            seen_paths.insert(summary.relative_path.clone());

            let full_path = vault.root_path().join(&summary.relative_path);
            let content = fs::read_to_string(&full_path)?;
            let content_hash = compute_sha256(&content);

            if let Some((old_hash, old_mtime)) = existing.get(&summary.relative_path) {
                if old_hash == &content_hash {
                    if *old_mtime != summary.modified_unix_secs {
                        tx.execute(
                            "UPDATE notes SET modified_epoch_ms = ?1 WHERE path = ?2",
                            params![summary.modified_unix_secs as i64, summary.relative_path],
                        )?;
                    }
                    stats.skipped_unchanged += 1;
                    continue;
                }
            }

            Self::upsert_note_in_tx(
                &tx,
                &summary.relative_path,
                &summary.title,
                &content,
                &content_hash,
                summary.modified_unix_secs,
                &resolver,
            )?;
            stats.indexed_or_updated += 1;
        }

        for old_path in existing.keys() {
            if !seen_paths.contains(old_path) {
                Self::delete_note_in_tx(&tx, old_path)?;
                stats.removed_deleted += 1;
            }
        }

        if stats.indexed_or_updated > 0 || stats.removed_deleted > 0 {
            Self::refresh_resolved_links_in_tx(&tx)?;
        }

        tx.commit()?;
        Ok(stats)
    }

    /// Indexa o actualiza incrementalmente una única nota de la bóveda.
    ///
    /// Devuelve `Ok(true)` si la nota fue re-indexada por ser nueva o haber cambiado su SHA-256,
    /// o `Ok(false)` si su contenido era idéntico al ya indexado.
    pub fn index_note(&mut self, vault: &Vault, relative_path: &str) -> CoreResult<bool> {
        self.ensure_healthy_connection()?;

        let doc = vault.read_note(relative_path)?;
        let content_hash = compute_sha256(&doc.content);

        let existing_hash: Option<String> = self
            .conn
            .query_row(
                "SELECT content_hash FROM notes WHERE path = ?1",
                params![doc.relative_path],
                |row| row.get(0),
            )
            .optional()?;

        if existing_hash.as_deref() == Some(content_hash.as_str()) {
            return Ok(false);
        }

        let summaries = vault.list_notes()?;
        let resolver = LinkResolverMap::from_summaries(&summaries);

        let tx = self.conn.transaction()?;
        Self::upsert_note_in_tx(
            &tx,
            &doc.relative_path,
            &doc.title,
            &doc.content,
            &content_hash,
            doc.modified_unix_secs,
            &resolver,
        )?;
        Self::refresh_resolved_links_in_tx(&tx)?;
        tx.commit()?;

        Ok(true)
    }

    /// Elimina una nota del índice SQLite (por ejemplo, tras borrarla del disco).
    ///
    /// Devuelve `Ok(true)` si la nota estaba en el índice y fue eliminada.
    pub fn remove_note(&mut self, relative_path: &str) -> CoreResult<bool> {
        self.ensure_healthy_connection()?;

        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/');

        let tx = self.conn.transaction()?;
        let deleted = Self::delete_note_in_tx(&tx, norm_path)?;
        if deleted {
            Self::refresh_resolved_links_in_tx(&tx)?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    fn upsert_note_in_tx(
        tx: &Transaction<'_>,
        relative_path: &str,
        fallback_title: &str,
        content: &str,
        content_hash: &str,
        modified_unix_secs: u64,
        resolver: &LinkResolverMap,
    ) -> CoreResult<()> {
        let parsed = parse_note(content);
        let effective_title = parsed.title.as_deref().unwrap_or(fallback_title);
        let (path_no_ext_lower, stem_lower) = path_lookup_keys(relative_path);
        let frontmatter_json =
            serde_json::to_string(&parsed.frontmatter).unwrap_or_else(|_| "{}".to_string());

        tx.execute(
            "
            INSERT INTO notes (
                path, stem_lower, path_no_ext_lower, title,
                content_hash, modified_epoch_ms, frontmatter_json, body_start_line
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(path) DO UPDATE SET
                stem_lower = excluded.stem_lower,
                path_no_ext_lower = excluded.path_no_ext_lower,
                title = excluded.title,
                content_hash = excluded.content_hash,
                modified_epoch_ms = excluded.modified_epoch_ms,
                frontmatter_json = excluded.frontmatter_json,
                body_start_line = excluded.body_start_line
            ",
            params![
                relative_path,
                stem_lower,
                path_no_ext_lower,
                effective_title,
                content_hash,
                modified_unix_secs as i64,
                frontmatter_json,
                parsed.body_start_line as i64,
            ],
        )?;

        // Reemplazar entrada en FTS5
        tx.execute(
            "DELETE FROM notes_fts WHERE path = ?1",
            params![relative_path],
        )?;
        let tags_joined = parsed.tags.join(" ");
        tx.execute(
            "INSERT INTO notes_fts (path, title, body, tags) VALUES (?1, ?2, ?3, ?4)",
            params![relative_path, effective_title, parsed.body, tags_joined],
        )?;

        // Reemplazar etiquetas de la nota
        tx.execute(
            "DELETE FROM note_tags WHERE path = ?1",
            params![relative_path],
        )?;
        if !parsed.tags.is_empty() {
            let mut tag_stmt = tx.prepare_cached(
                "INSERT INTO note_tags (path, tag, tag_folded) VALUES (?1, ?2, ?3)",
            )?;
            for tag in &parsed.tags {
                let folded = fold_diacritics(tag);
                tag_stmt.execute(params![relative_path, tag, folded])?;
            }
        }

        // Reemplazar wikilinks salientes de la nota
        tx.execute(
            "DELETE FROM note_links WHERE source_path = ?1",
            params![relative_path],
        )?;
        if !parsed.wikilinks.is_empty() {
            let mut link_stmt = tx.prepare_cached(
                "
                INSERT INTO note_links (
                    source_path, target_raw, target_norm_lower, target_resolved_path,
                    heading, block_id, alias, line, context, is_embed
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ",
            )?;
            for link in &parsed.wikilinks {
                let target_norm =
                    normalize_link_target(&link.target).unwrap_or_else(|| link.target.to_lowercase());
                let resolved = resolver.resolve(&link.target);
                link_stmt.execute(params![
                    relative_path,
                    link.target,
                    target_norm,
                    resolved,
                    link.heading,
                    link.block_id,
                    link.alias,
                    link.line as i64,
                    link.context,
                    if link.is_embed { 1i64 } else { 0i64 },
                ])?;
            }
        }

        // Reemplazar campos indexados (frontmatter + campos inline Dataview)
        tx.execute(
            "DELETE FROM note_fields WHERE path = ?1",
            params![relative_path],
        )?;
        if !parsed.frontmatter.is_empty() || !parsed.inline_fields.is_empty() {
            let mut field_stmt = tx.prepare_cached(
                "
                INSERT INTO note_fields (path, key, key_norm, value, source, line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
            )?;
            for (k, val) in &parsed.frontmatter {
                let Some(key_norm) = normalize_field_key(k) else {
                    continue;
                };
                match val {
                    FrontmatterValue::Scalar(s) => {
                        field_stmt.execute(params![
                            relative_path,
                            k,
                            key_norm,
                            s,
                            "frontmatter",
                            0i64
                        ])?;
                    }
                    FrontmatterValue::List(items) => {
                        let joined = items.join(", ");
                        field_stmt.execute(params![
                            relative_path,
                            k,
                            key_norm,
                            joined,
                            "frontmatter",
                            0i64
                        ])?;
                    }
                }
            }
            for field in &parsed.inline_fields {
                field_stmt.execute(params![
                    relative_path,
                    field.key,
                    field.key_norm,
                    field.value,
                    field.syntax.as_str(),
                    field.line as i64
                ])?;
            }
        }

        // Reemplazar tareas Markdown de la nota
        tx.execute(
            "DELETE FROM note_tasks WHERE path = ?1",
            params![relative_path],
        )?;
        if !parsed.tasks.is_empty() {
            let mut task_stmt = tx.prepare_cached(
                "
                INSERT INTO note_tasks (path, line, completed, text, tags_json)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
            )?;
            for task in &parsed.tasks {
                let tags_json =
                    serde_json::to_string(&task.tags).unwrap_or_else(|_| "[]".to_string());
                task_stmt.execute(params![
                    relative_path,
                    task.line as i64,
                    if task.completed { 1i64 } else { 0i64 },
                    task.text,
                    tags_json
                ])?;
            }
        }

        // Reemplazar encabezados de la nota
        tx.execute(
            "DELETE FROM note_headings WHERE path = ?1",
            params![relative_path],
        )?;
        if !parsed.headings.is_empty() {
            let mut head_stmt = tx.prepare_cached(
                "
                INSERT INTO note_headings (path, level, text, text_lower, line)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
            )?;
            for h in &parsed.headings {
                head_stmt.execute(params![
                    relative_path,
                    h.level as i64,
                    h.text,
                    h.text.to_lowercase(),
                    h.line as i64
                ])?;
            }
        }

        // Reemplazar bloques (^id) de la nota
        tx.execute(
            "DELETE FROM note_blocks WHERE path = ?1",
            params![relative_path],
        )?;
        if !parsed.blocks.is_empty() {
            let mut block_stmt = tx.prepare_cached(
                "
                INSERT OR REPLACE INTO note_blocks (path, block_id, block_id_lower, line, text)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
            )?;
            for b in &parsed.blocks {
                block_stmt.execute(params![
                    relative_path,
                    b.id,
                    b.id.to_lowercase(),
                    b.line as i64,
                    b.text
                ])?;
            }
        }

        Ok(())
    }

    fn delete_note_in_tx(tx: &Transaction<'_>, relative_path: &str) -> CoreResult<bool> {
        tx.execute(
            "DELETE FROM notes_fts WHERE path = ?1",
            params![relative_path],
        )?;
        tx.execute(
            "UPDATE note_links SET target_resolved_path = NULL WHERE target_resolved_path = ?1",
            params![relative_path],
        )?;
        let affected = tx.execute("DELETE FROM notes WHERE path = ?1", params![relative_path])?;
        Ok(affected > 0)
    }

    /// Actualiza en lote `target_resolved_path` para enlazar `[[wikilinks]]` hacia notas creadas
    /// después de la nota que las referencia.
    fn refresh_resolved_links_in_tx(tx: &Transaction<'_>) -> CoreResult<()> {
        tx.execute(
            "
            UPDATE note_links
            SET target_resolved_path = COALESCE(
                (SELECT n.path FROM notes n WHERE n.path_no_ext_lower = note_links.target_norm_lower ORDER BY n.path ASC LIMIT 1),
                (SELECT n.path FROM notes n WHERE n.stem_lower = note_links.target_norm_lower ORDER BY n.path ASC LIMIT 1)
            )
            ",
            [],
        )?;
        Ok(())
    }

    /// Ejecuta una búsqueda instantánea de texto completo sobre título, contenido y etiquetas (`[RF-05]`).
    ///
    /// Soporta términos con o sin tildes (`unicode61 remove_diacritics 2`), múltiples palabras
    /// y filtros por `#etiqueta` combinados o individuales.
    pub fn search(&self, query: &str, limit: usize) -> CoreResult<Vec<SearchResult>> {
        let max_results = limit.max(1);
        let (fts_query, tag_filters) = parse_search_query(query);

        if fts_query.is_none() && tag_filters.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        if let Some(fts_match) = fts_query {
            let mut stmt = self.conn.prepare(
                "
                SELECT
                    path,
                    title,
                    snippet(notes_fts, 2, '<mark>', '</mark>', '…', 16) AS snip,
                    bm25(notes_fts, 10.0, 1.0, 5.0) AS rank_score
                FROM notes_fts
                WHERE notes_fts MATCH ?1
                ORDER BY rank_score ASC, path ASC
                LIMIT ?2
                ",
            )?;

            // Si además hay filtros de #etiqueta, traemos un margen mayor y filtramos.
            let fetch_limit = if tag_filters.is_empty() {
                max_results
            } else {
                max_results.saturating_mul(4).max(50)
            };

            let rows = stmt.query_map(params![fts_match, fetch_limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })?;

            for row in rows {
                let (path, title, snippet, score) = row?;
                let tags = self.tags_for_note(&path)?;
                if matches_all_tag_filters(&tags, &tag_filters) {
                    results.push(SearchResult {
                        path,
                        title,
                        snippet,
                        score,
                        tags,
                    });
                    if results.len() >= max_results {
                        break;
                    }
                }
            }
        } else {
            // Consulta compuesta únicamente por #etiquetas (ej. "#economía")
            let first_tag = &tag_filters[0];
            let candidates = self.notes_by_tag(first_tag)?;
            for note in candidates {
                let tags = self.tags_for_note(&note.relative_path)?;
                if matches_all_tag_filters(&tags, &tag_filters) {
                    results.push(SearchResult {
                        path: note.relative_path,
                        title: note.title,
                        snippet: String::new(),
                        score: 0.0,
                        tags,
                    });
                    if results.len() >= max_results {
                        break;
                    }
                }
            }
        }

        Ok(results)
    }

    /// Devuelve todas las etiquetas de una nota específica ordenadas alfabéticamente.
    pub fn tags_for_note(&self, relative_path: &str) -> CoreResult<Vec<String>> {
        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/');

        let mut stmt = self
            .conn
            .prepare_cached("SELECT tag FROM note_tags WHERE path = ?1 ORDER BY tag ASC")?;
        let rows = stmt.query_map(params![norm_path], |row| row.get::<_, String>(0))?;
        let mut tags = Vec::new();
        for tag in rows {
            tags.push(tag?);
        }
        Ok(tags)
    }

    /// Lista todas las notas que contienen la `#etiqueta` indicada (incluyendo sub-etiquetas
    /// jerárquicas como `#economía/argentina` cuando se consulta `#economía`, con insensibilidad a tildes).
    pub fn notes_by_tag(&self, tag: &str) -> CoreResult<Vec<NoteSummary>> {
        let Some(norm_tag) = normalize_tag(tag) else {
            return Ok(Vec::new());
        };
        let folded_tag = fold_diacritics(&norm_tag);
        let hierarchical_pattern = format!("{folded_tag}/%");

        let mut stmt = self.conn.prepare(
            "
            SELECT DISTINCT n.path, n.title, n.modified_epoch_ms
            FROM notes n
            INNER JOIN note_tags nt ON nt.path = n.path
            WHERE nt.tag_folded = ?1 OR nt.tag_folded LIKE ?2
            ORDER BY n.path ASC
            ",
        )?;

        let rows = stmt.query_map(params![folded_tag, hierarchical_pattern], |row| {
            let relative_path: String = row.get(0)?;
            let title: String = row.get(1)?;
            let mtime: i64 = row.get(2)?;
            Ok(NoteSummary {
                relative_path,
                title,
                modified_unix_secs: mtime.max(0) as u64,
                size_bytes: 0,
            })
        })?;

        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row?);
        }
        Ok(summaries)
    }

    /// Devuelve el listado completo de `#etiquetas` de la bóveda junto con la cantidad de notas asociadas.
    pub fn list_all_tags(&self) -> CoreResult<Vec<TagCount>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT tag, COUNT(DISTINCT path) AS note_count
            FROM note_tags
            GROUP BY tag
            ORDER BY note_count DESC, tag ASC
            ",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(TagCount {
                tag: row.get(0)?,
                count: row.get::<_, i64>(1)?.max(0) as usize,
            })
        })?;

        let mut counts = Vec::new();
        for row in rows {
            counts.push(row?);
        }
        Ok(counts)
    }

    /// Devuelve todos los enlaces entrantes (*backlinks*) que apuntan hacia `relative_path` (`[RF-04]`).
    ///
    /// Resuelve tanto por ruta canónica resuelta (`target_resolved_path`) como por coincidencia
    /// insensible a mayúsculas con el nombre o ruta sin extensión (`target_norm_lower`).
    pub fn backlinks_for(&self, relative_path: &str) -> CoreResult<Vec<BacklinkHit>> {
        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/').to_string();
        let (path_no_ext_lower, stem_lower) = path_lookup_keys(&norm_path);

        let mut stmt = self.conn.prepare(
            "
            SELECT
                nl.source_path,
                n.title AS source_title,
                nl.target_raw,
                nl.alias,
                nl.heading,
                nl.line,
                nl.context,
                nl.is_embed
            FROM note_links nl
            INNER JOIN notes n ON n.path = nl.source_path
            WHERE
                nl.target_resolved_path = ?1
                OR (
                    nl.target_resolved_path IS NULL
                    AND (nl.target_norm_lower = ?2 OR nl.target_norm_lower = ?3)
                )
            ORDER BY nl.source_path ASC, nl.line ASC
            ",
        )?;

        let rows = stmt.query_map(
            params![norm_path, path_no_ext_lower, stem_lower],
            |row| {
                Ok(BacklinkHit {
                    source_path: row.get(0)?,
                    source_title: row.get(1)?,
                    target_raw: row.get(2)?,
                    alias: row.get(3)?,
                    heading: row.get(4)?,
                    line: row.get::<_, i64>(5)?.max(1) as usize,
                    context: row.get(6)?,
                    is_embed: row.get::<_, i64>(7)? != 0,
                })
            },
        )?;

        let mut hits = Vec::new();
        for row in rows {
            hits.push(row?);
        }
        Ok(hits)
    }

    /// Devuelve todos los campos indexados (`frontmatter` + campos inline Dataview) de una nota (`[RF-12]`).
    pub fn fields_for_note(&self, relative_path: &str) -> CoreResult<Vec<IndexedField>> {
        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/');

        let mut stmt = self.conn.prepare_cached(
            "
            SELECT path, key, key_norm, value, source, line
            FROM note_fields
            WHERE path = ?1
            ORDER BY line ASC, id ASC
            ",
        )?;
        let rows = stmt.query_map(params![norm_path], |row| {
            Ok(IndexedField {
                path: row.get(0)?,
                key: row.get(1)?,
                key_norm: row.get(2)?,
                value: row.get(3)?,
                source: row.get(4)?,
                line: row.get::<_, i64>(5)?.max(0) as usize,
            })
        })?;

        let mut fields = Vec::new();
        for row in rows {
            fields.push(row?);
        }
        Ok(fields)
    }

    /// Devuelve todas las tareas Markdown (`- [ ]` / `- [x]`) indexadas en una nota específica (`[RF-12]`).
    pub fn tasks_for_note(&self, relative_path: &str) -> CoreResult<Vec<IndexedTask>> {
        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/');

        let mut stmt = self.conn.prepare_cached(
            "
            SELECT t.path, n.title, t.line, t.completed, t.text, t.tags_json
            FROM note_tasks t
            INNER JOIN notes n ON n.path = t.path
            WHERE t.path = ?1
            ORDER BY t.line ASC
            ",
        )?;
        let rows = stmt.query_map(params![norm_path], |row| {
            let tags_json: String = row.get(5)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            Ok(IndexedTask {
                path: row.get(0)?,
                note_title: row.get(1)?,
                line: row.get::<_, i64>(2)?.max(1) as usize,
                completed: row.get::<_, i64>(3)? != 0,
                text: row.get(4)?,
                tags,
            })
        })?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    /// Lista todas las tareas Markdown de toda la bóveda ordenadas por nota y línea (`[RF-12]`).
    pub fn list_all_tasks(&self) -> CoreResult<Vec<IndexedTask>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT t.path, n.title, t.line, t.completed, t.text, t.tags_json
            FROM note_tasks t
            INNER JOIN notes n ON n.path = t.path
            ORDER BY t.path ASC, t.line ASC
            ",
        )?;
        let rows = stmt.query_map([], |row| {
            let tags_json: String = row.get(5)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            Ok(IndexedTask {
                path: row.get(0)?,
                note_title: row.get(1)?,
                line: row.get::<_, i64>(2)?.max(1) as usize,
                completed: row.get::<_, i64>(3)? != 0,
                text: row.get(4)?,
                tags,
            })
        })?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    /// Devuelve los encabezados Markdown (`#` a `######`) indexados en una nota (`[RF-10]`).
    pub fn headings_for_note(&self, relative_path: &str) -> CoreResult<Vec<HeadingItem>> {
        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/');

        let mut stmt = self.conn.prepare_cached(
            "
            SELECT level, text, line
            FROM note_headings
            WHERE path = ?1
            ORDER BY line ASC
            ",
        )?;
        let rows = stmt.query_map(params![norm_path], |row| {
            Ok(HeadingItem {
                level: row.get::<_, i64>(0)?.clamp(1, 6) as u8,
                text: row.get(1)?,
                line: row.get::<_, i64>(2)?.max(1) as usize,
            })
        })?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// Devuelve los bloques referenciables (`^id`) indexados en una nota (`[RF-10]`).
    pub fn blocks_for_note(&self, relative_path: &str) -> CoreResult<Vec<BlockRef>> {
        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/');

        let mut stmt = self.conn.prepare_cached(
            "
            SELECT block_id, line, text
            FROM note_blocks
            WHERE path = ?1
            ORDER BY line ASC
            ",
        )?;
        let rows = stmt.query_map(params![norm_path], |row| {
            Ok(BlockRef {
                id: row.get(0)?,
                line: row.get::<_, i64>(1)?.max(1) as usize,
                text: row.get(2)?,
            })
        })?;

        let mut blocks = Vec::new();
        for row in rows {
            blocks.push(row?);
        }
        Ok(blocks)
    }

    /// Resuelve el destino de un `[[wikilink]]` o `![[transclusión]]` a la ruta relativa canónica
    /// de la nota dentro de la bóveda (si existe).
    pub fn resolve_note_target(&self, raw_target: &str) -> CoreResult<Option<String>> {
        let Some(norm) = normalize_link_target(raw_target) else {
            return Ok(None);
        };

        let found: Option<String> = self
            .conn
            .query_row(
                "
                SELECT COALESCE(
                    (SELECT path FROM notes WHERE path_no_ext_lower = ?1 ORDER BY path ASC LIMIT 1),
                    (SELECT path FROM notes WHERE stem_lower = ?1 ORDER BY path ASC LIMIT 1)
                )
                ",
                params![norm],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        Ok(found)
    }

    /// Resuelve y extrae el contenido de una transclusión `![[Nota]]`, `![[Nota#Encabezado]]`
    /// o `![[Nota#^id-bloque]]` (`[RF-10]`).
    pub fn resolve_transclusion(
        &self,
        vault: &Vault,
        raw_target: &str,
        subtarget: Option<&str>,
    ) -> CoreResult<Option<TransclusionResolution>> {
        let (target_part, inline_sub) = match raw_target.split_once('#') {
            Some((left, right)) => (left.trim(), Some(right.trim())),
            None => (raw_target.trim(), None),
        };
        let effective_sub = subtarget
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or(inline_sub.filter(|s| !s.is_empty()));

        let Some(resolved_path) = self.resolve_note_target(target_part)? else {
            return Ok(None);
        };

        let doc = vault.read_note(&resolved_path)?;
        let parsed = parse_note(&doc.content);
        let title = parsed.title.clone().unwrap_or(doc.title);

        let extracted = match effective_sub {
            Some(sub) if sub.starts_with('^') => parsed.extract_block(sub),
            Some(sub) => parsed.extract_section(sub),
            None => Some(parsed.body.trim().to_string()),
        };

        let Some(content) = extracted else {
            return Ok(None);
        };

        Ok(Some(TransclusionResolution {
            resolved_path,
            title,
            subtarget: effective_sub.map(|s| s.to_string()),
            content,
        }))
    }

    /// Detecta todas las **Menciones No Enlazadas** (*Unlinked Mentions*) hacia `relative_path`
    /// en el resto de la bóveda, omitiendo las notas que coincidan con `excluded_patterns` (`[RF-11]`).
    pub fn unlinked_mentions_for(
        &self,
        relative_path: &str,
        excluded_patterns: &[String],
    ) -> CoreResult<Vec<UnlinkedMentionHit>> {
        let norm_path = relative_path.replace('\\', "/");
        let norm_path = norm_path.trim_start_matches('/').to_string();
        let (_, stem_lower) = path_lookup_keys(&norm_path);

        let stem_original = Path::new(
            norm_path
                .strip_suffix(".md")
                .or_else(|| norm_path.strip_suffix(".MD"))
                .unwrap_or(&norm_path),
        )
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&norm_path)
        .to_string();

        let mut candidate_terms: Vec<String> = Vec::new();
        push_unique_term(&mut candidate_terms, &stem_original);

        if let Some((title, frontmatter_json)) = self
            .conn
            .query_row(
                "SELECT title, frontmatter_json FROM notes WHERE path = ?1",
                params![norm_path],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            push_unique_term(&mut candidate_terms, &title);
            if let Ok(fm) =
                serde_json::from_str::<BTreeMap<String, FrontmatterValue>>(&frontmatter_json)
            {
                for alias_key in ["aliases", "alias"] {
                    if let Some(val) = fm.get(alias_key) {
                        match val {
                            FrontmatterValue::Scalar(s) => {
                                push_unique_term(&mut candidate_terms, s);
                            }
                            FrontmatterValue::List(items) => {
                                for item in items {
                                    push_unique_term(&mut candidate_terms, item);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Ordenar términos del más largo al más corto para preferir coincidencias completas.
        candidate_terms.sort_by(|a, b| b.len().cmp(&a.len()));
        if candidate_terms.is_empty() {
            return Ok(Vec::new());
        }

        let mut stmt = self.conn.prepare(
            "
            SELECT n.path, n.title, n.body_start_line, f.body
            FROM notes n
            INNER JOIN notes_fts f ON f.path = n.path
            WHERE n.path != ?1 AND n.stem_lower != ?2
            ORDER BY n.path ASC
            ",
        )?;

        let rows = stmt.query_map(params![norm_path, stem_lower], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(1) as usize,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut hits = Vec::new();
        for row in rows {
            let (source_path, source_title, body_start_line, body) = row?;
            if is_path_excluded(&source_path, excluded_patterns) {
                continue;
            }

            let mut in_fenced_code = false;
            let mut fence_char = '`';

            for (idx, raw_line) in body.lines().enumerate() {
                let trimmed = raw_line.trim();
                if trimmed.starts_with("```") {
                    if !in_fenced_code {
                        in_fenced_code = true;
                        fence_char = '`';
                        continue;
                    } else if fence_char == '`' {
                        in_fenced_code = false;
                        continue;
                    }
                } else if trimmed.starts_with("~~~") {
                    if !in_fenced_code {
                        in_fenced_code = true;
                        fence_char = '~';
                        continue;
                    } else if fence_char == '~' {
                        in_fenced_code = false;
                        continue;
                    }
                }

                if in_fenced_code || trimmed.is_empty() {
                    continue;
                }

                let line_number = body_start_line + idx;
                if let Some((matched_slice, col_start, col_end)) =
                    find_unlinked_term_in_line(trimmed, &candidate_terms)
                {
                    hits.push(UnlinkedMentionHit {
                        source_path: source_path.clone(),
                        source_title: source_title.clone(),
                        matched_term: matched_slice.to_string(),
                        line: line_number,
                        column_start: col_start,
                        column_end: col_end,
                        context: trimmed.to_string(),
                    });
                }
            }
        }

        Ok(hits)
    }

    /// Convierte una mención no enlazada en la línea `line` de `source_path` en un `[[wikilink]]`
    /// apuntando a `target_note_path`, guardando el archivo y actualizando el índice SQLite (`[RF-11]`).
    pub fn link_unlinked_mention(
        &mut self,
        vault: &Vault,
        source_path: &str,
        line: usize,
        matched_term: &str,
        target_note_path: &str,
    ) -> CoreResult<NoteDocument> {
        let doc = vault.read_note(source_path)?;
        let norm_target = target_note_path.replace('\\', "/");
        let target_stem = Path::new(
            norm_target
                .trim_start_matches('/')
                .strip_suffix(".md")
                .or_else(|| norm_target.trim_start_matches('/').strip_suffix(".MD"))
                .unwrap_or(norm_target.trim_start_matches('/')),
        )
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(norm_target.trim_start_matches('/'));

        let search_terms = vec![matched_term.trim().to_string()];
        let mut new_content = String::with_capacity(doc.content.len() + target_stem.len() + 8);
        let mut replaced = false;

        for (idx, chunk) in doc.content.split_inclusive('\n').enumerate() {
            let line_no = idx + 1;
            if !replaced && line_no == line {
                let (line_body, newline_suffix) = if let Some(stripped) = chunk.strip_suffix("\r\n")
                {
                    (stripped, "\r\n")
                } else if let Some(stripped) = chunk.strip_suffix('\n') {
                    (stripped, "\n")
                } else {
                    (chunk, "")
                };

                if let Some((found_slice, start_byte, end_byte)) =
                    find_unlinked_term_in_line(line_body, &search_terms)
                {
                    let replacement = if found_slice.to_lowercase() == target_stem.to_lowercase() {
                        format!("[[{target_stem}]]")
                    } else {
                        format!("[[{target_stem}|{found_slice}]]")
                    };
                    new_content.push_str(&line_body[..start_byte]);
                    new_content.push_str(&replacement);
                    new_content.push_str(&line_body[end_byte..]);
                    new_content.push_str(newline_suffix);
                    replaced = true;
                    continue;
                }
            }
            new_content.push_str(chunk);
        }

        let updated_doc = vault.save_note(source_path, &new_content)?;
        self.index_note(vault, source_path)?;
        Ok(updated_doc)
    }
}

/// Añade un término de búsqueda de menciones si tiene al menos 2 caracteres y no está duplicado.
fn push_unique_term(terms: &mut Vec<String>, candidate: &str) {
    let trimmed = candidate.trim();
    if trimmed.chars().count() < 2 {
        return;
    }
    let lower = trimmed.to_lowercase();
    if !terms.iter().any(|t| t.to_lowercase() == lower) {
        terms.push(trimmed.to_string());
    }
}

/// Evalúa si una ruta relativa coincide con alguno de los patrones de `Excluded files`
/// (soporta prefijos de carpeta, subcadenas y comodines `*`, sin distinguir mayúsculas).
pub fn is_path_excluded(relative_path: &str, excluded_patterns: &[String]) -> bool {
    let path_lower = relative_path.replace('\\', "/").to_lowercase();
    for raw_pat in excluded_patterns {
        let pat = raw_pat.trim().replace('\\', "/").to_lowercase();
        if pat.is_empty() {
            continue;
        }
        if wildcard_match(&path_lower, &pat) {
            return true;
        }
        let clean_pat = pat.trim_matches('/');
        if !clean_pat.is_empty()
            && (path_lower.starts_with(&format!("{clean_pat}/")) || path_lower.contains(clean_pat))
        {
            return true;
        }
    }
    false
}

fn wildcard_match(text: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return text == pattern;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0usize;
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if idx == 0 && !pattern.starts_with('*') {
            if !text[pos..].starts_with(part) {
                return false;
            }
            pos += part.len();
        } else if idx == parts.len() - 1 && !pattern.ends_with('*') {
            return text[pos..].ends_with(part);
        } else if let Some(found) = text[pos..].find(part) {
            pos += found + part.len();
        } else {
            return false;
        }
    }
    true
}

/// Busca la primera ocurrencia no enlazada (con límites de palabra completos y fuera de `[[...]]`,
/// código inline `` `...` ``, URLs o enlaces Markdown) de cualquiera de los `terms` en `line`.
pub(crate) fn find_unlinked_term_in_line<'a>(
    line: &'a str,
    terms: &[String],
) -> Option<(&'a str, usize, usize)> {
    let protected_ranges = compute_protected_ranges(line);
    let line_chars: Vec<(usize, char)> = line.char_indices().collect();

    for term in terms {
        let term_chars: Vec<char> = term.chars().collect();
        if term_chars.is_empty() || term_chars.len() > line_chars.len() {
            continue;
        }

        let max_start = line_chars.len() - term_chars.len();
        for start_idx in 0..=max_start {
            let end_idx = start_idx + term_chars.len();
            let start_byte = line_chars[start_idx].0;
            let end_byte = if end_idx < line_chars.len() {
                line_chars[end_idx].0
            } else {
                line.len()
            };

            // Verificar que no solape con [[wikilinks]], código inline ni enlaces Markdown.
            if protected_ranges
                .iter()
                .any(|&(p_start, p_end)| start_byte < p_end && end_byte > p_start)
            {
                continue;
            }

            // Verificar límites de palabra: ni el carácter previo ni el posterior pueden ser alfanuméricos.
            if start_idx > 0 && is_word_boundary_char(line_chars[start_idx - 1].1) {
                continue;
            }
            if end_idx < line_chars.len() && is_word_boundary_char(line_chars[end_idx].1) {
                continue;
            }

            // Comparación insensible a mayúsculas carácter por carácter.
            let matches = line_chars[start_idx..end_idx]
                .iter()
                .zip(term_chars.iter())
                .all(|((_, lc), tc)| lc.to_lowercase().eq(tc.to_lowercase()));

            if matches {
                return Some((&line[start_byte..end_byte], start_byte, end_byte));
            }
        }
    }

    None
}

pub(crate) fn is_word_boundary_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Calcula los rangos de bytes `[start..end]` dentro de una línea que ya forman parte de
/// `[[wikilinks]]`, `![[embeds]]`, código inline `` `...` ``, enlaces `[txt](url)`, URLs o `#tags`.
pub(crate) fn compute_protected_ranges(line: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // 1. Código inline `...`
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            ranges.push((start, i));
            continue;
        }

        // 2. Wikilinks [[...]] o ![[...]]
        if bytes[i] == b'[' && i + 1 < len && bytes[i + 1] == b'[' {
            let start = if i > 0 && bytes[i - 1] == b'!' {
                i - 1
            } else {
                i
            };
            i += 2;
            while i + 1 < len && !(bytes[i] == b']' && bytes[i + 1] == b']') {
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            } else {
                i = len;
            }
            ranges.push((start, i));
            continue;
        }

        // 3. Enlaces Markdown [texto](url)
        if bytes[i] == b'[' {
            let start = i;
            if let Some(rel_close) = line[i + 1..].find("](") {
                let paren_start = i + 1 + rel_close + 2;
                if let Some(rel_paren_end) = line[paren_start..].find(')') {
                    let end = paren_start + rel_paren_end + 1;
                    ranges.push((start, end));
                    i = end;
                    continue;
                }
            }
        }

        // 4. URLs desnudas http:// o https://
        if line.is_char_boundary(i)
            && (line[i..].starts_with("http://") || line[i..].starts_with("https://"))
        {
            let start = i;
            while i < len && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b')' | b'>' | b'"' | b'\'') {
                i += 1;
            }
            ranges.push((start, i));
            continue;
        }

        // 5. Etiquetas #tag
        if bytes[i] == b'#' {
            let start = i;
            i += 1;
            while i < len && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            ranges.push((start, i));
            continue;
        }

        i += 1;
    }

    ranges
}

/// Calcula el digest SHA-256 en hexadecimal minúsculas para el contenido UTF-8 de una nota.
pub fn compute_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Extrae `(path_no_ext_lower, stem_lower)` para indexar y resolver rutas de notas.
pub(crate) fn path_lookup_keys(relative_path: &str) -> (String, String) {
    let normalized = relative_path.replace('\\', "/");
    let normalized = normalized.trim_start_matches('/');
    let without_ext = normalized
        .strip_suffix(".md")
        .or_else(|| normalized.strip_suffix(".MD"))
        .unwrap_or(normalized);
    let path_no_ext_lower = without_ext.to_lowercase();
    let stem_lower = Path::new(without_ext)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(without_ext)
        .to_lowercase();
    (path_no_ext_lower, stem_lower)
}

/// Normaliza el destino de un `[[wikilink]]` para búsqueda en minúsculas sin extensión `.md`.
pub(crate) fn normalize_link_target(raw_target: &str) -> Option<String> {
    let before_pipe = raw_target.split('|').next()?.trim();
    let before_hash = before_pipe.split('#').next()?.trim();
    if before_hash.is_empty() {
        return None;
    }
    let normalized = before_hash.replace('\\', "/");
    let normalized = normalized.trim_matches('/');
    if normalized.is_empty() {
        return None;
    }
    let without_ext = normalized
        .strip_suffix(".md")
        .or_else(|| normalized.strip_suffix(".MD"))
        .unwrap_or(normalized);
    Some(without_ext.to_lowercase())
}

/// Separa una consulta de usuario en una expresión segura para FTS5 `MATCH` y una lista de filtros `#tag`.
fn parse_search_query(raw_query: &str) -> (Option<String>, Vec<String>) {
    let mut fts_terms = Vec::new();
    let mut tag_filters = Vec::new();

    for token in raw_query.split_whitespace() {
        if let Some(rest) = token.strip_prefix('#') {
            if let Some(norm_tag) = normalize_tag(rest) {
                tag_filters.push(norm_tag);
                continue;
            }
        } else if let Some(rest) = token.strip_prefix("tag:") {
            if let Some(norm_tag) = normalize_tag(rest) {
                tag_filters.push(norm_tag);
                continue;
            }
        }

        // Limpiamos caracteres especiales de sintaxis FTS5 conservando letras/números Unicode.
        let clean: String = token
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();

        if !clean.is_empty() {
            fts_terms.push(format!("\"{clean}\"*"));
        }
    }

    let fts_query = if fts_terms.is_empty() {
        None
    } else {
        Some(fts_terms.join(" AND "))
    };

    (fts_query, tag_filters)
}

fn matches_all_tag_filters(note_tags: &[String], required_filters: &[String]) -> bool {
    required_filters.iter().all(|req| {
        let req_folded = fold_diacritics(req);
        let prefix = format!("{req_folded}/");
        note_tags.iter().any(|t| {
            let t_folded = fold_diacritics(t);
            t_folded == req_folded || t_folded.starts_with(&prefix)
        })
    })
}

/// Normaliza vocales acentuadas o con diéresis (`á -> a`, etc.) preservando la `ñ`.
pub(crate) fn fold_diacritics(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            other => other,
        })
        .collect()
}

/// Nodo del grafo de conocimiento 2D (Global o Local).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub tags: Vec<String>,
    pub degree: usize,
    pub folder: String,
    pub exists: bool,
}

/// Arista dirigida entre dos notas en el grafo 2D.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub resolved: bool,
}

/// Grafo completo de enlaces de la bóveda para la vista de grafo 2D.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Registro agregado de una nota en SQLite usado por el motor sandboxed `Dataview`.
#[derive(Debug, Clone)]
pub(crate) struct DataviewPageRecord {
    pub path: String,
    pub title: String,
    pub stem: String,
    pub modified_epoch_ms: i64,
    pub tags: Vec<String>,
    pub outlinks: Vec<String>,
    pub inlinks: Vec<String>,
    pub fields: std::collections::BTreeMap<String, Vec<String>>,
}

impl VaultIndex {
    /// Extrae todos los registros indexados de páginas, etiquetas, campos y enlaces para `Dataview`.
    pub(crate) fn list_dataview_pages(&self) -> CoreResult<Vec<DataviewPageRecord>> {
        use std::collections::{BTreeMap, BTreeSet};

        let mut pages_by_path: BTreeMap<String, DataviewPageRecord> = BTreeMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT path, title, modified_epoch_ms FROM notes ORDER BY LOWER(path) ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for item in rows {
            let (path, title, modified_epoch_ms) = item?;
            let stem = Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&title)
                .to_string();
            pages_by_path.insert(
                path.clone(),
                DataviewPageRecord {
                    path,
                    title,
                    stem,
                    modified_epoch_ms,
                    tags: Vec::new(),
                    outlinks: Vec::new(),
                    inlinks: Vec::new(),
                    fields: BTreeMap::new(),
                },
            );
        }

        let mut tag_stmt = self
            .conn
            .prepare("SELECT path, tag FROM note_tags ORDER BY path, tag")?;
        let tag_rows = tag_stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        for item in tag_rows {
            let (path, tag) = item?;
            if let Some(page) = pages_by_path.get_mut(&path) {
                page.tags.push(tag);
            }
        }

        let mut field_stmt = self
            .conn
            .prepare("SELECT path, key_norm, value FROM note_fields ORDER BY line ASC, rowid ASC")?;
        let field_rows = field_stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for item in field_rows {
            let (path, key_norm, value) = item?;
            if let Some(page) = pages_by_path.get_mut(&path) {
                page.fields.entry(key_norm).or_default().push(value);
            }
        }

        let mut link_stmt = self.conn.prepare(
            "SELECT source_path, target_raw, target_resolved_path FROM note_links ORDER BY line ASC",
        )?;
        let link_rows = link_stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;

        let mut out_sets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut in_sets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for item in link_rows {
            let (source, target_raw, target_resolved) = item?;
            let link_target = target_resolved.clone().unwrap_or(target_raw);
            out_sets.entry(source.clone()).or_default().insert(link_target);
            if let Some(resolved_path) = target_resolved {
                in_sets.entry(resolved_path).or_default().insert(source);
            }
        }

        for (path, page) in &mut pages_by_path {
            if let Some(outs) = out_sets.remove(path) {
                page.outlinks = outs.into_iter().collect();
            }
            if let Some(ins) = in_sets.remove(path) {
                page.inlinks = ins.into_iter().collect();
            }
        }

        Ok(pages_by_path.into_values().collect())
    }

    /// Construye nodos y aristas del grafo de conocimiento 2D desde el índice SQLite.
    pub fn build_graph_data(&self) -> CoreResult<GraphData> {
        use std::collections::{BTreeMap, BTreeSet};

        let pages = self.list_dataview_pages()?;
        let mut degrees: BTreeMap<String, usize> = BTreeMap::new();
        let mut edge_set: BTreeSet<(String, String, bool)> = BTreeSet::new();
        let mut missing_nodes: BTreeMap<String, String> = BTreeMap::new();

        let existing_paths: BTreeSet<String> = pages.iter().map(|p| p.path.clone()).collect();

        let mut link_stmt = self.conn.prepare(
            "SELECT source_path, target_raw, target_resolved_path FROM note_links",
        )?;
        let link_rows = link_stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;

        for item in link_rows {
            let (source, target_raw, target_resolved) = item?;
            if let Some(target_path) = target_resolved {
                if source != target_path && existing_paths.contains(&target_path) {
                    if edge_set.insert((source.clone(), target_path.clone(), true)) {
                        *degrees.entry(source).or_insert(0) += 1;
                        *degrees.entry(target_path).or_insert(0) += 1;
                    }
                }
            } else {
                let clean_raw = target_raw
                    .split('|')
                    .next()
                    .unwrap_or(&target_raw)
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !clean_raw.is_empty() {
                    let ghost_id = format!("__unresolved__:{clean_raw}");
                    missing_nodes
                        .entry(ghost_id.clone())
                        .or_insert_with(|| clean_raw.to_string());
                    if edge_set.insert((source.clone(), ghost_id.clone(), false)) {
                        *degrees.entry(source).or_insert(0) += 1;
                        *degrees.entry(ghost_id).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut nodes: Vec<GraphNode> = pages
            .into_iter()
            .map(|p| {
                let folder = p
                    .path
                    .rsplit_once('/')
                    .map(|(dir, _)| dir.to_string())
                    .unwrap_or_default();
                let degree = *degrees.get(&p.path).unwrap_or(&0);
                GraphNode {
                    id: p.path,
                    label: p.title,
                    tags: p.tags,
                    degree,
                    folder,
                    exists: true,
                }
            })
            .collect();

        for (ghost_id, label) in missing_nodes {
            let degree = *degrees.get(&ghost_id).unwrap_or(&0);
            nodes.push(GraphNode {
                id: ghost_id,
                label,
                tags: Vec::new(),
                degree,
                folder: String::new(),
                exists: false,
            });
        }

        let edges = edge_set
            .into_iter()
            .map(|(source, target, resolved)| GraphEdge {
                source,
                target,
                resolved,
            })
            .collect();

        Ok(GraphData { nodes, edges })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    fn create_temp_vault_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Reloj del sistema válido")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "romsidian_index_test_{}_{}_{}",
            prefix,
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&dir).expect("Debe crearse directorio temporal de prueba");
        dir
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).expect("Crear carpeta destino");
        for entry in fs::read_dir(src).expect("Leer carpeta origen").flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let target = dst.join(name);
            if path.is_dir() {
                copy_dir_recursive(&path, &target);
            } else {
                fs::copy(&path, &target).expect("Copiar archivo de prueba");
            }
        }
    }

    #[test]
    fn test_vault_indexing_search_tags_and_backlinks() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let real_test_vault = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("Raíz del workspace")
            .join("test-vault");

        let temp_dir = create_temp_vault_dir("test_vault_copy");
        copy_dir_recursive(&real_test_vault, &temp_dir);
        crate::tests::ensure_canonical_seed_notes(&temp_dir);

        let vault = Vault::open(&temp_dir).expect("Abrir copia temporal de test-vault");
        let (index, stats) = VaultIndex::open_and_sync(&vault).expect("Sincronizar índice FTS5");

        assert!(stats.total_notes >= 5);
        assert_eq!(stats.indexed_or_updated, stats.total_notes);
        assert_eq!(stats.skipped_unchanged, 0);
        assert_eq!(stats.removed_deleted, 0);
        assert!(index.db_path().exists(), "Debe existir .romsidian/index.db");

        // 1. Búsqueda FTS5 con y sin tildes (unicode61 remove_diacritics 2)
        let hits_accent = index
            .search("inflación", 10)
            .expect("Búsqueda con tilde");
        let hits_no_accent = index
            .search("inflacion", 10)
            .expect("Búsqueda sin tilde");
        assert!(
            !hits_accent.is_empty(),
            "Debe encontrar notas que hablen de inflación"
        );
        assert_eq!(hits_accent[0].path, "Análisis Macroeconómico.md");
        assert_eq!(
            hits_accent[0].path, hits_no_accent[0].path,
            "La búsqueda sin tilde debe encontrar la misma nota que con tilde"
        );
        assert!(
            hits_accent[0].snippet.contains("<mark>"),
            "El snippet FTS5 debe resaltar la coincidencia con <mark>"
        );

        // 2. Filtro por etiquetas jerárquicas (#economía debe incluir #economía/argentina)
        let eco_notes = index
            .notes_by_tag("#economía")
            .expect("Filtrar por #economía");
        assert!(
            eco_notes
                .iter()
                .any(|n| n.relative_path == "Análisis Macroeconómico.md"),
            "Análisis Macroeconómico.md debe aparecer bajo #economía"
        );

        let all_tags = index.list_all_tags().expect("Listar todas las etiquetas");
        assert!(
            !all_tags.is_empty(),
            "El conteo global de etiquetas no debe estar vacío"
        );

        // 3. Backlinks entrantes hacia "Análisis Macroeconómico.md" y "subcarpeta/Notas de Investigación.md"
        let backlinks_macro = index
            .backlinks_for("Análisis Macroeconómico.md")
            .expect("Consultar backlinks hacia Análisis Macroeconómico.md");
        assert!(
            backlinks_macro
                .iter()
                .any(|b| b.source_path == "Índice General.md"),
            "Índice General.md debe figurar como backlink entrante de Análisis Macroeconómico.md"
        );
        assert!(
            backlinks_macro
                .iter()
                .any(|b| b.source_path == "subcarpeta/Notas de Investigación.md"),
            "subcarpeta/Notas de Investigación.md debe figurar como backlink entrante"
        );

        let backlinks_sub = index
            .backlinks_for("subcarpeta/Notas de Investigación.md")
            .expect("Consultar backlinks hacia subcarpeta/Notas de Investigación.md");
        assert!(
            !backlinks_sub.is_empty(),
            "Notas de Investigación.md debe recibir backlinks desde otras notas de test-vault"
        );

        drop(index);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn incremental_indexing_skips_unchanged_and_handles_crud() {
        let temp_dir = create_temp_vault_dir("incremental_crud");
        let vault = Vault::open(&temp_dir).expect("Abrir bóveda temporal");

        vault
            .create_note(
                "Nota A.md",
                "---\ntitle: Nota A\ntags: [ base ]\n---\n# Nota A\nEnlace hacia [[Nota B]] y [[Nota Futura]].\n",
            )
            .expect("Crear Nota A");
        vault
            .create_note(
                "Nota B.md",
                "# Nota B\nContenido inicial sobre programación en Rust #rust.\n",
            )
            .expect("Crear Nota B");

        let (mut index, stats1) = VaultIndex::open_and_sync(&vault).expect("Primer sync");
        assert_eq!(stats1.total_notes, 2);
        assert_eq!(stats1.indexed_or_updated, 2);
        assert_eq!(stats1.skipped_unchanged, 0);

        // Segunda sincronización sin cambios: debe omitir ambas notas por SHA-256
        let stats2 = index.sync_vault(&vault).expect("Segundo sync sin cambios");
        assert_eq!(stats2.total_notes, 2);
        assert_eq!(stats2.indexed_or_updated, 0);
        assert_eq!(stats2.skipped_unchanged, 2);
        assert_eq!(stats2.removed_deleted, 0);

        // Creamos "Nota Futura.md" que ya era referenciada por "Nota A.md"
        vault
            .create_note("Nota Futura.md", "# Nota Futura\nAhora existo.\n")
            .expect("Crear Nota Futura");
        let stats3 = index.sync_vault(&vault).expect("Tercer sync con nota nueva");
        assert_eq!(stats3.total_notes, 3);
        assert_eq!(stats3.indexed_or_updated, 1);
        assert_eq!(stats3.skipped_unchanged, 2);

        // El backlink desde Nota A hacia Nota Futura debe resolverse automáticamente
        let backlinks_futura = index
            .backlinks_for("Nota Futura.md")
            .expect("Backlinks de Nota Futura");
        assert_eq!(backlinks_futura.len(), 1);
        assert_eq!(backlinks_futura[0].source_path, "Nota A.md");

        // Eliminamos Nota B y sincronizamos
        vault.delete_note("Nota B.md").expect("Borrar Nota B");
        let stats4 = index.sync_vault(&vault).expect("Cuarto sync tras borrado");
        assert_eq!(stats4.total_notes, 2);
        assert_eq!(stats4.removed_deleted, 1);
        assert!(
            index.search("programación", 10).unwrap().is_empty(),
            "Nota B borrada no debe aparecer en búsquedas FTS5"
        );

        drop(index);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn auto_rebuilds_when_index_db_is_deleted_or_corrupted() {
        let temp_dir = create_temp_vault_dir("auto_rebuild");
        let vault = Vault::open(&temp_dir).expect("Abrir bóveda");

        vault
            .create_note(
                "Recuperación.md",
                "# Recuperación Automática\nLos archivos Markdown son la única fuente de la verdad #resiliencia.\n",
            )
            .expect("Crear nota");

        let (index, _) = VaultIndex::open_and_sync(&vault).expect("Indexar inicial");
        let db_file = index.db_path().to_path_buf();
        drop(index);

        // Caso 1: Borrado completo de .romsidian/index.db
        fs::remove_file(&db_file).expect("Borrar index.db manualmente");
        assert!(!db_file.exists());

        let (index_rebuilt, stats_rebuilt) =
            VaultIndex::open_and_sync(&vault).expect("Debe reconstruir tras borrado");
        assert_eq!(stats_rebuilt.indexed_or_updated, 1);
        assert_eq!(
            index_rebuilt.search("fuente de la verdad", 5).unwrap().len(),
            1
        );
        drop(index_rebuilt);

        // Caso 2: Corrupción de .romsidian/index.db con bytes inválidos
        fs::write(&db_file, b"ARCHIVO_SQLITE_CORRUPTO_NO_VALIDO").expect("Corromper index.db");
        let (index_healed, stats_healed) =
            VaultIndex::open_and_sync(&vault).expect("Debe sanear y reconstruir tras corrupción");
        assert_eq!(stats_healed.indexed_or_updated, 1);
        assert_eq!(
            index_healed.notes_by_tag("#resiliencia").unwrap().len(),
            1
        );
        drop(index_healed);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn benchmark_1000_notes_fts5_and_incremental_sync() {
        let temp_dir = create_temp_vault_dir("bench_1000");
        let vault = Vault::open(&temp_dir).expect("Abrir bóveda de benchmark");

        // Generamos 1.000 notas sintéticas con tildes, eñes, espacios, wikilinks y etiquetas
        for i in 0..1000 {
            let folder = match i % 4 {
                0 => "",
                1 => "Economía y Finanzas/",
                2 => "Ingeniería de Software/",
                _ => "Investigación Científica/",
            };
            let next_id = (i + 1) % 1000;
            let rel_path = format!("{folder}Nota de Análisis Número {i:04}.md");
            let special_keyword = if i == 777 {
                "hipopotomonstrosesquipedaliofobia cuántica"
            } else {
                "conocimiento distribuido"
            };

            let content = format!(
                "---\ntitle: Nota de Análisis {i:04}\ntags: [ proyecto/romsidian, lote-{bucket} ]\n---\n\
                # Nota de Análisis {i:04}\n\n\
                Esta nota documenta la investigación número {i} sobre sincronización local, \
                inflación interanual, diseño de compiladores y {special_keyword} en español (ñ, á, é, í, ó, ú).\n\
                Enlaza directamente con [[Nota de Análisis Número {next_id:04}|Siguiente Nota]] \
                y clasifica bajo #investigación/nota-{bucket}.\n",
                bucket = i % 20
            );

            vault
                .create_note(&rel_path, &content)
                .expect("Crear nota sintética");
        }

        // 1. Indexación inicial de las 1.000 notas
        let (mut index, stats_initial) =
            VaultIndex::open_and_sync(&vault).expect("Indexar 1.000 notas inicialmente");
        assert_eq!(stats_initial.total_notes, 1000);
        assert_eq!(stats_initial.indexed_or_updated, 1000);

        // 2. Modificamos 1 nota y medimos la indexación incremental (< 1.000 ms según SPEC.md)
        vault
            .save_note(
                "Nota de Análisis Número 0000.md",
                "---\ntitle: Nota de Análisis 0000 Actualizada\ntags: [ proyecto/romsidian, actualizado ]\n---\n\
                # Nota de Análisis 0000 Actualizada\n\
                Texto modificado con término único supercalifragilisticoespialidoso y enlace a [[Nota de Análisis Número 0001]].\n",
            )
            .expect("Actualizar nota 0000");

        let start_inc = Instant::now();
        let stats_inc = index
            .sync_vault(&vault)
            .expect("Sincronización incremental sobre 1.000 notas");
        let elapsed_inc = start_inc.elapsed();

        assert_eq!(stats_inc.total_notes, 1000);
        assert_eq!(stats_inc.indexed_or_updated, 1);
        assert_eq!(stats_inc.skipped_unchanged, 999);
        assert!(
            elapsed_inc.as_millis() < 1000,
            "La indexación incremental de 1.000 notas debe tardar < 1000 ms (tardó {} ms)",
            elapsed_inc.as_millis()
        );

        // 3. Medimos consultas FTS5 (< 50 ms por consulta según SPEC.md)
        let start_search = Instant::now();
        let hits_unique = index
            .search("hipopotomonstrosesquipedaliofobia cuantica", 10)
            .expect("Consulta FTS5 sin tilde sobre 1.000 notas");
        let elapsed_search = start_search.elapsed();

        assert_eq!(hits_unique.len(), 1);
        assert!(hits_unique[0].path.ends_with("Nota de Análisis Número 0777.md"));
        assert!(
            elapsed_search.as_millis() < 50,
            "La consulta FTS5 sobre 1.000 notas debe tardar < 50 ms (tardó {:?})",
            elapsed_search
        );

        // 4. Verificamos backlinks y búsqueda por etiqueta en las 1.000 notas
        let backlinks_0001 = index
            .backlinks_for("Economía y Finanzas/Nota de Análisis Número 0001.md")
            .expect("Backlinks hacia nota 0001");
        assert_eq!(backlinks_0001.len(), 1);
        assert_eq!(
            backlinks_0001[0].source_path,
            "Nota de Análisis Número 0000.md"
        );

        let tag_lote_0 = index
            .notes_by_tag("#lote-0")
            .expect("Consultar etiqueta #lote-0");
        // De las 50 notas originales con lote-0 (0, 20, 40... 980), la 0000 cambió su tag a `actualizado`, quedan 49.
        assert_eq!(tag_lote_0.len(), 49);

        drop(index);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn indexes_inline_fields_tasks_transclusions_and_unlinked_mentions() {
        let temp_dir = create_temp_vault_dir("v02_extended_index");
        let vault = Vault::open(&temp_dir).expect("Abrir bóveda temporal v0.2");

        vault
            .create_note(
                "Política Monetaria.md",
                "---\ntitle: Política Monetaria\naliases: [ Banco Central, BCRA ]\nstatus: publicado\n---\n\
                # Política Monetaria\n\
                Tasa de Referencia:: 40%\n\
                Análisis con [horizonte:: 2026] y (autor:: [[Roma]]).\n\n\
                ## Instrumentos de Regulación\n\
                Los encajes y pases pasivos regulan la liquidez bancaria.\n\
                Este resultado empírico es fundamental para el modelo ^hallazgo-bcra\n\n\
                ## Bibliografía\n\
                - [ ] Revisar informe trimestral #economía/monetaria\n\
                - [x] Descargar serie histórica\n",
            )
            .expect("Crear Política Monetaria.md");

        vault
            .create_note(
                "Informe de Coyuntura.md",
                "# Informe de Coyuntura\n\
                En la línea 2 mencionamos Política Monetaria sin corchetes.\n\
                En la línea 3 ya está enlazada como [[Política Monetaria]].\n\
                En la línea 4 se cita al alias Banco Central dentro del análisis.\n",
            )
            .expect("Crear Informe de Coyuntura.md");

        vault
            .create_note(
                "plantillas/Borrador Excluido.md",
                "# Borrador\nMención a Política Monetaria en carpeta excluida.\n",
            )
            .expect("Crear nota en carpeta excluida");

        let (mut index, stats) = VaultIndex::open_and_sync(&vault).expect("Sincronizar bóveda v0.2");
        assert_eq!(stats.total_notes, 3);

        // 1. Campos indexados (frontmatter + inline)
        let fields = index
            .fields_for_note("Política Monetaria.md")
            .expect("Campos de Política Monetaria");
        assert!(fields
            .iter()
            .any(|f| f.key_norm == "status" && f.value == "publicado" && f.source == "frontmatter"));
        assert!(fields
            .iter()
            .any(|f| f.key_norm == "tasa-de-referencia" && f.value == "40%" && f.source == "bare"));
        assert!(fields
            .iter()
            .any(|f| f.key_norm == "horizonte" && f.value == "2026" && f.source == "bracket"));
        assert!(fields
            .iter()
            .any(|f| f.key_norm == "autor" && f.value == "[[Roma]]" && f.source == "paren"));

        // 2. Tareas indexadas
        let tasks = index.list_all_tasks().expect("Listar tareas");
        assert_eq!(tasks.len(), 2);
        assert!(!tasks[0].completed);
        assert_eq!(tasks[0].tags, vec!["economía/monetaria"]);
        assert!(tasks[1].completed);

        // 3. Transclusiones de sección (#Encabezado) y de bloque (#^id)
        let sec = index
            .resolve_transclusion(
                &vault,
                "Política Monetaria",
                Some("Instrumentos de Regulación"),
            )
            .expect("Resolver transclusión de sección")
            .expect("Debe existir la sección");
        assert!(sec.content.contains("Los encajes y pases pasivos"));
        assert!(!sec.content.contains("Descargar serie histórica"));

        let blk = index
            .resolve_transclusion(&vault, "Política Monetaria#^hallazgo-bcra", None)
            .expect("Resolver transclusión de bloque")
            .expect("Debe existir el bloque");
        assert_eq!(
            blk.content,
            "Este resultado empírico es fundamental para el modelo"
        );

        // 4. Menciones no enlazadas (Unlinked Mentions) respetando excluded_patterns
        let excluded = vec!["plantillas/*".to_string()];
        let unlinked = index
            .unlinked_mentions_for("Política Monetaria.md", &excluded)
            .expect("Consultar menciones no enlazadas");

        // Debe detectar línea 2 ("Política Monetaria") y línea 4 ("Banco Central") en Informe de Coyuntura.md,
        // ignorando la línea 3 (ya enlazada) y plantillas/Borrador Excluido.md.
        assert_eq!(unlinked.len(), 2);
        assert_eq!(unlinked[0].source_path, "Informe de Coyuntura.md");
        assert_eq!(unlinked[0].line, 2);
        assert_eq!(unlinked[0].matched_term, "Política Monetaria");

        assert_eq!(unlinked[1].source_path, "Informe de Coyuntura.md");
        assert_eq!(unlinked[1].line, 4);
        assert_eq!(unlinked[1].matched_term, "Banco Central");

        // 5. Acción "Enlazar" sobre la mención no enlazada de la línea 2
        let updated_doc = index
            .link_unlinked_mention(
                &vault,
                "Informe de Coyuntura.md",
                2,
                "Política Monetaria",
                "Política Monetaria.md",
            )
            .expect("Enlazar mención no enlazada");
        assert!(updated_doc
            .content
            .contains("mencionamos [[Política Monetaria]] sin corchetes."));

        // Ahora queda solo 1 mención no enlazada (la del alias en línea 4) y hay 2 backlinks enlazados
        let unlinked_after = index
            .unlinked_mentions_for("Política Monetaria.md", &excluded)
            .expect("Menciones tras enlazar");
        assert_eq!(unlinked_after.len(), 1);
        assert_eq!(unlinked_after[0].line, 4);

        let backlinks_after = index
            .backlinks_for("Política Monetaria.md")
            .expect("Backlinks tras enlazar");
        assert_eq!(backlinks_after.len(), 2);

        drop(index);
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
