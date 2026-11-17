//! Motor de mapa de conocimiento, sugerencia de conexiones y auto-inyección PKM (`[RF-15]`).
//!
//! Opera en dos etapas para ahorrar tokens y RAM:
//! 1. **Capa determinista local (0 tokens, SQLite FTS5)**: construye el mapa compacto
//!    de la bóveda (`build_vault_knowledge_map`), detecta conexiones candidatas combinando
//!    menciones no enlazadas, `#tags` compartidos, campos inline/YAML y similitud léxica BM25
//!    (`suggest_note_connections`), y genera planes offline (`build_deterministic_enrichment_plan`).
//! 2. **Inyección idempotente y previsualización**: fusiona metadatos `frontmatter`,
//!    normaliza `#tags`, inyecta `[[wikilinks]]` protegiendo código/URLs/enlaces previos
//!    (`preview_ai_enrichment`, `apply_ai_enrichment`) y genera mapas `.canvas` (`generate_knowledge_canvas`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::index::{
    compute_protected_ranges, find_unlinked_term_in_line, fold_diacritics, is_word_boundary_char,
    normalize_link_target, path_lookup_keys, TagCount, VaultIndex,
};
use crate::parser::{
    detect_code_fence, extract_frontmatter, normalize_tag, parse_note, FrontmatterValue, ParsedNote,
};
use crate::vault::{format_unix_secs_utc, CanvasDocument, NoteDocument, Vault};
use crate::{CoreError, CoreResult};

/// Resumen compacto de una nota dentro del mapa de conocimiento de la bóveda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeNoteEntry {
    pub path: String,
    pub stem: String,
    pub title: String,
    pub folder: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub headings: Vec<String>,
    pub frontmatter_keys: Vec<String>,
    pub outlink_count: usize,
    pub inlink_count: usize,
    pub is_orphan: bool,
}

/// Estadística de uso de una clave de `frontmatter` en toda la bóveda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontmatterKeyStat {
    pub key: String,
    pub count: usize,
    pub sample_values: Vec<String>,
}

/// Mapa compacto de conocimiento de toda la bóveda (`0 tokens`, generado desde SQLite).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultKnowledgeMap {
    pub total_notes: usize,
    pub total_links: usize,
    pub total_tags: usize,
    pub orphan_paths: Vec<String>,
    pub folders: Vec<String>,
    pub tag_taxonomy: Vec<TagCount>,
    pub frontmatter_schema: Vec<FrontmatterKeyStat>,
    pub notes: Vec<KnowledgeNoteEntry>,
}

impl VaultKnowledgeMap {
    /// Serializa el mapa en formato compacto tipo `llms.txt` para reducir consumo de tokens en IA.
    pub fn to_compact_prompt_context(&self, max_notes: usize) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# Mapa de Bóveda ({} notas, {} enlaces, {} tags, {} huérfanas)\n",
            self.total_notes,
            self.total_links,
            self.total_tags,
            self.orphan_paths.len()
        ));

        if !self.folders.is_empty() {
            out.push_str("## Carpetas\n");
            out.push_str(&self.folders.join(", "));
            out.push('\n');
        }

        if !self.tag_taxonomy.is_empty() {
            out.push_str("## Taxonomía de #tags\n");
            let tags_line: Vec<String> = self
                .tag_taxonomy
                .iter()
                .take(40)
                .map(|t| format!("#{} ({})", t.tag, t.count))
                .collect();
            out.push_str(&tags_line.join(", "));
            out.push('\n');
        }

        if !self.frontmatter_schema.is_empty() {
            out.push_str("## Claves Frontmatter\n");
            let keys_line: Vec<String> = self
                .frontmatter_schema
                .iter()
                .take(20)
                .map(|k| format!("{} ({})", k.key, k.count))
                .collect();
            out.push_str(&keys_line.join(", "));
            out.push('\n');
        }

        out.push_str("## Notas\n");
        let limit = if max_notes == 0 {
            self.notes.len()
        } else {
            max_notes
        };
        for note in self.notes.iter().take(limit) {
            let mut meta_parts = Vec::new();
            if !note.aliases.is_empty() {
                meta_parts.push(format!("aliases:[{}]", note.aliases.join("|")));
            }
            if !note.tags.is_empty() {
                let tag_list: Vec<String> = note.tags.iter().map(|t| format!("#{t}")).collect();
                meta_parts.push(tag_list.join(" "));
            }
            if !note.headings.is_empty() {
                meta_parts.push(format!("H:[{}]", note.headings.join(" / ")));
            }
            let suffix = if meta_parts.is_empty() {
                String::new()
            } else {
                format!(" | {}", meta_parts.join(" | "))
            };
            out.push_str(&format!("- [[{}]] ({}){}\n", note.stem, note.path, suffix));
        }
        out
    }
}

/// Señal individual que respalda una conexión candidata entre dos notas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionSignal {
    OutgoingUnlinkedMention {
        matched_term: String,
        line: usize,
        context: String,
    },
    IncomingUnlinkedMention {
        matched_term: String,
        line: usize,
        context: String,
    },
    SharedTags {
        tags: Vec<String>,
    },
    SharedFields {
        fields: Vec<String>,
    },
    LexicalBm25 {
        score: f64,
        matched_keywords: Vec<String>,
    },
}

/// Reemplazo concreto propuesto para convertir texto en un `[[wikilink]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikilinkReplacement {
    pub target_path: String,
    pub target_stem: String,
    pub matched_term: String,
    pub replacement_wikilink: String,
    pub line: usize,
    pub context: String,
}

/// Conexión sugerida entre la nota consultada y otra nota de la bóveda.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateConnection {
    pub target_path: String,
    pub target_title: String,
    pub target_stem: String,
    pub target_tags: Vec<String>,
    pub score: f64,
    pub already_linked: bool,
    pub suggested_wikilink: String,
    pub inline_replacements: Vec<WikilinkReplacement>,
    pub signals: Vec<ConnectionSignal>,
    pub reason_summary: String,
}

/// Enlace relacionado propuesto para la sección opcional `## Conexiones Relacionadas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedConnectionLink {
    pub target_path: String,
    pub target_title: String,
    pub relation: Option<String>,
}

/// Plan estructurado de enriquecimiento (`frontmatter`, `#tags` y `[[wikilinks]]`) aplicable a una nota.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiEnrichmentPlan {
    #[serde(default)]
    pub frontmatter_updates: BTreeMap<String, FrontmatterValue>,
    #[serde(default)]
    pub overwrite_existing_frontmatter: bool,
    #[serde(default)]
    pub suggested_tags: Vec<String>,
    #[serde(default)]
    pub inline_wikilinks: Vec<WikilinkReplacement>,
    #[serde(default)]
    pub related_connections: Vec<RelatedConnectionLink>,
    #[serde(default)]
    pub append_related_section: bool,
}

/// Resultado de previsualización pura en memoria antes de escribir en disco.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiEnrichmentPreview {
    pub relative_path: String,
    pub original_content: String,
    pub enriched_content: String,
    pub changed: bool,
    pub added_frontmatter_keys: Vec<String>,
    pub added_tags: Vec<String>,
    pub injected_wikilinks: Vec<WikilinkReplacement>,
    pub appended_related_links: Vec<String>,
    pub unified_diff_lines: Vec<String>,
}

/// Resultado tras aplicar un plan de enriquecimiento en disco y reindexar SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiEnrichmentApplyResult {
    pub preview: AiEnrichmentPreview,
    pub note: NoteDocument,
}

/// Parámetros para generar un lienzo JSON Canvas (`.canvas`) temático desde notas y conexiones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeCanvasRequest {
    pub canvas_path: String,
    pub title: Option<String>,
    #[serde(default)]
    pub seed_note_paths: Vec<String>,
    #[serde(default = "default_true")]
    pub include_neighbors: bool,
    #[serde(default = "default_canvas_max_nodes")]
    pub max_nodes: usize,
}

/// Resultado de la creación/actualización de un lienzo temático `.canvas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeCanvasResult {
    pub canvas: CanvasDocument,
    pub node_count: usize,
    pub edge_count: usize,
}

fn default_true() -> bool {
    true
}

fn default_canvas_max_nodes() -> usize {
    12
}

/// Datos internos de cada nota cargada desde SQLite para cómputo de mapa y candidatos.
struct IndexedNoteRow {
    path: String,
    stem: String,
    title: String,
    aliases: Vec<String>,
    frontmatter: BTreeMap<String, FrontmatterValue>,
    tags: Vec<String>,
    headings: Vec<String>,
    body: String,
    body_start_line: usize,
}

