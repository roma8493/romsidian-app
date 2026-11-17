//! Parser de Markdown conectado para ROMSIDIAN (`[RF-03]`, `[RF-10]`, `[RF-12]`).
//!
//! Extrae de cada nota Markdown:
//! - Bloque YAML superior (`frontmatter`) en formato clave-valor (escalares y listas).
//! - Enlaces internos salientes `[[destino]]`, `[[destino|alias]]`, `[[destino#sección|alias]]`,
//!   referencias a bloques `[[destino#^id]]` y transclusiones `![[...]]` con número de línea (1-based)
//!   y fragmento de contexto para *backlinks*.
//! - Etiquetas `#etiqueta` y jerárquicas `#padre/hijo` (con soporte completo para tildes y `ñ`),
//!   unificando las del `frontmatter` y las del cuerpo e ignorando encabezados Markdown,
//!   bloques de código (` ``` ` / `` ` ``), fragmentos de URL y números puros.
//! - Campos inline estilo Dataview (`Clave:: Valor`, `[clave:: valor]`, `(clave:: valor)`).
//! - Listas de tareas Markdown (`- [ ]` y `- [x]`) con sus etiquetas asociadas.
//! - Estructura de encabezados (`#` a `######`) e identificadores de bloque al final de línea (`^id`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Valor de una propiedad dentro del bloque `frontmatter` YAML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FrontmatterValue {
    /// Valor escalar de texto, fecha, número o booleano.
    Scalar(String),
    /// Lista de valores (por ejemplo, `tags` o `aliases`).
    List(Vec<String>),
}

impl FrontmatterValue {
    /// Devuelve el valor como `&str` si es un escalar.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Scalar(s) => Some(s.as_str()),
            Self::List(_) => None,
        }
    }

    /// Devuelve los elementos como slice si es una lista, o un slice vacío si es escalar.
    pub fn as_list(&self) -> &[String] {
        match self {
            Self::Scalar(_) => &[],
            Self::List(items) => items.as_slice(),
        }
    }
}

/// Tipo de sintaxis empleada para declarar un campo inline de Dataview (`[RF-12]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InlineFieldSyntax {
    /// Sintaxis de línea completa o ítem de lista (`Clave:: Valor`).
    Bare,
    /// Sintaxis entre corchetes (`[clave:: valor]`).
    Bracket,
    /// Sintaxis entre paréntesis con clave oculta en lectura (`(clave:: valor)`).
    Paren,
}

impl InlineFieldSyntax {
    /// Representación textual corta para persistencia en SQLite.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bare => "bare",
            Self::Bracket => "bracket",
            Self::Paren => "paren",
        }
    }
}

/// Campo de metadatos inline extraído del cuerpo de la nota (`Clave:: Valor`, `[k:: v]`, `(k:: v)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineField {
    /// Nombre limpio de la clave (sin formato Markdown envolvente como `**` o `_`).
    pub key: String,
    /// Clave canónica normalizada (minúsculas Unicode y espacios/guiones bajos convertidos a `-`).
    pub key_norm: String,
    /// Valor del campo recortado.
    pub value: String,
    /// Variante sintáctica con la que fue escrito.
    pub syntax: InlineFieldSyntax,
    /// Número de línea (basado en 1) dentro del archivo original.
    pub line: usize,
}

/// Tarea interactiva extraída de listas Markdown (`- [ ]` o `- [x]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    /// Texto descriptivo de la tarea (sin el prefijo `- [ ] `).
    pub text: String,
    /// `true` si la casilla está completada (`[x]` o `[X]`).
    pub completed: bool,
    /// Número de línea (basado en 1) dentro del archivo original.
    pub line: usize,
    /// Etiquetas normalizadas (`#tag`) presentes dentro de la línea de la tarea.
    pub tags: Vec<String>,
}

/// Encabezado Markdown (`#` a `######`) extraído del cuerpo de la nota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadingItem {
    /// Nivel jerárquico del encabezado (`1..=6`).
    pub level: u8,
    /// Texto del encabezado limpio (sin `#` iniciales ni identificador `^bloque` final).
    pub text: String,
    /// Número de línea (basado en 1) dentro del archivo original.
    pub line: usize,
}

/// Identificador de bloque (`^id-bloque`) anclado al final de una línea o párrafo (`[RF-10]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRef {
    /// Identificador del bloque sin el prefijo `^` (por ejemplo, `"definicion-pkm"`).
    pub id: String,
    /// Número de línea (basado en 1) dentro del archivo original.
    pub line: usize,
    /// Contenido limpio de la línea o bloque sin el sufijo `^id`.
    pub text: String,
}

/// Representa un enlace interno `[[wikilink]]` o transclusión `![[...]]` extraído de una nota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiLink {
    /// Nombre o ruta relativa de la nota destino (sin `#sección` ni `|alias`).
    pub target: String,
    /// Encabezado o sub-objetivo dentro de la nota destino (`[[Nota#Sección]]` o `[[Nota#^bloque]]`).
    pub heading: Option<String>,
    /// Identificador de bloque limpio (sin `^`) cuando el enlace apunta a `[[Nota#^bloque]]`.
    pub block_id: Option<String>,
    /// Texto visible opcional (`[[Nota|Texto visible]]`).
    pub alias: Option<String>,
    /// Número de línea (basado en 1) dentro del archivo original donde aparece el enlace.
    pub line: usize,
    /// Línea de contexto recortada para mostrar en el panel de *backlinks*.
    pub context: String,
    /// `true` si el enlace fue precedido por `!` (`![[recurso]]`).
    pub is_embed: bool,
}

/// Resultado estructurado tras analizar una nota Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedNote {
    /// Título detectado (priorizando `title` del `frontmatter` y luego el primer `# Encabezado`).
    pub title: Option<String>,
    /// Mapa ordenado de propiedades clave-valor del `frontmatter` YAML.
    pub frontmatter: BTreeMap<String, FrontmatterValue>,
    /// Contenido del cuerpo Markdown sin el bloque `frontmatter` superior.
    pub body: String,
    /// Línea inicial (1-based) donde comienza `body` dentro del archivo completo.
    pub body_start_line: usize,
    /// Enlaces internos `[[wikilinks]]` y transclusiones `![[...]]` salientes en orden de aparición.
    pub wikilinks: Vec<WikiLink>,
    /// Etiquetas únicas normalizadas (sin `#` inicial, en minúsculas, ordenadas alfabéticamente).
    pub tags: Vec<String>,
    /// Campos inline estilo Dataview (`Clave:: Valor`, `[k:: v]`, `(k:: v)`).
    pub inline_fields: Vec<InlineField>,
    /// Ítems de tareas (`- [ ]` / `- [x]`) extraídos del cuerpo.
    pub tasks: Vec<TaskItem>,
    /// Encabezados (`#` a `######`) en orden de aparición.
    pub headings: Vec<HeadingItem>,
    /// Bloques referenciables (`^id`) en orden de aparición.
    pub blocks: Vec<BlockRef>,
}

impl ParsedNote {
    /// Verifica si la nota contiene una etiqueta dada (acepta con o sin `#` inicial y sin importar mayúsculas).
    pub fn has_tag(&self, query_tag: &str) -> bool {
        if let Some(norm) = normalize_tag(query_tag) {
            self.tags.binary_search(&norm).is_ok()
        } else {
            false
        }
    }

    /// Extrae el fragmento Markdown perteneciente a un encabezado específico (`![[Nota#Encabezado]]`),
    /// abarcando desde la línea del encabezado hasta antes del siguiente encabezado de nivel igual o superior.
    pub fn extract_section(&self, heading_query: &str) -> Option<String> {
        let clean_query = heading_query.trim().trim_start_matches('#').trim();
        if clean_query.is_empty() {
            return None;
        }
        let query_lower = clean_query.to_lowercase();

        let target_heading = self
            .headings
            .iter()
            .find(|h| h.text.to_lowercase() == query_lower)?;

        let next_boundary_line = self
            .headings
            .iter()
            .find(|h| h.line > target_heading.line && h.level <= target_heading.level)
            .map(|h| h.line);

        let mut section_lines = Vec::new();
        for (idx, line) in self.body.lines().enumerate() {
            let line_no = self.body_start_line + idx;
            if line_no < target_heading.line {
                continue;
            }
            if let Some(end_line) = next_boundary_line {
                if line_no >= end_line {
                    break;
                }
            }
            section_lines.push(line);
        }

        let extracted = section_lines.join("\n").trim().to_string();
        if extracted.is_empty() {
            None
        } else {
            Some(extracted)
        }
    }

    /// Extrae el texto de un bloque identificado por `^id` (`![[Nota#^id]]`).
    pub fn extract_block(&self, block_query: &str) -> Option<String> {
        let clean_id = block_query
            .trim()
            .trim_start_matches('#')
            .trim_start_matches('^')
            .trim();
        if clean_id.is_empty() {
            return None;
        }
        let id_lower = clean_id.to_lowercase();
        self.blocks
            .iter()
            .find(|b| b.id.to_lowercase() == id_lower)
            .map(|b| b.text.clone())
    }
}