/// Construye el mapa compacto de conocimiento de toda la bóveda desde `.romsidian/index.db` (`[RF-15]`).
pub fn build_vault_knowledge_map(index: &VaultIndex) -> CoreResult<VaultKnowledgeMap> {
    let rows = load_indexed_notes(index)?;
    let tag_taxonomy = index.list_all_tags()?;

    let mut out_counts: HashMap<String, usize> = HashMap::new();
    let mut in_counts: HashMap<String, usize> = HashMap::new();
    let mut total_links = 0usize;

    {
        let mut stmt = index.conn.prepare(
            "SELECT source_path, target_resolved_path FROM note_links",
        )?;
        let link_rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for item in link_rows {
            let (source_path, target_resolved) = item?;
            total_links += 1;
            *out_counts.entry(source_path.clone()).or_insert(0) += 1;
            if let Some(target_path) = target_resolved {
                if target_path != source_path {
                    *in_counts.entry(target_path).or_insert(0) += 1;
                }
            }
        }
    }

    let mut folder_set = BTreeSet::new();
    let mut orphan_paths = Vec::new();
    let mut fm_counts: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    let mut notes = Vec::with_capacity(rows.len());

    for row in rows {
        let folder = extract_folder(&row.path);
        if !folder.is_empty() {
            folder_set.insert(folder.clone());
        }

        let frontmatter_keys: Vec<String> = row.frontmatter.keys().cloned().collect();
        for (key, value) in &row.frontmatter {
            let entry = fm_counts
                .entry(key.clone())
                .or_insert_with(|| (0, BTreeSet::new()));
            entry.0 += 1;
            if entry.1.len() < 3 {
                match value {
                    FrontmatterValue::Scalar(s) if !s.trim().is_empty() => {
                        entry.1.insert(truncate_sample(s.trim(), 48));
                    }
                    FrontmatterValue::List(items) => {
                        for item in items.iter().take(2) {
                            if !item.trim().is_empty() && entry.1.len() < 3 {
                                entry.1.insert(truncate_sample(item.trim(), 48));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let outlink_count = *out_counts.get(&row.path).unwrap_or(&0);
        let inlink_count = *in_counts.get(&row.path).unwrap_or(&0);
        let is_orphan = outlink_count == 0 && inlink_count == 0;
        if is_orphan {
            orphan_paths.push(row.path.clone());
        }

        notes.push(KnowledgeNoteEntry {
            path: row.path,
            stem: row.stem,
            title: row.title,
            folder,
            aliases: row.aliases,
            tags: row.tags,
            headings: row.headings,
            frontmatter_keys,
            outlink_count,
            inlink_count,
            is_orphan,
        });
    }

    let mut frontmatter_schema: Vec<FrontmatterKeyStat> = fm_counts
        .into_iter()
        .map(|(key, (count, samples))| FrontmatterKeyStat {
            key,
            count,
            sample_values: samples.into_iter().collect(),
        })
        .collect();
    frontmatter_schema.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));

    Ok(VaultKnowledgeMap {
        total_notes: notes.len(),
        total_links,
        total_tags: tag_taxonomy.len(),
        orphan_paths,
        folders: folder_set.into_iter().collect(),
        tag_taxonomy,
        frontmatter_schema,
        notes,
    })
}

/// Detecta conexiones candidatas para `relative_path` combinando menciones no enlazadas,
/// `#tags` compartidos, campos en común y similitud léxica BM25 en SQLite FTS5 (`[RF-15]`).
pub fn suggest_note_connections(
    index: &VaultIndex,
    relative_path: &str,
    max_candidates: usize,
) -> CoreResult<Vec<CandidateConnection>> {
    if max_candidates == 0 {
        return Ok(Vec::new());
    }

    let resolved_source = resolve_indexed_note_path(index, relative_path)?;
    let all_notes = load_indexed_notes(index)?;
    let Some(source_idx) = all_notes.iter().position(|n| n.path == resolved_source) else {
        return Err(CoreError::NoteNotFound(resolved_source));
    };

    let outgoing_targets = load_outgoing_resolved_targets(index, &resolved_source)?;
    let fields_by_note = load_significant_fields_map(index)?;
    let incoming_mentions = index.unlinked_mentions_for(&resolved_source, &[])?;

    let source_note = &all_notes[source_idx];
    let bm25_hits = compute_bm25_neighbors(index, source_note)?;

    struct Accumulator {
        score: f64,
        signals: Vec<ConnectionSignal>,
        inline_replacements: Vec<WikilinkReplacement>,
    }

    let mut acc_map: HashMap<usize, Accumulator> = HashMap::new();

    // Señal 1: Menciones salientes dentro de la nota actual hacia otras notas (stem/título/aliases).
    let target_terms_per_note: Vec<Vec<String>> = all_notes
        .iter()
        .enumerate()
        .map(|(idx, note)| {
            if idx == source_idx {
                Vec::new()
            } else {
                collect_candidate_terms(note)
            }
        })
        .collect();

    scan_outgoing_unlinked_mentions(source_note, &all_notes, &target_terms_per_note, &mut |target_idx, replacement| {
        let entry = acc_map.entry(target_idx).or_insert_with(|| Accumulator {
            score: 0.0,
            signals: Vec::new(),
            inline_replacements: Vec::new(),
        });
        // Evitar múltiples entradas repetidas hacia la misma nota destino.
        if !entry
            .inline_replacements
            .iter()
            .any(|r| r.target_path == replacement.target_path)
        {
            entry.score += 45.0;
            entry.signals.push(ConnectionSignal::OutgoingUnlinkedMention {
                matched_term: replacement.matched_term.clone(),
                line: replacement.line,
                context: replacement.context.clone(),
            });
            entry.inline_replacements.push(replacement);
        }
    });

    // Señal 2: Menciones entrantes no enlazadas desde otras notas hacia la nota actual.
    let path_to_idx: HashMap<&str, usize> = all_notes
        .iter()
        .enumerate()
        .map(|(idx, note)| (note.path.as_str(), idx))
        .collect();
    let mut incoming_seen = HashSet::new();
    for hit in incoming_mentions {
        if let Some(&target_idx) = path_to_idx.get(hit.source_path.as_str()) {
            if target_idx != source_idx && incoming_seen.insert(target_idx) {
                let entry = acc_map.entry(target_idx).or_insert_with(|| Accumulator {
                    score: 0.0,
                    signals: Vec::new(),
                    inline_replacements: Vec::new(),
                });
                entry.score += 25.0;
                entry.signals.push(ConnectionSignal::IncomingUnlinkedMention {
                    matched_term: hit.matched_term,
                    line: hit.line,
                    context: hit.context,
                });
            }
        }
    }

    // Señales 3, 4 y 5 por cada nota candidata != source.
    let source_fields = fields_by_note.get(&source_note.path);
    for (idx, candidate) in all_notes.iter().enumerate() {
        if idx == source_idx {
            continue;
        }

        // Señal 3: Solapamiento de #tags exactos o jerárquicos.
        let shared_tags = compute_shared_tags(&source_note.tags, &candidate.tags);
        if !shared_tags.is_empty() {
            let entry = acc_map.entry(idx).or_insert_with(|| Accumulator {
                score: 0.0,
                signals: Vec::new(),
                inline_replacements: Vec::new(),
            });
            entry.score += 15.0 * (shared_tags.len().min(3) as f64);
            entry.signals.push(ConnectionSignal::SharedTags { tags: shared_tags });
        }

        // Señal 4: Campos significativos compartidos (autor, fuente, tema, proyecto, categoría...).
        if let (Some(src_f), Some(dst_f)) = (source_fields, fields_by_note.get(&candidate.path)) {
            let mut shared_fields = Vec::new();
            for (key_norm, src_vals) in src_f {
                if let Some(dst_vals) = dst_f.get(key_norm) {
                    if let Some(common_val) = src_vals.iter().find(|v| dst_vals.contains(*v)) {
                        shared_fields.push(format!("{key_norm}={common_val}"));
                    }
                }
            }
            if !shared_fields.is_empty() {
                let entry = acc_map.entry(idx).or_insert_with(|| Accumulator {
                    score: 0.0,
                    signals: Vec::new(),
                    inline_replacements: Vec::new(),
                });
                entry.score += 12.0 * (shared_fields.len().min(3) as f64);
                entry.signals.push(ConnectionSignal::SharedFields {
                    fields: shared_fields,
                });
            }
        }

        // Señal 5: Similitud léxica BM25 en FTS5.
        if let Some((bm25_score, matched_keywords)) = bm25_hits.get(&candidate.path) {
            let entry = acc_map.entry(idx).or_insert_with(|| Accumulator {
                score: 0.0,
                signals: Vec::new(),
                inline_replacements: Vec::new(),
            });
            entry.score += *bm25_score;
            entry.signals.push(ConnectionSignal::LexicalBm25 {
                score: *bm25_score,
                matched_keywords: matched_keywords.clone(),
            });
        }
    }

    let mut candidates: Vec<CandidateConnection> = acc_map
        .into_iter()
        .map(|(idx, acc)| {
            let note = &all_notes[idx];
            let already_linked = outgoing_targets.contains(&note.path);
            let reason_summary = summarize_signals(&acc.signals);
            CandidateConnection {
                target_path: note.path.clone(),
                target_title: note.title.clone(),
                target_stem: note.stem.clone(),
                target_tags: note.tags.clone(),
                score: (acc.score * 10.0).round() / 10.0,
                already_linked,
                suggested_wikilink: format!("[[{}]]", note.stem),
                inline_replacements: acc.inline_replacements,
                signals: acc.signals,
                reason_summary,
            }
        })
        .collect();

    // Priorizar notas aún no enlazadas, luego mayor score y orden alfabético determinista.
    candidates.sort_by(|a, b| {
        a.already_linked
            .cmp(&b.already_linked)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| a.target_path.cmp(&b.target_path))
    });
    candidates.truncate(max_candidates);

    Ok(candidates)
}

/// Genera un plan determinista offline (0 tokens) para enriquecer `relative_path` con
/// `frontmatter`, `#tags` existentes en la taxonomía de la bóveda y `[[wikilinks]]` (`[RF-15]`).
pub fn build_deterministic_enrichment_plan(
    vault: &Vault,
    index: &VaultIndex,
    relative_path: &str,
    max_connections: usize,
    append_related_section: bool,
) -> CoreResult<AiEnrichmentPlan> {
    let resolved_path = resolve_indexed_note_path(index, relative_path)
        .unwrap_or_else(|_| relative_path.replace('\\', "/"));
    let doc = vault.read_note(&resolved_path)?;
    let parsed = parse_note(&doc.content);
    let candidates = suggest_note_connections(index, &doc.relative_path, max_connections.max(1))?;

    let mut frontmatter_updates = BTreeMap::new();
    if !parsed.frontmatter.contains_key("title") {
        let inferred_title = parsed
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| doc.title.clone());
        frontmatter_updates.insert("title".to_string(), FrontmatterValue::Scalar(inferred_title));
    }

    if !parsed.frontmatter.contains_key("updated") && !parsed.frontmatter.contains_key("created") {
        let (date_str, _) = format_unix_secs_utc(doc.modified_unix_secs as i64);
        if date_str != "1970-01-01" {
            frontmatter_updates.insert("updated".to_string(), FrontmatterValue::Scalar(date_str));
        }
    }

    // Sugerir #tags de la taxonomía de la bóveda presentes en el texto o compartidos por candidatos fuertes.
    let all_tags = index.list_all_tags()?;
    let searchable_text = fold_diacritics(&format!("{} {}", doc.title, parsed.body).to_lowercase());
    let mut tag_candidates: Vec<(String, usize)> = Vec::new();
    let mut seen_suggested_tags = HashSet::new();

    for tc in &all_tags {
        if parsed.has_tag(&tc.tag) || seen_suggested_tags.contains(&tc.tag) {
            continue;
        }
        let leaf = tc.tag.rsplit('/').next().unwrap_or(&tc.tag);
        let leaf_folded = fold_diacritics(&leaf.replace(['-', '_'], " "));
        if leaf_folded.chars().count() >= 4 && contains_whole_phrase(&searchable_text, &leaf_folded)
        {
            seen_suggested_tags.insert(tc.tag.clone());
            tag_candidates.push((tc.tag.clone(), tc.count + 100));
        }
    }

    for cand in &candidates {
        if cand.score >= 30.0 {
            for t in &cand.target_tags {
                if !parsed.has_tag(t) && seen_suggested_tags.insert(t.clone()) {
                    tag_candidates.push((t.clone(), 10));
                }
            }
        }
    }
    tag_candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let suggested_tags: Vec<String> = tag_candidates.into_iter().take(5).map(|(t, _)| t).collect();

    let mut inline_wikilinks = Vec::new();
    let mut related_connections = Vec::new();

    for cand in &candidates {
        if cand.already_linked {
            continue;
        }
        if let Some(first_rep) = cand.inline_replacements.first() {
            inline_wikilinks.push(first_rep.clone());
        } else if append_related_section {
            related_connections.push(RelatedConnectionLink {
                target_path: cand.target_path.clone(),
                target_title: cand.target_title.clone(),
                relation: if cand.reason_summary.is_empty() {
                    None
                } else {
                    Some(cand.reason_summary.clone())
                },
            });
        }
    }

    let should_append = append_related_section && !related_connections.is_empty();

    Ok(AiEnrichmentPlan {
        frontmatter_updates,
        overwrite_existing_frontmatter: false,
        suggested_tags,
        inline_wikilinks,
        related_connections,
        append_related_section: should_append,
    })
}

/// Previsualiza de forma pura e idempotente los cambios de `AiEnrichmentPlan` sobre `note_content` (`[RF-15]`).
///
/// Protege bloques de código cercados, bloques `dataview`, encabezados `#`, código inline,
/// URLs, enlaces Markdown y `[[wikilinks]]` ya presentes.
pub fn preview_ai_enrichment(
    relative_path: &str,
    note_content: &str,
    plan: &AiEnrichmentPlan,
) -> AiEnrichmentPreview {
    let clean_rel = relative_path.replace('\\', "/");
    let line_sep = if note_content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let clean_content = note_content.strip_prefix('\u{feff}').unwrap_or(note_content);
    let parsed = parse_note(clean_content);
    let (_, raw_body_slice, body_start_line) = extract_frontmatter(clean_content);

    // 1. Fusionar Frontmatter + Tags manteniendo claves existentes del usuario.
    let (merged_frontmatter, added_frontmatter_keys, added_tags, frontmatter_changed) =
        merge_frontmatter_and_tags(&parsed, plan);

    let frontmatter_block = if !frontmatter_changed {
        extract_raw_frontmatter_prefix(clean_content, body_start_line)
    } else {
        serialize_yaml_frontmatter(&merged_frontmatter, line_sep)
    };

    // 2. Registrar destinos ya enlazados en la nota para garantizar idempotencia.
    let mut linked_targets = collect_existing_link_keys(&parsed);

    // 3. Inyectar [[wikilinks]] inline de forma segura línea por línea en el cuerpo.
    let mut lines = split_body_lines_with_eligibility(raw_body_slice, body_start_line);
    let mut injected_wikilinks = Vec::new();

    for rep in &plan.inline_wikilinks {
        let target_key = target_key_from_path_or_stem(&rep.target_path, &rep.target_stem);
        if target_key.is_empty() || linked_targets.contains(&target_key) {
            continue;
        }
        let term = rep.matched_term.trim();
        if term.chars().count() < 2 {
            continue;
        }
        let wikilink_str = format_replacement_wikilink(
            &rep.target_stem,
            &rep.target_path,
            term,
            &rep.replacement_wikilink,
        );

        let injected = inject_single_wikilink(
            &mut lines,
            rep.line,
            term,
            &wikilink_str,
            rep,
        );
        if let Some(done) = injected {
            linked_targets.insert(target_key);
            if let Some(norm_stem) = normalize_link_target(&rep.target_stem) {
                linked_targets.insert(norm_stem);
            }
            injected_wikilinks.push(done);
        }
    }

    let mut body_output: String = lines
        .into_iter()
        .map(|l| format!("{}{}", l.text, l.newline))
        .collect();

    // 4. Anexar / completar sección opcional `## Conexiones Relacionadas`.
    let mut appended_related_links = Vec::new();
    if plan.append_related_section && !plan.related_connections.is_empty() {
        let mut bullet_lines = Vec::new();
        for rel in &plan.related_connections {
            let stem = extract_stem(&rel.target_path);
            let target_key = target_key_from_path_or_stem(&rel.target_path, &stem);
            let stem_key = normalize_link_target(&stem).unwrap_or_default();
            if target_key.is_empty()
                || linked_targets.contains(&target_key)
                || (!stem_key.is_empty() && linked_targets.contains(&stem_key))
            {
                continue;
            }
            linked_targets.insert(target_key);
            if !stem_key.is_empty() {
                linked_targets.insert(stem_key);
            }

            let wikilink = if rel.target_title.trim().is_empty()
                || rel.target_title.trim().eq_ignore_ascii_case(&stem)
            {
                format!("[[{stem}]]")
            } else {
                format!("[[{stem}|{}]]", rel.target_title.trim())
            };
            let line = match rel.relation.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(reason) => format!("- {wikilink} — {reason}"),
                None => format!("- {wikilink}"),
            };
            appended_related_links.push(wikilink);
            bullet_lines.push(line);
        }

        if !bullet_lines.is_empty() {
            body_output = append_related_connections_section(&body_output, &bullet_lines, line_sep);
        }
    }

    let enriched_content = if frontmatter_block.is_empty() {
        body_output
    } else if body_output.is_empty() {
        frontmatter_block
    } else {
        let sep_needed = !frontmatter_block.ends_with('\n');
        if sep_needed {
            format!("{frontmatter_block}{line_sep}{body_output}")
        } else {
            format!("{frontmatter_block}{body_output}")
        }
    };

    let changed = enriched_content != note_content;
    let unified_diff_lines = if changed {
        build_line_diff_summary(note_content, &enriched_content)
    } else {
        Vec::new()
    };

    AiEnrichmentPreview {
        relative_path: clean_rel,
        original_content: note_content.to_string(),
        enriched_content,
        changed,
        added_frontmatter_keys,
        added_tags,
        injected_wikilinks,
        appended_related_links,
        unified_diff_lines,
    }
}

/// Aplica un `AiEnrichmentPlan` a `relative_path` en disco si hay cambios reales y reindexa SQLite (`[RF-15]`).
pub fn apply_ai_enrichment(
    vault: &Vault,
    index: &mut VaultIndex,
    relative_path: &str,
    plan: &AiEnrichmentPlan,
) -> CoreResult<AiEnrichmentApplyResult> {
    let doc = vault.read_note(relative_path)?;
    let preview = preview_ai_enrichment(&doc.relative_path, &doc.content, plan);
    let updated_doc = if preview.changed {
        let saved = vault.save_note(&doc.relative_path, &preview.enriched_content)?;
        index.index_note(vault, &saved.relative_path)?;
        saved
    } else {
        doc
    };

    Ok(AiEnrichmentApplyResult {
        preview,
        note: updated_doc,
    })
}

/// Crea o actualiza un lienzo JSON Canvas 1.0 (`.canvas`) conectando notas semilla y vecinas (`[RF-15]`).
pub fn generate_knowledge_canvas(
    vault: &Vault,
    index: &VaultIndex,
    req: &KnowledgeCanvasRequest,
) -> CoreResult<KnowledgeCanvasResult> {
    let max_nodes = req.max_nodes.clamp(1, 40);
    let all_notes = load_indexed_notes(index)?;
    let note_by_path: HashMap<&str, &IndexedNoteRow> =
        all_notes.iter().map(|n| (n.path.as_str(), n)).collect();

    let mut selected_paths: Vec<String> = Vec::new();
    let mut selected_set: HashSet<String> = HashSet::new();

    if req.seed_note_paths.is_empty() {
        for note in all_notes.iter().take(max_nodes) {
            if selected_set.insert(note.path.clone()) {
                selected_paths.push(note.path.clone());
            }
        }
    } else {
        for seed in &req.seed_note_paths {
            if selected_paths.len() >= max_nodes {
                break;
            }
            if let Ok(resolved) = resolve_indexed_note_path(index, seed) {
                if selected_set.insert(resolved.clone()) {
                    selected_paths.push(resolved);
                }
            }
        }

        if req.include_neighbors && selected_paths.len() < max_nodes {
            let seeds_snapshot = selected_paths.clone();
            for seed_path in seeds_snapshot {
                if selected_paths.len() >= max_nodes {
                    break;
                }
                if let Ok(cands) = suggest_note_connections(index, &seed_path, 6) {
                    for cand in cands {
                        if selected_paths.len() >= max_nodes {
                            break;
                        }
                        if selected_set.insert(cand.target_path.clone()) {
                            selected_paths.push(cand.target_path);
                        }
                    }
                }
            }
        }
    }

    let mut nodes = Vec::new();
    let header_title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Mapa de Conocimiento");
    nodes.push(json!({
        "id": "ai-canvas-header",
        "type": "text",
        "text": format!("# {header_title}\nGenerado con {} nota(s) conectadas en ROMSIDIAN.", selected_paths.len()),
        "x": 0,
        "y": 0,
        "width": 680,
        "height": 120,
        "color": "6"
    }));

    let count = selected_paths.len().max(1);
    let cols = ((count as f64).sqrt().ceil() as usize).clamp(1, 4);
    let mut id_by_path: HashMap<String, (String, i64, i64)> = HashMap::new();
    let palette = ["5", "4", "2", "3", "6"];

    for (idx, path) in selected_paths.iter().enumerate() {
        let col = (idx % cols) as i64;
        let row = (idx / cols) as i64;
        let x = col * 400;
        let y = 180 + row * 240;
        let node_id = format!("note-{}", idx + 1);
        let color = palette[idx % palette.len()];

        id_by_path.insert(path.clone(), (node_id.clone(), x, y));
        nodes.push(json!({
            "id": node_id,
            "type": "file",
            "file": path,
            "x": x,
            "y": y,
            "width": 320,
            "height": 160,
            "color": color
        }));
    }

    let mut edges = Vec::new();
    let mut edge_pairs: HashSet<(String, String)> = HashSet::new();

    // 1. Aristas reales desde note_links.
    {
        let mut stmt = index.conn.prepare(
            "SELECT source_path, target_resolved_path FROM note_links WHERE target_resolved_path IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (src, dst) = row?;
            if src == dst || !selected_set.contains(&src) || !selected_set.contains(&dst) {
                continue;
            }
            if edge_pairs.insert((src.clone(), dst.clone())) {
                if let (Some((from_id, fx, fy)), Some((to_id, tx, ty))) =
                    (id_by_path.get(&src), id_by_path.get(&dst))
                {
                    let (from_side, to_side) = pick_canvas_sides(*fx, *fy, *tx, *ty);
                    edges.push(json!({
                        "id": format!("edge-{}", edges.len() + 1),
                        "fromNode": from_id,
                        "fromSide": from_side,
                        "toNode": to_id,
                        "toSide": to_side,
                        "toEnd": "arrow"
                    }));
                }
            }
        }
    }

    // 2. Aristas sugeridas entre notas seleccionadas aún sin vínculo directo.
    for path in &selected_paths {
        if let Ok(cands) = suggest_note_connections(index, path, 5) {
            for cand in cands {
                if cand.already_linked || cand.score < 15.0 || !selected_set.contains(&cand.target_path)
                {
                    continue;
                }
                let already_undirected = edge_pairs.contains(&(path.clone(), cand.target_path.clone()))
                    || edge_pairs.contains(&(cand.target_path.clone(), path.clone()));
                if !already_undirected {
                    edge_pairs.insert((path.clone(), cand.target_path.clone()));
                    if let (Some((from_id, fx, fy)), Some((to_id, tx, ty))) =
                        (id_by_path.get(path), id_by_path.get(&cand.target_path))
                    {
                        let (from_side, to_side) = pick_canvas_sides(*fx, *fy, *tx, *ty);
                        let label = note_by_path
                            .get(cand.target_path.as_str())
                            .map(|_| "Sugerida")
                            .unwrap_or("Relacionada");
                        edges.push(json!({
                            "id": format!("edge-{}", edges.len() + 1),
                            "fromNode": from_id,
                            "fromSide": from_side,
                            "toNode": to_id,
                            "toSide": to_side,
                            "toEnd": "arrow",
                            "label": label,
                            "color": "3"
                        }));
                    }
                }
            }
        }
    }

    let canvas_json = serde_json::to_string_pretty(&json!({
        "nodes": nodes,
        "edges": edges
    }))
    .map_err(|err| CoreError::InvalidNotePath(format!("No se pudo serializar JSON Canvas: {err}")))?;

    let saved = vault.save_canvas(&req.canvas_path, &format!("{canvas_json}\n"))?;
    Ok(KnowledgeCanvasResult {
        canvas: saved,
        node_count: nodes.len(),
        edge_count: edges.len(),
    })
}

fn load_indexed_notes(index: &VaultIndex) -> CoreResult<Vec<IndexedNoteRow>> {
    let mut stmt = index.conn.prepare(
        "
        SELECT n.path, n.title, n.frontmatter_json, n.body_start_line, COALESCE(f.body, '')
        FROM notes n
        LEFT JOIN notes_fts f ON f.path = n.path
        ORDER BY n.path ASC
        ",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?.max(1) as usize,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut notes = Vec::new();
    let mut path_to_pos = HashMap::new();
    for row in rows {
        let (path, title, fm_json, body_start_line, body) = row?;
        let frontmatter: BTreeMap<String, FrontmatterValue> =
            serde_json::from_str(&fm_json).unwrap_or_default();
        let aliases = extract_aliases(&frontmatter);
        let stem = extract_stem(&path);
        path_to_pos.insert(path.clone(), notes.len());
        notes.push(IndexedNoteRow {
            path,
            stem,
            title,
            aliases,
            frontmatter,
            tags: Vec::new(),
            headings: Vec::new(),
            body,
            body_start_line,
        });
    }

    let mut tag_stmt = index
        .conn
        .prepare("SELECT path, tag FROM note_tags ORDER BY path ASC, tag ASC")?;
    let tag_rows = tag_stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    for row in tag_rows {
        let (path, tag) = row?;
        if let Some(&pos) = path_to_pos.get(&path) {
            notes[pos].tags.push(tag);
        }
    }

    let mut heading_stmt = index.conn.prepare(
        "SELECT path, text FROM note_headings WHERE level <= 3 ORDER BY path ASC, line ASC",
    )?;
    let heading_rows =
        heading_stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    for row in heading_rows {
        let (path, text) = row?;
        if let Some(&pos) = path_to_pos.get(&path) {
            if notes[pos].headings.len() < 8 {
                notes[pos].headings.push(text);
            }
        }
    }

    Ok(notes)
}

fn resolve_indexed_note_path(index: &VaultIndex, raw_path: &str) -> CoreResult<String> {
    let trimmed = raw_path.trim().replace('\\', "/");
    let clean = trimmed.trim_start_matches('/');
    if clean.is_empty() {
        return Err(CoreError::InvalidNotePath(
            "La ruta de la nota no puede estar vacía".to_string(),
        ));
    }

    let exact_candidate = if clean.ends_with(".md") || clean.ends_with(".MD") {
        clean.to_string()
    } else {
        format!("{clean}.md")
    };

    if let Some(found) = index
        .conn
        .query_row(
            "SELECT path FROM notes WHERE path = ?1",
            params![exact_candidate],
            |row| row.get::<_, String>(0),
        )
        .ok()
    {
        return Ok(found);
    }

    if let Some(found) = index.resolve_note_target(&exact_candidate)? {
        return Ok(found);
    }

    Err(CoreError::NoteNotFound(exact_candidate))
}

fn load_outgoing_resolved_targets(index: &VaultIndex, source_path: &str) -> CoreResult<HashSet<String>> {
    let mut stmt = index.conn.prepare(
        "SELECT target_resolved_path FROM note_links WHERE source_path = ?1 AND target_resolved_path IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![source_path], |row| row.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set)
}

fn load_significant_fields_map(
    index: &VaultIndex,
) -> CoreResult<HashMap<String, HashMap<String, Vec<String>>>> {
    let mut stmt = index
        .conn
        .prepare("SELECT path, key_norm, value FROM note_fields")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let significant = [
        "author", "autor", "source", "fuente", "project", "proyecto", "topic", "tema",
        "category", "categoria", "course", "materia", "area", "área", "type", "tipo",
    ];
    let mut map: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for row in rows {
        let (path, key_norm, value) = row?;
        if !significant.contains(&key_norm.as_str()) {
            continue;
        }
        let val_norm = value.trim().to_lowercase();
        if val_norm.chars().count() < 2 {
            continue;
        }
        let by_key = map.entry(path).or_default();
        let vals = by_key.entry(key_norm).or_default();
        if !vals.contains(&val_norm) {
            vals.push(val_norm);
        }
    }
    Ok(map)
}

fn compute_bm25_neighbors(
    index: &VaultIndex,
    source_note: &IndexedNoteRow,
) -> CoreResult<HashMap<String, (f64, Vec<String>)>> {
    let keywords = extract_top_keywords(source_note, 8);
    if keywords.is_empty() {
        return Ok(HashMap::new());
    }

    let fts_query = keywords
        .iter()
        .map(|kw| format!("\"{kw}\""))
        .collect::<Vec<_>>()
        .join(" OR ");

    let mut stmt = index.conn.prepare(
        "
        SELECT f.path, bm25(notes_fts, 10.0, 1.0, 5.0) AS rank, f.title, f.body, f.tags
        FROM notes_fts f
        WHERE notes_fts MATCH ?1 AND f.path != ?2
        ORDER BY rank ASC
        LIMIT 25
        ",
    )?;

    let rows = stmt.query_map(params![fts_query, source_note.path], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut raw_results = Vec::new();
    let mut max_rel = 0.1f64;
    for row in rows {
        let (path, rank, title, body, tags) = row?;
        let rel = (-rank).max(0.1);
        if rel > max_rel {
            max_rel = rel;
        }
        raw_results.push((path, rel, title, body, tags));
    }

    let mut scores = HashMap::new();
    for (path, rel, title, body, tags) in raw_results {
        let normalized_score = ((5.0 + (rel / max_rel) * 15.0) * 10.0).round() / 10.0;
        let combined = fold_diacritics(&format!("{title} {tags} {body}").to_lowercase());
        let matched: Vec<String> = keywords
            .iter()
            .filter(|kw| contains_whole_phrase(&combined, &fold_diacritics(kw)))
            .cloned()
            .collect();
        scores.insert(path, (normalized_score.clamp(5.0, 20.0), matched));
    }

    Ok(scores)
}

fn scan_outgoing_unlinked_mentions(
    source_note: &IndexedNoteRow,
    all_notes: &[IndexedNoteRow],
    terms_per_note: &[Vec<String>],
    on_match: &mut dyn FnMut(usize, WikilinkReplacement),
) {
    let mut in_fenced_code = false;
    let mut fence_marker = '`';

    for (idx, raw_line) in source_note.body.lines().enumerate() {
        let trimmed = raw_line.trim();
        if let Some(marker) = detect_code_fence(trimmed) {
            if !in_fenced_code {
                in_fenced_code = true;
                fence_marker = marker;
                continue;
            } else if marker == fence_marker {
                in_fenced_code = false;
                continue;
            }
        }
        if in_fenced_code || trimmed.is_empty() || is_markdown_heading_line(trimmed) {
            continue;
        }

        let line_no = source_note.body_start_line + idx;
        for (target_idx, terms) in terms_per_note.iter().enumerate() {
            if terms.is_empty() {
                continue;
            }
            if let Some((matched_slice, _, _)) = find_unlinked_term_in_line(raw_line, terms) {
                let target_note = &all_notes[target_idx];
                let replacement_wikilink =
                    if matched_slice.eq_ignore_ascii_case(&target_note.stem)
                        || matched_slice.to_lowercase() == target_note.stem.to_lowercase()
                    {
                        format!("[[{}]]", target_note.stem)
                    } else {
                        format!("[[{}|{matched_slice}]]", target_note.stem)
                    };
                on_match(
                    target_idx,
                    WikilinkReplacement {
                        target_path: target_note.path.clone(),
                        target_stem: target_note.stem.clone(),
                        matched_term: matched_slice.to_string(),
                        replacement_wikilink,
                        line: line_no,
                        context: trimmed.to_string(),
                    },
                );
            }
        }
    }
}

fn collect_candidate_terms(note: &IndexedNoteRow) -> Vec<String> {
    let mut terms = Vec::new();
    push_candidate_term(&mut terms, &note.stem);
    push_candidate_term(&mut terms, &note.title);
    for alias in &note.aliases {
        push_candidate_term(&mut terms, alias);
    }
    terms.sort_by(|a, b| b.len().cmp(&a.len()));
    terms
}

fn push_candidate_term(terms: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.chars().count() < 3 {
        return;
    }
    let lower = trimmed.to_lowercase();
    if !terms.iter().any(|t| t.to_lowercase() == lower) {
        terms.push(trimmed.to_string());
    }
}

fn compute_shared_tags(a_tags: &[String], b_tags: &[String]) -> Vec<String> {
    let mut shared = BTreeSet::new();
    for ta in a_tags {
        let ta_folded = fold_diacritics(ta);
        let ta_root = ta_folded.split('/').next().unwrap_or(&ta_folded);
        for tb in b_tags {
            let tb_folded = fold_diacritics(tb);
            let tb_root = tb_folded.split('/').next().unwrap_or(&tb_folded);
            if ta_folded == tb_folded {
                shared.insert(ta.clone());
            } else if ta_root == tb_root && ta_root.chars().count() >= 3 {
                shared.insert(ta.split('/').next().unwrap_or(ta).to_string());
            }
        }
    }
    shared.into_iter().collect()
}

fn summarize_signals(signals: &[ConnectionSignal]) -> String {
    let mut parts = Vec::new();
    for sig in signals {
        match sig {
            ConnectionSignal::OutgoingUnlinkedMention { matched_term, line, .. } => {
                parts.push(format!("Menciona \"{matched_term}\" sin enlazar (L{line})"));
            }
            ConnectionSignal::IncomingUnlinkedMention { matched_term, .. } => {
                parts.push(format!("Mencionada como \"{matched_term}\" en esa nota"));
            }
            ConnectionSignal::SharedTags { tags } => {
                let tags_fmt: Vec<String> = tags.iter().map(|t| format!("#{t}")).collect();
                parts.push(format!("Tags en común: {}", tags_fmt.join(", ")));
            }
            ConnectionSignal::SharedFields { fields } => {
                parts.push(format!("Metadatos en común: {}", fields.join(", ")));
            }
            ConnectionSignal::LexicalBm25 { matched_keywords, .. } => {
                if matched_keywords.is_empty() {
                    parts.push("Similitud léxica FTS5".to_string());
                } else {
                    parts.push(format!("Términos afines: {}", matched_keywords.join(", ")));
                }
            }
        }
    }
    parts.join(" · ")
}

fn extract_aliases(frontmatter: &BTreeMap<String, FrontmatterValue>) -> Vec<String> {
    let mut aliases = Vec::new();
    for key in ["aliases", "alias"] {
        if let Some(val) = frontmatter.get(key) {
            match val {
                FrontmatterValue::Scalar(s) => {
                    for part in s.split(',') {
                        let trimmed = part.trim();
                        if !trimmed.is_empty() && !aliases.contains(&trimmed.to_string()) {
                            aliases.push(trimmed.to_string());
                        }
                    }
                }
                FrontmatterValue::List(items) => {
                    for item in items {
                        let trimmed = item.trim();
                        if !trimmed.is_empty() && !aliases.contains(&trimmed.to_string()) {
                            aliases.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }
    aliases
}

fn extract_top_keywords(note: &IndexedNoteRow, limit: usize) -> Vec<String> {
    let mut freq: HashMap<String, usize> = HashMap::new();
    add_weighted_tokens(&note.title, 4, &mut freq);
    for h in &note.headings {
        add_weighted_tokens(h, 3, &mut freq);
    }
    add_weighted_tokens(&note.body, 1, &mut freq);

    let mut ranked: Vec<(String, usize)> = freq.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().take(limit).map(|(w, _)| w).collect()
}

fn add_weighted_tokens(text: &str, weight: usize, freq: &mut HashMap<String, usize>) {
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                current.push(lc);
            }
        } else if !current.is_empty() {
            record_keyword_token(&current, weight, freq);
            current.clear();
        }
    }
    if !current.is_empty() {
        record_keyword_token(&current, weight, freq);
    }
}

fn record_keyword_token(token: &str, weight: usize, freq: &mut HashMap<String, usize>) {
    if token.chars().count() < 4 || token.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    let folded = fold_diacritics(token);
    if is_stopword(&folded) {
        return;
    }
    *freq.entry(token.to_string()).or_insert(0) += weight;
}

fn is_stopword(folded_lower: &str) -> bool {
    matches!(
        folded_lower,
        "para" | "como" | "esta" | "este" | "esto" | "estos" | "estas" | "pero" | "porque"
            | "sobre" | "entre" | "desde" | "hasta" | "hacia" | "donde" | "cuando" | "quien"
            | "cual" | "cuales" | "tiene" | "tienen" | "tener" | "hacer" | "puede" | "pueden"
            | "todo" | "toda" | "todos" | "todas" | "otro" | "otra" | "otros" | "otras"
            | "mismo" | "misma" | "cada" | "mucho" | "poco" | "solo" | "mas" | "tambien"
            | "ademas" | "entonces" | "luego" | "nota" | "notas" | "resumen" | "titulo"
            | "with" | "from" | "that" | "this" | "these" | "those" | "there" | "their"
            | "about" | "into" | "over" | "after" | "before" | "between" | "where" | "when"
            | "which" | "while" | "should" | "could" | "would" | "have" | "been" | "more"
            | "also" | "only" | "some" | "such" | "note" | "notes" | "http" | "https"
    )
}

fn contains_whole_phrase(haystack_lower_folded: &str, needle_lower_folded: &str) -> bool {
    if needle_lower_folded.is_empty() {
        return false;
    }
    let mut search_from = 0usize;
    while let Some(rel) = haystack_lower_folded[search_from..].find(needle_lower_folded) {
        let start = search_from + rel;
        let end = start + needle_lower_folded.len();
        let before_ok = haystack_lower_folded[..start]
            .chars()
            .next_back()
            .map(|c| !is_word_boundary_char(c))
            .unwrap_or(true);
        let after_ok = haystack_lower_folded[end..]
            .chars()
            .next()
            .map(|c| !is_word_boundary_char(c))
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        search_from = start + 1;
        while search_from < haystack_lower_folded.len()
            && !haystack_lower_folded.is_char_boundary(search_from)
        {
            search_from += 1;
        }
    }
    false
}

fn merge_frontmatter_and_tags(
    parsed: &ParsedNote,
    plan: &AiEnrichmentPlan,
) -> (BTreeMap<String, FrontmatterValue>, Vec<String>, Vec<String>, bool) {
    let mut merged = parsed.frontmatter.clone();
    let mut added_keys = Vec::new();
    let mut added_tags = Vec::new();
    let mut changed = false;

    for (raw_key, new_val) in &plan.frontmatter_updates {
        let key = raw_key.trim();
        if key.is_empty() {
            continue;
        }
        if key == "tags" || key == "tag" {
            // Normalizar y acumular en sugerencia de tags.
            continue;
        }
        match merged.get(key) {
            None => {
                merged.insert(key.to_string(), new_val.clone());
                added_keys.push(key.to_string());
                changed = true;
            }
            Some(existing) if plan.overwrite_existing_frontmatter && existing != new_val => {
                merged.insert(key.to_string(), new_val.clone());
                added_keys.push(key.to_string());
                changed = true;
            }
            _ => {}
        }
    }

    // Recolectar tags sugeridas tanto desde `plan.suggested_tags` como desde `frontmatter_updates["tags"]`.
    let mut candidate_tags: Vec<String> = plan.suggested_tags.clone();
    for t_key in ["tags", "tag"] {
        if let Some(val) = plan.frontmatter_updates.get(t_key) {
            match val {
                FrontmatterValue::Scalar(s) => {
                    for part in s.split(|c: char| c == ',' || c.is_whitespace()) {
                        candidate_tags.push(part.to_string());
                    }
                }
                FrontmatterValue::List(items) => {
                    candidate_tags.extend(items.iter().cloned());
                }
            }
        }
    }

    let mut new_norm_tags = Vec::new();
    let mut seen_new = HashSet::new();
    for raw_tag in candidate_tags {
        if let Some(norm) = normalize_tag(&raw_tag) {
            if !parsed.has_tag(&norm) && seen_new.insert(norm.clone()) {
                new_norm_tags.push(norm);
            }
        }
    }

    if !new_norm_tags.is_empty() {
        let target_fm_key = if merged.contains_key("tags") || !merged.contains_key("tag") {
            "tags".to_string()
        } else {
            "tag".to_string()
        };

        let mut list = match merged.get(&target_fm_key) {
            Some(FrontmatterValue::List(existing)) => existing.clone(),
            Some(FrontmatterValue::Scalar(s)) => s
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter_map(normalize_tag)
                .collect(),
            None => Vec::new(),
        };

        let mut existing_folded: HashSet<String> = list
            .iter()
            .filter_map(|t| normalize_tag(t))
            .collect();

        for tag in new_norm_tags {
            if existing_folded.insert(tag.clone()) {
                list.push(tag.clone());
                added_tags.push(tag);
            }
        }

        if !added_tags.is_empty() {
            if !merged.contains_key(&target_fm_key) {
                added_keys.push(target_fm_key.clone());
            }
            merged.insert(target_fm_key, FrontmatterValue::List(list));
            changed = true;
        }
    }

    (merged, added_keys, added_tags, changed)
}

fn extract_raw_frontmatter_prefix(clean_content: &str, body_start_line: usize) -> String {
    if body_start_line <= 1 {
        return String::new();
    }
    let mut prefix = String::new();
    for (idx, chunk) in clean_content.split_inclusive('\n').enumerate() {
        if idx + 1 >= body_start_line {
            break;
        }
        prefix.push_str(chunk);
    }
    prefix
}

fn serialize_yaml_frontmatter(
    frontmatter: &BTreeMap<String, FrontmatterValue>,
    line_sep: &str,
) -> String {
    if frontmatter.is_empty() {
        return String::new();
    }

    let mut ordered_keys: Vec<&String> = Vec::with_capacity(frontmatter.len());
    if let Some(k) = frontmatter.get_key_value("title").map(|(k, _)| k) {
        ordered_keys.push(k);
    }
    if let Some(k) = frontmatter.get_key_value("tags").map(|(k, _)| k) {
        ordered_keys.push(k);
    }
    for k in frontmatter.keys() {
        if k != "title" && k != "tags" {
            ordered_keys.push(k);
        }
    }

    let mut out = format!("---{line_sep}");
    for key in ordered_keys {
        if let Some(val) = frontmatter.get(key) {
            match val {
                FrontmatterValue::Scalar(s) => {
                    out.push_str(&format!("{key}: {}{line_sep}", format_yaml_scalar(s)));
                }
                FrontmatterValue::List(items) => {
                    if items.is_empty() {
                        out.push_str(&format!("{key}: []{line_sep}"));
                    } else {
                        out.push_str(&format!("{key}:{line_sep}"));
                        for item in items {
                            out.push_str(&format!("  - {}{line_sep}", format_yaml_scalar(item)));
                        }
                    }
                }
            }
        }
    }
    out.push_str(&format!("---{line_sep}"));
    out
}

fn format_yaml_scalar(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = s.contains(": ")
        || s.contains('#')
        || s.starts_with(['-', '[', '{', '*', '&', '!', '%', '@', '"', '\''])
        || s.ends_with(':');
    if !needs_quotes {
        s.to_string()
    } else if !s.contains('"') {
        format!("\"{s}\"")
    } else {
        let safe = s.replace('\'', "");
        format!("'{safe}'")
    }
}

fn collect_existing_link_keys(parsed: &ParsedNote) -> HashSet<String> {
    let mut keys = HashSet::new();
    for link in &parsed.wikilinks {
        if let Some(norm) = normalize_link_target(&link.target) {
            keys.insert(norm.clone());
            let stem_only = norm.rsplit('/').next().unwrap_or(&norm).to_string();
            keys.insert(stem_only);
        }
    }
    keys
}

fn target_key_from_path_or_stem(target_path: &str, target_stem: &str) -> String {
    if !target_path.trim().is_empty() {
        let (path_no_ext_lower, _) = path_lookup_keys(target_path);
        path_no_ext_lower
    } else {
        normalize_link_target(target_stem).unwrap_or_default()
    }
}

fn format_replacement_wikilink(
    target_stem: &str,
    target_path: &str,
    matched_term: &str,
    raw_replacement: &str,
) -> String {
    let trimmed = raw_replacement.trim();
    if trimmed.starts_with("[[") && trimmed.ends_with("]]") && trimmed.len() > 4 {
        return trimmed.to_string();
    }
    let stem = if !target_stem.trim().is_empty() {
        target_stem.trim().to_string()
    } else {
        extract_stem(target_path)
    };
    if matched_term.to_lowercase() == stem.to_lowercase() {
        format!("[[{stem}]]")
    } else {
        format!("[[{stem}|{matched_term}]]")
    }
}

struct BodyLine {
    line_no: usize,
    text: String,
    newline: String,
    eligible: bool,
}

fn split_body_lines_with_eligibility(raw_body: &str, body_start_line: usize) -> Vec<BodyLine> {
    let mut lines = Vec::new();
    let mut in_fenced_code = false;
    let mut fence_marker = '`';
    let mut in_html_comment = false;

    for (idx, chunk) in raw_body.split_inclusive('\n').enumerate() {
        let line_no = body_start_line + idx;
        let (text, newline) = if let Some(s) = chunk.strip_suffix("\r\n") {
            (s.to_string(), "\r\n".to_string())
        } else if let Some(s) = chunk.strip_suffix('\n') {
            (s.to_string(), "\n".to_string())
        } else {
            (chunk.to_string(), String::new())
        };

        let trimmed = text.trim();
        let mut eligible = true;

        if let Some(marker) = detect_code_fence(trimmed) {
            if !in_fenced_code {
                in_fenced_code = true;
                fence_marker = marker;
            } else if marker == fence_marker {
                in_fenced_code = false;
            }
            eligible = false;
        } else if in_fenced_code {
            eligible = false;
        }

        if trimmed.contains("<!--") && !trimmed.contains("-->") {
            in_html_comment = true;
            eligible = false;
        } else if in_html_comment {
            eligible = false;
            if trimmed.contains("-->") {
                in_html_comment = false;
            }
        } else if trimmed.starts_with("<!--") && trimmed.ends_with("-->") {
            eligible = false;
        }

        if is_markdown_heading_line(trimmed) {
            eligible = false;
        }

        lines.push(BodyLine {
            line_no,
            text,
            newline,
            eligible,
        });
    }

    lines
}

fn inject_single_wikilink(
    lines: &mut [BodyLine],
    preferred_line: usize,
    term: &str,
    wikilink_str: &str,
    template_rep: &WikilinkReplacement,
) -> Option<WikilinkReplacement> {
    let terms = vec![term.to_string()];

    // 1. Intentar primero en la línea sugerida exacta.
    if preferred_line > 0 {
        if let Some(line) = lines.iter_mut().find(|l| l.line_no == preferred_line && l.eligible) {
            if let Some(rep) =
                try_replace_in_body_line(line, &terms, wikilink_str, template_rep)
            {
                return Some(rep);
            }
        }
    }

    // 2. Fallback seguro: primera aparición elegible en todo el cuerpo.
    for line in lines.iter_mut() {
        if !line.eligible {
            continue;
        }
        if let Some(rep) = try_replace_in_body_line(line, &terms, wikilink_str, template_rep) {
            return Some(rep);
        }
    }

    None
}

fn try_replace_in_body_line(
    line: &mut BodyLine,
    terms: &[String],
    wikilink_str: &str,
    template_rep: &WikilinkReplacement,
) -> Option<WikilinkReplacement> {
    let (matched_slice, start_byte, end_byte) = find_unlinked_term_in_line(&line.text, terms)?;
    let _ = compute_protected_ranges(&line.text); // Garantiza consistencia con rangos protegidos
    let matched_term = matched_slice.to_string();

    let mut updated = String::with_capacity(line.text.len() + wikilink_str.len());
    updated.push_str(&line.text[..start_byte]);
    updated.push_str(wikilink_str);
    updated.push_str(&line.text[end_byte..]);
    line.text = updated;

    Some(WikilinkReplacement {
        target_path: template_rep.target_path.clone(),
        target_stem: template_rep.target_stem.clone(),
        matched_term,
        replacement_wikilink: wikilink_str.to_string(),
        line: line.line_no,
        context: line.text.trim().to_string(),
    })
}

fn append_related_connections_section(
    body_output: &str,
    bullet_lines: &[String],
    line_sep: &str,
) -> String {
    let lines: Vec<&str> = body_output.lines().collect();
    let heading_idx = lines
        .iter()
        .position(|l| l.trim().eq_ignore_ascii_case("## Conexiones Relacionadas"));

    let bullets_joined = bullet_lines.join(line_sep);
    if let Some(h_idx) = heading_idx {
        // Insertar antes del próximo encabezado de nivel 1 o 2, o al final de esa sección.
        let mut insert_after = lines.len();
        for i in (h_idx + 1)..lines.len() {
            let t = lines[i].trim();
            if t.starts_with("## ") || t.starts_with("# ") {
                insert_after = i;
                break;
            }
        }
        let mut out_lines: Vec<String> = lines[..insert_after]
            .iter()
            .map(|s| s.to_string())
            .collect();
        while out_lines.last().is_some_and(|s| s.trim().is_empty()) {
            out_lines.pop();
        }
        for b in bullet_lines {
            out_lines.push(b.clone());
        }
        for rest in &lines[insert_after..] {
            out_lines.push((*rest).to_string());
        }
        return format!("{}{line_sep}", out_lines.join(line_sep));
    }

    let trimmed_end = body_output.trim_end_matches(['\r', '\n']);
    if trimmed_end.is_empty() {
        format!("## Conexiones Relacionadas{line_sep}{bullets_joined}{line_sep}")
    } else {
        format!("{trimmed_end}{line_sep}{line_sep}## Conexiones Relacionadas{line_sep}{bullets_joined}{line_sep}")
    }
}

fn build_line_diff_summary(before: &str, after: &str) -> Vec<String> {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let before_set: HashSet<&str> = before_lines.iter().copied().collect();
    let after_set: HashSet<&str> = after_lines.iter().copied().collect();

    let mut diff = Vec::new();
    for line in before_lines {
        if !after_set.contains(line) {
            diff.push(format!("- {line}"));
        }
    }
    for line in after_lines {
        if !before_set.contains(line) {
            diff.push(format!("+ {line}"));
        }
    }
    diff
}

fn is_markdown_heading_line(trimmed: &str) -> bool {
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes)
        && (trimmed.len() == hashes || trimmed.as_bytes()[hashes].is_ascii_whitespace())
}

fn extract_stem(rel_path: &str) -> String {
    let norm = rel_path.replace('\\', "/");
    let no_ext = norm
        .strip_suffix(".md")
        .or_else(|| norm.strip_suffix(".MD"))
        .unwrap_or(&norm);
    Path::new(no_ext)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(no_ext)
        .to_string()
}

fn extract_folder(rel_path: &str) -> String {
    rel_path
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

fn truncate_sample(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn pick_canvas_sides(fx: i64, fy: i64, tx: i64, ty: i64) -> (&'static str, &'static str) {
    let dx = tx - fx;
    let dy = ty - fy;
    if dx.abs() >= dy.abs() {
        if dx >= 0 {
            ("right", "left")
        } else {
            ("left", "right")
        }
    } else if dy >= 0 {
        ("bottom", "top")
    } else {
        ("top", "bottom")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static AI_TEST_SEQ: AtomicU64 = AtomicU64::new(1);

    struct TempAiVault {
        path: PathBuf,
    }

    impl TempAiVault {
        fn new(prefix: &str) -> Self {
            let id = AI_TEST_SEQ.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "romsidian_ai_{}_{}_{}",
                prefix,
                std::process::id(),
                id
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("Crear directorio temporal para ai_linker");
            Self { path }
        }
    }

    impl Drop for TempAiVault {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn setup_sample_vault() -> (TempAiVault, Vault, VaultIndex) {
        let dir = TempAiVault::new("pkm");
        let vault = Vault::open(&dir.path).expect("Abrir bóveda temporal");

        vault
            .create_note(
                "Conceptos/Política Monetaria.md",
                "---\ntitle: Política Monetaria\naliases: [emisión monetaria, tasa de interés]\ntags: [economia, macro]\n---\n# Política Monetaria\nControl de la inflación y base monetaria por parte del banco central.\n",
            )
            .unwrap();

        vault
            .create_note(
                "Conceptos/Inflación Subyacente.md",
                "---\ntitle: Inflación Subyacente\ntags: [economia/precios]\nauthor: Roma\n---\n# Inflación Subyacente\nMedición de variación de precios excluyendo estacionales y regulados.\n",
            )
            .unwrap();

        vault
            .create_note(
                "Notas/Huérfana Aislada.md",
                "---\ntitle: Huérfana Aislada\n---\n# Huérfana Aislada\nBorrador sin etiquetas ni enlaces salientes.\n",
            )
            .unwrap();

        vault
            .create_note(
                "Inbox/Resumen Video YouTube.md",
                "En este video analizamos cómo la Política Monetaria impacta en los precios.\nTema:: Macroeconomia\nAuthor:: Roma\nTambién explica el rol del banco central y la variación de precios regulados.\n`No tocar Política Monetaria en código`\nhttps://example.com/Politica-Monetaria/Inflacion\n",
            )
            .unwrap();

        vault
            .create_note(
                "Macro/Informe 2026.md",
                "---\ntitle: Informe Macro 2026\nauthor: Roma\ntags: [economia/macro]\n---\n# Informe Macro 2026\nReferencia sin enlazar a Resumen Video YouTube y Inflación Subyacente.\n",
            )
            .unwrap();

        let (index, _) = VaultIndex::open_and_sync(&vault).expect("Sincronizar índice");
        (dir, vault, index)
    }

    #[test]
    fn knowledge_map_summarizes_notes_tags_frontmatter_and_orphans() {
        let (_dir, _vault, index) = setup_sample_vault();
        let map = build_vault_knowledge_map(&index).expect("Construir mapa de bóveda");

        assert_eq!(map.total_notes, 5);
        assert!(map
            .orphan_paths
            .contains(&"Notas/Huérfana Aislada.md".to_string()));
        assert!(map.folders.contains(&"Conceptos".to_string()));
        assert!(map.folders.contains(&"Inbox".to_string()));
        assert!(map.tag_taxonomy.iter().any(|t| t.tag == "economia"));
        assert!(map
            .frontmatter_schema
            .iter()
            .any(|k| k.key == "author" && k.count == 2));

        let compact = map.to_compact_prompt_context(10);
        assert!(compact.contains("# Mapa de Bóveda (5 notas"));
        assert!(compact.contains("[[Política Monetaria]]"));
    }

    #[test]
    fn suggest_connections_combines_unlinked_mentions_tags_fields_and_bm25() {
        let (_dir, _vault, index) = setup_sample_vault();
        let candidates = suggest_note_connections(&index, "Inbox/Resumen Video YouTube.md", 10)
            .expect("Sugerir conexiones");

        assert!(
            !candidates.is_empty(),
            "Debe detectar múltiples notas candidatas"
        );

        let pm = candidates
            .iter()
            .find(|c| c.target_path == "Conceptos/Política Monetaria.md")
            .expect("Debe sugerir Política Monetaria por mención no enlazada");
        assert!(!pm.already_linked);
        assert!(!pm.inline_replacements.is_empty());
        assert_eq!(pm.inline_replacements[0].line, 1);

        let inf = candidates
            .iter()
            .find(|c| c.target_path == "Conceptos/Inflación Subyacente.md")
            .expect("Debe sugerir Inflación Subyacente por author compartido y BM25");
        assert!(inf
            .signals
            .iter()
            .any(|s| matches!(s, ConnectionSignal::SharedFields { .. })));
        assert!(inf
            .signals
            .iter()
            .any(|s| matches!(s, ConnectionSignal::LexicalBm25 { .. })));
    }

    #[test]
    fn preview_and_apply_enrichment_is_idempotent_and_protects_code_and_urls() {
        let (_dir, vault, mut index) = setup_sample_vault();

        let content_with_code_and_heading = "\
---
title: Borrador Diario
custom_user_key: conservar-siempre
tags:
  - diario
---
# Política Monetaria y Código

El texto menciona Política Monetaria fuera de código, y también tasas con alias tasa de interés.
Código protegido: `Política Monetaria no se enlaza aquí`.
URL protegida: https://example.com/Política%20Monetaria y [link](https://example.com).
Ya enlazada [[Conceptos/Inflación Subyacente|Inflación Subyacente]].

```markdown
Política Monetaria dentro de bloque cercado jamás cambia.
```
";
        vault
            .create_note("Inbox/Nota Protegida.md", content_with_code_and_heading)
            .unwrap();
        index.index_note(&vault, "Inbox/Nota Protegida.md").unwrap();

        let mut fm_updates = BTreeMap::new();
        fm_updates.insert(
            "source".to_string(),
            FrontmatterValue::Scalar("youtube".to_string()),
        );
        fm_updates.insert(
            "custom_user_key".to_string(),
            FrontmatterValue::Scalar("NO_SOBRESCRIBIR".to_string()),
        );

        let plan = AiEnrichmentPlan {
            frontmatter_updates: fm_updates,
            overwrite_existing_frontmatter: false,
            suggested_tags: vec!["#Economía".to_string(), "diario".to_string()],
            inline_wikilinks: vec![
                WikilinkReplacement {
                    target_path: "Conceptos/Política Monetaria.md".to_string(),
                    target_stem: "Política Monetaria".to_string(),
                    matched_term: "Política Monetaria".to_string(),
                    replacement_wikilink: "[[Política Monetaria]]".to_string(),
                    line: 9,
                    context: String::new(),
                },
                WikilinkReplacement {
                    target_path: "Conceptos/Inflación Subyacente.md".to_string(),
                    target_stem: "Inflación Subyacente".to_string(),
                    matched_term: "Inflación Subyacente".to_string(),
                    replacement_wikilink: "[[Inflación Subyacente]]".to_string(),
                    line: 12,
                    context: String::new(),
                },
            ],
            related_connections: vec![RelatedConnectionLink {
                target_path: "Macro/Informe 2026.md".to_string(),
                target_title: "Informe Macro 2026".to_string(),
                relation: Some("Mismo autor Roma".to_string()),
            }],
            append_related_section: true,
        };

        let first = apply_ai_enrichment(&vault, &mut index, "Inbox/Nota Protegida.md", &plan)
            .expect("Primera aplicación debe tener éxito");
        assert!(first.preview.changed);
        let text = &first.note.content;

        // Protecciones clave
        assert!(text.contains("custom_user_key: conservar-siempre"));
        assert!(text.contains("source: youtube"));
        assert!(text.contains("- economía"));
        assert!(text.contains("# Política Monetaria y Código\n"));
        assert!(text.contains("menciona [[Política Monetaria]] fuera de código"));
        assert!(text.contains("`Política Monetaria no se enlaza aquí`"));
        assert!(text.contains("https://example.com/Política%20Monetaria"));
        assert!(text.contains("Política Monetaria dentro de bloque cercado jamás cambia."));
        assert!(text.contains("## Conexiones Relacionadas"));
        assert!(text.contains("- [[Informe 2026|Informe Macro 2026]] — Mismo autor Roma"));

        // Segunda aplicación idéntica no debe producir ningún cambio ni duplicar enlaces/tags.
        let second = apply_ai_enrichment(&vault, &mut index, "Inbox/Nota Protegida.md", &plan)
            .expect("Segunda aplicación debe ser idempotente");
        assert!(
            !second.preview.changed,
            "Aplicar el mismo plan dos veces debe ser 100% idempotente"
        );
        assert_eq!(second.note.content, first.note.content);
    }

    #[test]
    fn deterministic_enrichment_plan_auto_connects_unlinked_source_note() {
        let (_dir, vault, mut index) = setup_sample_vault();
        let plan = build_deterministic_enrichment_plan(
            &vault,
            &index,
            "Inbox/Resumen Video YouTube.md",
            5,
            true,
        )
        .expect("Construir plan determinista");

        assert!(plan.frontmatter_updates.contains_key("title"));
        assert!(plan
            .inline_wikilinks
            .iter()
            .any(|w| w.target_stem == "Política Monetaria"));

        let applied =
            apply_ai_enrichment(&vault, &mut index, "Inbox/Resumen Video YouTube.md", &plan)
                .expect("Aplicar plan determinista");
        assert!(applied.preview.changed);
        assert!(applied.note.content.contains("[[Política Monetaria]]"));

        // El índice SQLite debe reflejar inmediatamente el nuevo enlace saliente y backlink.
        let backlinks = index
            .backlinks_for("Conceptos/Política Monetaria.md")
            .expect("Consultar backlinks tras auto-conexión");
        assert!(backlinks
            .iter()
            .any(|b| b.source_path == "Inbox/Resumen Video YouTube.md"));
    }

    #[test]
    fn generate_knowledge_canvas_creates_valid_json_canvas_with_nodes_and_edges() {
        let (_dir, vault, mut index) = setup_sample_vault();
        let plan = build_deterministic_enrichment_plan(
            &vault,
            &index,
            "Inbox/Resumen Video YouTube.md",
            5,
            true,
        )
        .unwrap();
        apply_ai_enrichment(&vault, &mut index, "Inbox/Resumen Video YouTube.md", &plan).unwrap();

        let result = generate_knowledge_canvas(
            &vault,
            &index,
            &KnowledgeCanvasRequest {
                canvas_path: "Mapas/Mapa Macro.canvas".to_string(),
                title: Some("Mapa Macro y Fuentes".to_string()),
                seed_note_paths: vec!["Inbox/Resumen Video YouTube.md".to_string()],
                include_neighbors: true,
                max_nodes: 6,
            },
        )
        .expect("Generar lienzo .canvas");

        assert_eq!(result.canvas.relative_path, "Mapas/Mapa Macro.canvas");
        assert!(result.node_count >= 3);
        assert!(result.edge_count >= 1);

        let parsed: serde_json::Value =
            serde_json::from_str(&result.canvas.content).expect("JSON Canvas válido");
        assert!(parsed.get("nodes").and_then(|v| v.as_array()).is_some());
        assert!(parsed.get("edges").and_then(|v| v.as_array()).is_some());
    }
}