/// Analiza el contenido completo de una nota Markdown y extrae `frontmatter`, `[[wikilinks]]`,
/// `#etiquetas`, campos inline Dataview, tareas, encabezados, bloques `^id`, título y cuerpo limpio.
pub fn parse_note(content: &str) -> ParsedNote {
    let clean_content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let (frontmatter, body_slice, body_start_line) = extract_frontmatter(clean_content);

    let mut tag_set = BTreeSet::new();

    // 1. Extraer etiquetas declaradas en el frontmatter (`tags` o `tag`).
    for key in ["tags", "tag"] {
        if let Some(val) = frontmatter.get(key) {
            match val {
                FrontmatterValue::List(items) => {
                    for item in items {
                        if let Some(norm) = normalize_tag(item) {
                            tag_set.insert(norm);
                        }
                    }
                }
                FrontmatterValue::Scalar(raw) => {
                    for token in raw.split(|c: char| c == ',' || c.is_whitespace()) {
                        if let Some(norm) = normalize_tag(token) {
                            tag_set.insert(norm);
                        }
                    }
                }
            }
        }
    }

    // 2. Recorrer el cuerpo línea por línea ignorando bloques de código cercados (` ``` ` o `~~~`).
    let mut wikilinks = Vec::new();
    let mut inline_fields = Vec::new();
    let mut tasks = Vec::new();
    let mut headings = Vec::new();
    let mut blocks = Vec::new();
    let mut first_h1: Option<String> = None;
    let mut in_fenced_code = false;
    let mut fence_marker: char = '`';

    for (idx, raw_line) in body_slice.lines().enumerate() {
        let line_number = body_start_line + idx;
        let trimmed = raw_line.trim();

        // Detectar apertura o cierre de bloque de código cercado.
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

        if in_fenced_code {
            continue;
        }

        // Extraer encabezados Markdown (`#` a `######`).
        if let Some(heading_item) = extract_heading_item(trimmed, line_number) {
            if heading_item.level == 1 && first_h1.is_none() {
                first_h1 = Some(heading_item.text.clone());
            }
            headings.push(heading_item);
        }

        // Extraer identificador de bloque al final de la línea (`^id-bloque`).
        if let Some(block_ref) = extract_block_ref(trimmed, line_number) {
            blocks.push(block_ref);
        }

        // Enmascarar segmentos de código en línea (`...`) para no extraer enlaces ni tags falsos.
        let masked_line = mask_inline_code(raw_line);
        let context = trimmed.to_string();

        // Extraer [[wikilinks]] y ![[transclusiones]] en la línea actual.
        extract_wikilinks_from_line(&masked_line, line_number, &context, &mut wikilinks);

        // Extraer #etiquetas en la línea actual.
        extract_tags_from_line(&masked_line, &mut tag_set);

        // Extraer tareas Markdown (`- [ ]` / `- [x]`).
        if let Some(task_item) = extract_task_from_line(raw_line, &masked_line, line_number) {
            tasks.push(task_item);
        }

        // Extraer campos inline Dataview (`Clave:: Valor`, `[k:: v]`, `(k:: v)`).
        extract_inline_fields_from_line(raw_line, line_number, &mut inline_fields);
    }

    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or(first_h1);

    ParsedNote {
        title,
        frontmatter,
        body: body_slice.to_string(),
        body_start_line,
        wikilinks,
        tags: tag_set.into_iter().collect(),
        inline_fields,
        tasks,
        headings,
        blocks,
    }
}

/// Normaliza el nombre de un campo (`frontmatter` o `inline field`) al formato canónico de Dataview:
/// limpia formato Markdown envolvente (`*`, `_`, `~`, `` ` ``), convierte a minúsculas Unicode y
/// reemplaza espacios o guiones bajos consecutivos por `-`.
pub fn normalize_field_key(raw_key: &str) -> Option<String> {
    let cleaned = clean_field_key(raw_key)?;
    let mut norm = String::with_capacity(cleaned.len());
    let mut prev_dash = false;

    for ch in cleaned.chars() {
        if ch.is_whitespace() || ch == '_' || ch == '-' {
            if !prev_dash && !norm.is_empty() {
                norm.push('-');
                prev_dash = true;
            }
        } else {
            for lower in ch.to_lowercase() {
                norm.push(lower);
            }
            prev_dash = false;
        }
    }

    let trimmed = norm.trim_matches('-').to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Normaliza una etiqueta eliminando el prefijo `#` (si existe), recortando espacios y barras
/// sobrantes, y convirtiendo a minúsculas Unicode. Devuelve `None` si la etiqueta no es válida.
pub fn normalize_tag(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('#').trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    if !is_valid_tag_body(trimmed) {
        return None;
    }

    Some(trimmed.to_lowercase())
}

/// Extrae el bloque YAML superior (`--- ... ---`) si está al inicio del documento.
/// Devuelve `(mapa_frontmatter, slice_del_cuerpo, linea_inicial_del_cuerpo_1_based)`.
pub(crate) fn extract_frontmatter(content: &str) -> (BTreeMap<String, FrontmatterValue>, &str, usize) {
    let mut lines = content.split_inclusive('\n');
    let Some(first_line) = lines.next() else {
        return (BTreeMap::new(), content, 1);
    };

    if first_line.trim_end_matches(['\r', '\n']) != "---" {
        return (BTreeMap::new(), content, 1);
    }

    let mut byte_offset = first_line.len();
    let mut line_count = 1usize;
    let mut yaml_end_offset: Option<usize> = None;
    let mut body_offset: usize = byte_offset;

    for line in lines {
        line_count += 1;
        let trimmed_line = line.trim_end_matches(['\r', '\n']).trim_end();
        if trimmed_line == "---" || trimmed_line == "..." {
            yaml_end_offset = Some(byte_offset);
            body_offset = byte_offset + line.len();
            break;
        }
        byte_offset += line.len();
    }

    let Some(end_offset) = yaml_end_offset else {
        // No se cerró el bloque YAML: tratar todo el documento como cuerpo normal.
        return (BTreeMap::new(), content, 1);
    };

    let yaml_block = &content[first_line.len()..end_offset];
    let parsed_map = parse_simple_yaml(yaml_block);
    let body_slice = &content[body_offset..];
    let body_start_line = line_count + 1;

    (parsed_map, body_slice, body_start_line)
}

/// Parser ligero de YAML orientado a bloques `frontmatter` de notas Markdown.
/// Soporta pares `clave: valor`, strings entrecomillados, listas inline `[a, b]`
/// y listas multilínea con `- item`.
fn parse_simple_yaml(yaml: &str) -> BTreeMap<String, FrontmatterValue> {
    let mut map = BTreeMap::new();
    let mut current_key: Option<String> = None;
    let mut current_list: Vec<String> = Vec::new();

    for raw_line in yaml.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // ¿Es un ítem de lista multilínea (`- valor`) bajo la clave activa?
        if let Some(rest) = trimmed.strip_prefix('-') {
            if current_key.is_some() {
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    let item = unquote_yaml_value(rest.trim());
                    if !item.is_empty() {
                        current_list.push(item);
                    }
                    continue;
                }
            }
        }

        // Si veníamos acumulando una lista para `current_key`, guardarla antes de procesar la nueva clave.
        if let Some(prev_key) = current_key.take() {
            if !current_list.is_empty() {
                map.insert(
                    prev_key,
                    FrontmatterValue::List(std::mem::take(&mut current_list)),
                );
            }
        }

        // Parsear `clave: valor`
        if let Some((raw_key, raw_val)) = trimmed.split_once(':') {
            let key = raw_key.trim().to_string();
            if key.is_empty() {
                continue;
            }
            let val = raw_val.trim();

            if val.is_empty() {
                // El valor puede venir como lista multilínea en las siguientes líneas.
                current_key = Some(key);
                current_list.clear();
            } else if val.starts_with('[') && val.ends_with(']') {
                let inner = &val[1..val.len() - 1];
                let items: Vec<String> = inner
                    .split(',')
                    .map(|s| unquote_yaml_value(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect();
                map.insert(key, FrontmatterValue::List(items));
            } else {
                map.insert(key, FrontmatterValue::Scalar(unquote_yaml_value(val)));
            }
        }
    }

    if let Some(last_key) = current_key.take() {
        if !current_list.is_empty() {
            map.insert(last_key, FrontmatterValue::List(current_list));
        } else {
            map.insert(last_key, FrontmatterValue::Scalar(String::new()));
        }
    }

    map
}

/// Quita comillas simples o dobles envolventes de un valor YAML escalar.
fn unquote_yaml_value(val: &str) -> String {
    let bytes = val.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        val[1..val.len() - 1].to_string()
    } else {
        val.to_string()
    }
}

/// Detecta si una línea inicia o cierra un bloque de código cercado (` ``` ` o `~~~`).
pub(crate) fn detect_code_fence(trimmed_line: &str) -> Option<char> {
    if trimmed_line.starts_with("```") {
        Some('`')
    } else if trimmed_line.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// Extrae un encabezado Markdown (`#` a `######`) limpiando un eventual `^id-bloque` al final.
fn extract_heading_item(trimmed_line: &str, line_number: usize) -> Option<HeadingItem> {
    let level = trimmed_line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }

    let rest = &trimmed_line[level..];
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }

    let mut text = rest.trim().trim_end_matches('#').trim();
    if let Some((without_block, _)) = split_trailing_block_id(text) {
        text = without_block.trim();
    }

    if text.is_empty() {
        None
    } else {
        Some(HeadingItem {
            level: level as u8,
            text: text.to_string(),
            line: line_number,
        })
    }
}

/// Detecta si la línea termina con un ancla de bloque `^id-bloque` precedida por un espacio.
fn extract_block_ref(trimmed_line: &str, line_number: usize) -> Option<BlockRef> {
    let (without_id, block_id) = split_trailing_block_id(trimmed_line)?;
    let clean_text = without_id.trim();
    Some(BlockRef {
        id: block_id.to_string(),
        line: line_number,
        text: clean_text.to_string(),
    })
}

/// Separa un sufijo ` ^id` al final de una línea cuando `id` es un identificador de bloque válido.
fn split_trailing_block_id(trimmed_line: &str) -> Option<(&str, &str)> {
    let caret_pos = trimmed_line.rfind('^')?;
    // Debe estar precedido por espacio en blanco (o ser una línea dedicada si hubiera texto previo).
    if caret_pos == 0 {
        return None;
    }
    let before = &trimmed_line[..caret_pos];
    if !before.ends_with(char::is_whitespace) {
        return None;
    }

    let id_candidate = &trimmed_line[caret_pos + 1..];
    if id_candidate.is_empty()
        || !id_candidate
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }

    // No tratar `[[Nota#^id]]` al final de línea como definición de bloque.
    if before.trim_end().ends_with('#') {
        return None;
    }

    Some((before.trim_end(), id_candidate))
}

/// Extrae una tarea Markdown (`- [ ] texto`, `- [x] texto`, `* [X] texto`, etc.).
fn extract_task_from_line(
    raw_line: &str,
    masked_line: &str,
    line_number: usize,
) -> Option<TaskItem> {
    let mut s = raw_line.trim_start();
    // Permitir tareas dentro de citas (`> - [ ] ...`)
    while let Some(after_quote) = s.strip_prefix('>') {
        s = after_quote.trim_start();
    }

    let after_bullet = s
        .strip_prefix("- ")
        .or_else(|| s.strip_prefix("* "))
        .or_else(|| s.strip_prefix("+ "))?
        .trim_start();

    let (completed, rest) = if let Some(r) = after_bullet.strip_prefix("[ ]") {
        (false, r)
    } else if let Some(r) = after_bullet
        .strip_prefix("[x]")
        .or_else(|| after_bullet.strip_prefix("[X]"))
    {
        (true, r)
    } else {
        return None;
    };

    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }

    let mut task_tags = BTreeSet::new();
    extract_tags_from_line(masked_line, &mut task_tags);

    let clean_text = match split_trailing_block_id(rest.trim()) {
        Some((without_block, _)) => without_block.trim().to_string(),
        None => rest.trim().to_string(),
    };

    Some(TaskItem {
        text: clean_text,
        completed,
        line: line_number,
        tags: task_tags.into_iter().collect(),
    })
}

/// Extrae campos inline de Dataview (`[clave:: valor]`, `(clave:: valor)` y `Clave:: Valor` de línea).
fn extract_inline_fields_from_line(
    raw_line: &str,
    line_number: usize,
    out: &mut Vec<InlineField>,
) {
    let mut found_delimited = false;
    let chars: Vec<(usize, char)> = raw_line.char_indices().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_code = false;

    while i < len {
        let (byte_pos, ch) = chars[i];
        if ch == '`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if in_code {
            i += 1;
            continue;
        }

        // Saltar wikilinks independientes `[[...]]` al nivel raíz de la línea.
        if ch == '[' && i + 1 < len && chars[i + 1].1 == '[' {
            i += 2;
            while i + 1 < len && !(chars[i].1 == ']' && chars[i + 1].1 == ']') {
                i += 1;
            }
            i += 2;
            continue;
        }

        if ch == '[' || ch == '(' {
            let close_ch = if ch == '[' { ']' } else { ')' };
            let syntax = if ch == '[' {
                InlineFieldSyntax::Bracket
            } else {
                InlineFieldSyntax::Paren
            };

            let mut j = i + 1;
            let mut inner_code = false;
            let mut wikilink_depth = 0usize;
            let mut matched_close_idx: Option<usize> = None;

            while j < len {
                let (_, c) = chars[j];
                if c == '`' {
                    inner_code = !inner_code;
                    j += 1;
                    continue;
                }
                if !inner_code {
                    if c == '[' && j + 1 < len && chars[j + 1].1 == '[' {
                        wikilink_depth += 1;
                        j += 2;
                        continue;
                    }
                    if c == ']' && j + 1 < len && chars[j + 1].1 == ']' && wikilink_depth > 0 {
                        wikilink_depth -= 1;
                        j += 2;
                        continue;
                    }
                    if wikilink_depth == 0 {
                        if c == close_ch {
                            matched_close_idx = Some(j);
                            break;
                        }
                        if c == ch {
                            // Un nuevo `[` o `(` anidado antes de cerrar invalida el contenedor externo.
                            break;
                        }
                    }
                }
                j += 1;
            }

            if let Some(close_idx) = matched_close_idx {
                let inner_start = byte_pos + ch.len_utf8();
                let inner_end = chars[close_idx].0;
                let inner = &raw_line[inner_start..inner_end];

                if let Some((raw_k, raw_v)) = split_inline_field_pair(inner) {
                    if let (Some(key), Some(key_norm)) =
                        (clean_field_key(raw_k), normalize_field_key(raw_k))
                    {
                        out.push(InlineField {
                            key,
                            key_norm,
                            value: raw_v.trim().to_string(),
                            syntax,
                            line: line_number,
                        });
                        found_delimited = true;
                        i = close_idx + 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }

    if found_delimited {
        return;
    }

    // Si no hubo campos `[k:: v]` ni `(k:: v)`, verificar si la línea es un campo `Bare` (`Clave:: Valor`).
    let mut candidate = raw_line.trim();
    while let Some(after_quote) = candidate.strip_prefix('>') {
        candidate = after_quote.trim_start();
    }
    if let Some(after_bullet) = candidate
        .strip_prefix("- ")
        .or_else(|| candidate.strip_prefix("* "))
        .or_else(|| candidate.strip_prefix("+ "))
    {
        candidate = after_bullet.trim_start();
        if let Some(after_box) = candidate
            .strip_prefix("[ ]")
            .or_else(|| candidate.strip_prefix("[x]"))
            .or_else(|| candidate.strip_prefix("[X]"))
        {
            candidate = after_box.trim_start();
        }
    }

    if let Some((raw_k, raw_v)) = split_inline_field_pair(candidate) {
        // En campos Bare, la clave no debe contener corchetes ni paréntesis previos.
        if !raw_k.contains(['[', ']', '(', ')']) {
            if let (Some(key), Some(key_norm)) =
                (clean_field_key(raw_k), normalize_field_key(raw_k))
            {
                let clean_val = match split_trailing_block_id(raw_v.trim()) {
                    Some((without_block, _)) => without_block.trim().to_string(),
                    None => raw_v.trim().to_string(),
                };
                out.push(InlineField {
                    key,
                    key_norm,
                    value: clean_val,
                    syntax: InlineFieldSyntax::Bare,
                    line: line_number,
                });
            }
        }
    }
}

/// Divide `clave:: valor` buscando el primer `::` fuera de código inline `` `...` `` y `[[...]]`.
fn split_inline_field_pair(segment: &str) -> Option<(&str, &str)> {
    let bytes = segment.as_bytes();
    let mut i = 0;
    let mut in_code = false;
    let mut wikilink_depth = 0usize;

    while i + 1 < bytes.len() {
        if bytes[i] == b'`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if !in_code {
            if bytes[i] == b'[' && bytes[i + 1] == b'[' {
                wikilink_depth += 1;
                i += 2;
                continue;
            }
            if bytes[i] == b']' && bytes[i + 1] == b']' && wikilink_depth > 0 {
                wikilink_depth -= 1;
                i += 2;
                continue;
            }
            if wikilink_depth == 0 && bytes[i] == b':' && bytes[i + 1] == b':' {
                // Evitar confundir `:::` con `::`.
                if i + 2 < bytes.len() && bytes[i + 2] == b':' {
                    return None;
                }
                let left = &segment[..i];
                let right = &segment[i + 2..];
                return Some((left, right));
            }
        }
        i += 1;
    }
    None
}

/// Limpia el nombre de una clave de campo inline eliminando formato Markdown (`**`, `_`, `~`)
/// y validando que sea un identificador humano legítimo.
fn clean_field_key(raw_key: &str) -> Option<String> {
    let trimmed = raw_key
        .trim()
        .trim_matches(|c: char| matches!(c, '*' | '_' | '~' | '`'))
        .trim();

    if trimmed.is_empty() || trimmed.len() > 80 {
        return None;
    }

    let first = trimmed.chars().next()?;
    if !first.is_alphanumeric() {
        return None;
    }

    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c.is_whitespace() || matches!(c, '_' | '-' | '/' | '.'))
    {
        return None;
    }

    Some(trimmed.to_string())
}

/// Reemplaza el contenido dentro de comillas invertidas en línea (`` `...` ``) por espacios
/// para evitar extraer `[[wikilinks]]` o `#etiquetas` dentro de fragmentos de código inline.
fn mask_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_code = false;

    while let Some(ch) = chars.next() {
        if ch == '`' {
            in_code = !in_code;
            out.push(' ');
        } else if in_code {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }

    out
}

/// Extrae todos los enlaces `[[destino]]`, `[[destino|alias]]`, `[[destino#sección|alias]]`,
/// `[[destino#^bloque]]` y `![[...]]` de una línea.
fn extract_wikilinks_from_line(
    masked_line: &str,
    line_number: usize,
    context: &str,
    out: &mut Vec<WikiLink>,
) {
    let bytes = masked_line.as_bytes();
    let mut i = 0;

    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let is_embed = i > 0 && bytes[i - 1] == b'!';
            let content_start = i + 2;

            // Buscar cierre `]]` en la misma línea.
            if let Some(rel_end) = masked_line[content_start..].find("]]") {
                let inner = masked_line[content_start..content_start + rel_end].trim();
                if !inner.is_empty() && !inner.contains('\n') {
                    if let Some(link) = parse_wikilink_inner(inner, line_number, context, is_embed)
                    {
                        out.push(link);
                    }
                }
                i = content_start + rel_end + 2;
                continue;
            } else {
                break;
            }
        }
        i += 1;
    }
}

/// Analiza el interior de `[[...]]` separando destino, `#sección` / `#^bloque` opcional y `|alias` opcional.
fn parse_wikilink_inner(
    inner: &str,
    line_number: usize,
    context: &str,
    is_embed: bool,
) -> Option<WikiLink> {
    let (target_and_heading, alias) = match inner.split_once('|') {
        Some((left, right)) => {
            let alias_str = right.trim();
            let alias_opt = if alias_str.is_empty() {
                None
            } else {
                Some(alias_str.to_string())
            };
            (left.trim(), alias_opt)
        }
        None => (inner.trim(), None),
    };

    let (target_raw, heading, block_id) = match target_and_heading.split_once('#') {
        Some((left, right)) => {
            let h = right.trim();
            let h_opt = if h.is_empty() {
                None
            } else {
                Some(h.to_string())
            };
            let b_opt = h
                .strip_prefix('^')
                .map(|b| b.trim().to_string())
                .filter(|b| !b.is_empty());
            (left.trim(), h_opt, b_opt)
        }
        None => (target_and_heading, None, None),
    };

    if target_raw.is_empty() {
        return None;
    }

    Some(WikiLink {
        target: target_raw.to_string(),
        heading,
        block_id,
        alias,
        line: line_number,
        context: context.to_string(),
        is_embed,
    })
}

/// Extrae etiquetas `#tag` y `#padre/hijo` de una línea de texto (ya sin código inline).
fn extract_tags_from_line(masked_line: &str, out: &mut BTreeSet<String>) {
    let chars: Vec<(usize, char)> = masked_line.char_indices().collect();
    let len = chars.len();
    let mut idx = 0;

    while idx < len {
        let (byte_pos, ch) = chars[idx];
        if ch == '#' {
            // Verificar carácter previo: debe ser inicio de línea o un separador permitido.
            let prev_char = if idx == 0 {
                None
            } else {
                Some(chars[idx - 1].1)
            };

            let valid_boundary = match prev_char {
                None => true,
                Some(p) => p.is_whitespace() || matches!(p, '(' | '[' | '{' | ',' | ';' | '"' | '\''),
            };

            // Asegurar que no sea `##` ni `# Encabezado`.
            let next_char = if idx + 1 < len {
                Some(chars[idx + 1].1)
            } else {
                None
            };

            if valid_boundary && next_char.is_some_and(is_tag_char) {
                let start_byte = byte_pos + ch.len_utf8();
                let mut end_idx = idx + 1;
                while end_idx < len && is_tag_char(chars[end_idx].1) {
                    end_idx += 1;
                }

                let end_byte = if end_idx < len {
                    chars[end_idx].0
                } else {
                    masked_line.len()
                };

                let candidate = masked_line[start_byte..end_byte].trim_end_matches('/');
                if is_valid_tag_body(candidate) && !is_likely_css_hex_color(masked_line, byte_pos, candidate) {
                    out.insert(candidate.to_lowercase());
                }

                idx = end_idx;
                continue;
            }
        }
        idx += 1;
    }
}

/// Determina si un carácter puede formar parte de una `#etiqueta`.
fn is_tag_char(c: char) -> bool {
    c.is_alphabetic() || c.is_ascii_digit() || matches!(c, '_' | '-' | '/')
}

/// Valida que el cuerpo de una etiqueta (sin `#`) tenga al menos una letra, no tenga barras
/// vacías (`//`) y todos sus caracteres sean válidos.
fn is_valid_tag_body(candidate: &str) -> bool {
    if candidate.is_empty()
        || candidate.starts_with('/')
        || candidate.ends_with('/')
        || candidate.contains("//")
    {
        return false;
    }

    if !candidate.chars().all(is_tag_char) {
        return false;
    }

    // Debe incluir al menos un carácter alfabético (descarta números puros como `#123`).
    candidate.chars().any(|c| c.is_alphabetic())
}

/// Descarta códigos de color hexadecimales (`#fff`, `#1a2b3c`) cuando están precedidos de `:`
/// (por ejemplo `color: #ff00aa`).
fn is_likely_css_hex_color(line: &str, hash_byte_pos: usize, candidate: &str) -> bool {
    let is_hex_len = matches!(candidate.len(), 3 | 6 | 8);
    if !is_hex_len || !candidate.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }

    let before = line[..hash_byte_pos].trim_end();
    before.ends_with(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn parses_yaml_frontmatter_and_merges_tags() {
        let md = "\
---
title: \"Análisis Macroeconómico Argentino\"
author: Roma
created: 2026-11-11
tags:
  - economía/macro
  - #Inflación
  - series_de_tiempo
---

# Encabezado Secundario

Nota sobre #economía/monetaria y #Política-Fiscal (ver #inflación).
";

        let parsed = parse_note(md);
        assert_eq!(
            parsed.title.as_deref(),
            Some("Análisis Macroeconómico Argentino")
        );
        assert_eq!(
            parsed.frontmatter.get("author").and_then(|v| v.as_str()),
            Some("Roma")
        );
        assert_eq!(
            parsed.frontmatter.get("tags").map(|v| v.as_list().len()),
            Some(3)
        );

        assert!(parsed.has_tag("economía/macro"));
        assert!(parsed.has_tag("#ECONOMÍA/MACRO"));
        assert!(parsed.has_tag("inflación"));
        assert!(parsed.has_tag("series_de_tiempo"));
        assert!(parsed.has_tag("economía/monetaria"));
        assert!(parsed.has_tag("política-fiscal"));
        assert_eq!(parsed.tags.len(), 5);
    }

    #[test]
    fn extracts_wikilinks_with_aliases_headings_and_line_numbers() {
        let md = "\
---
title: Prueba de Enlaces
---
Línea 4 con [[Análisis Macroeconómico]] simple.
Línea 5 con [[Arquitectura de RomSidian|Diseño del Núcleo Rust]] y [[Subcarpeta/Notas#Sección 2|Alias Sección]].
Línea 6 con incrustación ![[diagrama.png]].
";

        let parsed = parse_note(md);
        assert_eq!(parsed.wikilinks.len(), 4);

        assert_eq!(parsed.wikilinks[0].target, "Análisis Macroeconómico");
        assert_eq!(parsed.wikilinks[0].alias, None);
        assert_eq!(parsed.wikilinks[0].heading, None);
        assert_eq!(parsed.wikilinks[0].line, 4);
        assert!(!parsed.wikilinks[0].is_embed);
        assert!(parsed.wikilinks[0].context.contains("[[Análisis Macroeconómico]]"));

        assert_eq!(parsed.wikilinks[1].target, "Arquitectura de RomSidian");
        assert_eq!(
            parsed.wikilinks[1].alias.as_deref(),
            Some("Diseño del Núcleo Rust")
        );
        assert_eq!(parsed.wikilinks[1].line, 5);

        assert_eq!(parsed.wikilinks[2].target, "Subcarpeta/Notas");
        assert_eq!(parsed.wikilinks[2].heading.as_deref(), Some("Sección 2"));
        assert_eq!(parsed.wikilinks[2].alias.as_deref(), Some("Alias Sección"));
        assert_eq!(parsed.wikilinks[2].line, 5);

        assert_eq!(parsed.wikilinks[3].target, "diagrama.png");
        assert!(parsed.wikilinks[3].is_embed);
        assert_eq!(parsed.wikilinks[3].line, 6);
    }

    #[test]
    fn ignores_headings_code_blocks_urls_and_pure_numbers() {
        let md = "\
# Título Principal Inferido
## Subtítulo con #tag_en_encabezado

Texto con `[[EnlaceEnCodigoInline]]` y `#tag_inline_falso`, pero con [[Enlace Valido]] y #tag_real.
URL con fragmento: https://example.com/docs#seccion-no-es-tag y número #12345 o CSS color: #ff00aa.

```rust
// Dentro de bloque de código cercado:
let link = \"[[EnlaceEnBloqueCodigo]]\";
let tag = \"#tag_en_bloque_codigo\";
```
";

        let parsed = parse_note(md);
        assert_eq!(parsed.title.as_deref(), Some("Título Principal Inferido"));

        // Solo debe extraer [[Enlace Valido]]
        assert_eq!(parsed.wikilinks.len(), 1);
        assert_eq!(parsed.wikilinks[0].target, "Enlace Valido");

        // Solo debe extraer #tag_en_encabezado y #tag_real
        assert_eq!(parsed.tags, vec!["tag_en_encabezado", "tag_real"]);
    }

    #[test]
    fn parses_all_real_notes_in_test_vault() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let test_vault_dir = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .expect("Raíz del workspace")
            .join("test-vault");

        let real_vault = crate::vault::Vault::open(&test_vault_dir).expect("Debe abrir test-vault/");
        for summary in real_vault.list_notes().expect("Debe listar notas reales") {
            let doc = real_vault
                .read_note(&summary.relative_path)
                .expect("Debe leer cada nota real");
            let _ = parse_note(&doc.content);
        }

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Reloj válido")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "romsidian_parser_seed_{}_{}",
            std::process::id(),
            unique
        ));
        crate::tests::copy_non_hidden_dir(&test_vault_dir, &temp_dir);
        crate::tests::ensure_canonical_seed_notes(&temp_dir);

        let index_note = fs::read_to_string(temp_dir.join("Índice General.md"))
            .expect("Debe leer Índice General.md");
        let parsed = parse_note(&index_note);

        assert_eq!(
            parsed.title.as_deref(),
            Some("Índice General de la Bóveda")
        );
        assert!(parsed.has_tag("indice"));
        assert!(parsed.has_tag("organizacion/boveda"));
        assert!(parsed.has_tag("inicio"));

        let targets: Vec<&str> = parsed.wikilinks.iter().map(|w| w.target.as_str()).collect();
        assert!(targets.contains(&"Análisis Macroeconómico"));
        assert!(targets.contains(&"Arquitectura de RomSidian"));
        assert!(targets.contains(&"Notas de Investigación"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn extracts_inline_fields_tasks_headings_blocks_and_sections() {
        let md = "\
---
title: Nota de Prueba v0.2
---
# Introducción General

Estado del Proyecto:: En progreso
- **Calificación Global**:: 9.5

Texto con [prioridad:: alta] y campo oculto (responsable:: [[Ana Gómez]]) en la misma línea.
Este párrafo define un bloque clave para citar en otras notas ^bloque-clave

## Tareas Pendientes
- [ ] Implementar motor Dataview #desarrollo/core
- [x] Diseñar parser de bloques ^tarea-lista

## Conclusión
Cierre del documento con transclusión ![[Otra Nota#^cita-1|Ver cita]].
";

        let parsed = parse_note(md);

        // 1. Campos inline (Bare, Bracket y Paren)
        assert_eq!(parsed.inline_fields.len(), 4);
        assert_eq!(parsed.inline_fields[0].key, "Estado del Proyecto");
        assert_eq!(parsed.inline_fields[0].key_norm, "estado-del-proyecto");
        assert_eq!(parsed.inline_fields[0].value, "En progreso");
        assert_eq!(parsed.inline_fields[0].syntax, InlineFieldSyntax::Bare);

        assert_eq!(parsed.inline_fields[1].key, "Calificación Global");
        assert_eq!(parsed.inline_fields[1].key_norm, "calificación-global");
        assert_eq!(parsed.inline_fields[1].value, "9.5");
        assert_eq!(parsed.inline_fields[1].syntax, InlineFieldSyntax::Bare);

        assert_eq!(parsed.inline_fields[2].key_norm, "prioridad");
        assert_eq!(parsed.inline_fields[2].value, "alta");
        assert_eq!(parsed.inline_fields[2].syntax, InlineFieldSyntax::Bracket);

        assert_eq!(parsed.inline_fields[3].key_norm, "responsable");
        assert_eq!(parsed.inline_fields[3].value, "[[Ana Gómez]]");
        assert_eq!(parsed.inline_fields[3].syntax, InlineFieldSyntax::Paren);

        // 2. Tareas Markdown
        assert_eq!(parsed.tasks.len(), 2);
        assert!(!parsed.tasks[0].completed);
        assert_eq!(
            parsed.tasks[0].text,
            "Implementar motor Dataview #desarrollo/core"
        );
        assert_eq!(parsed.tasks[0].tags, vec!["desarrollo/core"]);

        assert!(parsed.tasks[1].completed);
        assert_eq!(parsed.tasks[1].text, "Diseñar parser de bloques");

        // 3. Bloques (^id) y extracción
        assert_eq!(parsed.blocks.len(), 2);
        assert_eq!(parsed.blocks[0].id, "bloque-clave");
        assert_eq!(
            parsed.extract_block("^bloque-clave").as_deref(),
            Some("Este párrafo define un bloque clave para citar en otras notas")
        );

        // 4. Encabezados y extracción de sección
        assert_eq!(parsed.headings.len(), 3);
        let seccion_tareas = parsed
            .extract_section("Tareas Pendientes")
            .expect("Extraer sección Tareas Pendientes");
        assert!(seccion_tareas.contains("Implementar motor Dataview"));
        assert!(!seccion_tareas.contains("Cierre del documento"));

        // 5. Enlace a bloque con transclusión
        let embed_link = parsed
            .wikilinks
            .iter()
            .find(|w| w.target == "Otra Nota")
            .expect("Debe extraer ![[Otra Nota#^cita-1|Ver cita]]");
        assert!(embed_link.is_embed);
        assert_eq!(embed_link.heading.as_deref(), Some("^cita-1"));
        assert_eq!(embed_link.block_id.as_deref(), Some("cita-1"));
        assert_eq!(embed_link.alias.as_deref(), Some("Ver cita"));
    }
}
