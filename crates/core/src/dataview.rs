//! Motor declarativo `Dataview` (DQL sandboxed) y evaluador de expresiones inline `` `= this.*` ``.
//!
//! Opera exclusivamente sobre el índice SQLite (`VaultIndex`) y la bóveda en disco, sin
//! ejecutar JavaScript arbitrario ni acceder fuera del repositorio. Soporta:
//! - Consultas `TABLE [WITHOUT ID]`, `LIST [WITHOUT ID]` y `TASK`
//! - Cláusulas `FROM` (`#etiqueta`, `"carpeta"`, `[[Nota]]`, `outgoing([[Nota]])`, `AND`/`OR`/`!`)
//! - Cláusulas `WHERE` (`=`, `!=`, `<`, `<=`, `>`, `>=`, `contains`, `startswith`, `endswith`, `AND`/`OR`/`!`)
//! - Cláusulas `SORT <campo> [ASC|DESC]` y `LIMIT <n>`
//! - Expresiones inline `` `= this.file.*` `` y `` `= this.<campo>` ``

use crate::index::DataviewPageRecord;
use crate::parser::{normalize_field_key, normalize_tag};
use crate::vault::format_unix_secs_utc;
use crate::{CoreError, IndexedTask, Vault, VaultIndex};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;

/// Error al analizar o ejecutar una consulta DQL o expresión inline de `Dataview`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataviewError(pub String);

impl fmt::Display for DataviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error Dataview: {}", self.0)
    }
}

impl std::error::Error for DataviewError {}

impl From<CoreError> for DataviewError {
    fn from(value: CoreError) -> Self {
        Self(value.to_string())
    }
}

/// Valor tipado resultante de evaluar una expresión DQL o inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataviewValue {
    Null,
    Bool { value: bool },
    Number { value: f64 },
    Text { value: String },
    Link { path: String, label: String },
    List { items: Vec<DataviewValue> },
}

impl DataviewValue {
    fn is_truthy(&self) -> bool {
        match self {
            DataviewValue::Null => false,
            DataviewValue::Bool { value } => *value,
            DataviewValue::Number { value } => *value != 0.0 && !value.is_nan(),
            DataviewValue::Text { value } => !value.trim().is_empty(),
            DataviewValue::Link { path, label } => !path.is_empty() || !label.is_empty(),
            DataviewValue::List { items } => !items.is_empty(),
        }
    }

    fn as_display_string(&self) -> String {
        match self {
            DataviewValue::Null => String::new(),
            DataviewValue::Bool { value } => value.to_string(),
            DataviewValue::Number { value } => {
                if (value.fract()).abs() < f64::EPSILON {
                    format!("{}", *value as i64)
                } else {
                    value.to_string()
                }
            }
            DataviewValue::Text { value } => value.clone(),
            DataviewValue::Link { label, .. } => label.clone(),
            DataviewValue::List { items } => items
                .iter()
                .map(DataviewValue::as_display_string)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    fn as_number(&self) -> Option<f64> {
        match self {
            DataviewValue::Number { value } => Some(*value),
            DataviewValue::Bool { value } => Some(if *value { 1.0 } else { 0.0 }),
            DataviewValue::Text { value } => value.trim().parse::<f64>().ok(),
            _ => None,
        }
    }
}

/// Columna declarada en una consulta `TABLE`.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub expr: Expr,
    pub header: String,
}

/// Sentido de ordenación en `SORT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

/// Cláusula individual dentro de `SORT`.
#[derive(Debug, Clone, PartialEq)]
pub struct SortClause {
    pub expr: Expr,
    pub order: SortOrder,
}

/// Variante principal de una consulta DQL.
#[derive(Debug, Clone, PartialEq)]
pub enum DataviewQueryKind {
    Table {
        columns: Vec<TableColumn>,
        without_id: bool,
    },
    List {
        expr: Option<Expr>,
        without_id: bool,
    },
    Task,
}

/// Árbol sintáctico de una consulta DQL parseada.
#[derive(Debug, Clone, PartialEq)]
pub struct DataviewQuery {
    pub kind: DataviewQueryKind,
    pub from: Option<SourceExpr>,
    pub where_clause: Option<Expr>,
    pub sort: Vec<SortClause>,
    pub limit: Option<usize>,
}

/// Expresión booleana de origen en `FROM`.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceExpr {
    Tag(String),
    Folder(String),
    IncomingLink(String),
    OutgoingLink(String),
    Not(Box<SourceExpr>),
    And(Box<SourceExpr>, Box<SourceExpr>),
    Or(Box<SourceExpr>, Box<SourceExpr>),
}

/// Operadores binarios soportados en expresiones DQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Expresión evaluable en `TABLE`, `LIST`, `WHERE`, `SORT` e inline `` `= ...` ``.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Tag(String),
    Link(String),
    Field(String),
    Not(Box<Expr>),
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

/// Elemento individual devuelto por una consulta `LIST`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataviewListItem {
    pub path: String,
    pub title: String,
    pub value: Option<DataviewValue>,
}

/// Tarea individual devuelta por una consulta `TASK`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataviewTaskItem {
    pub path: String,
    pub note_title: String,
    pub line: usize,
    pub completed: bool,
    pub text: String,
    pub tags: Vec<String>,
}

/// Resultado serializable de ejecutar una consulta DQL en `ROMSIDIAN`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "lowercase")]
pub enum DataviewQueryResult {
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<DataviewValue>>,
        total_count: usize,
    },
    List {
        items: Vec<DataviewListItem>,
        total_count: usize,
    },
    Task {
        items: Vec<DataviewTaskItem>,
        total_count: usize,
    },
}

/// Resultado individual de evaluar una expresión inline `` `= ...` `` en una nota.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataviewInlineEvaluation {
    pub expression: String,
    pub value: DataviewValue,
}

// ============================================================================
// Tokenizador léxico para DQL y expresiones inline
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Ident(String),
    StringLit(String),
    NumberLit(f64),
    TagLit(String),
    LinkLit(String),
    Comma,
    LParen,
    RParen,
    Bang,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    pos: usize,
}

fn tokenize_dql(input: &str) -> Result<Vec<Token>, DataviewError> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Comentarios de línea `// ...`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Wikilink literal `[[...]]`
        if b == b'[' && i + 1 < len && bytes[i + 1] == b'[' {
            let start = i;
            i += 2;
            let content_start = i;
            while i + 1 < len && !(bytes[i] == b']' && bytes[i + 1] == b']') {
                i += 1;
            }
            if i + 1 >= len {
                return Err(DataviewError(format!(
                    "Wikilink sin cerrar en posición {start}"
                )));
            }
            let inner = input[content_start..i].trim().to_string();
            i += 2;
            tokens.push(Token {
                kind: TokenKind::LinkLit(inner),
                pos: start,
            });
            continue;
        }

        // Cadenas `"..."` o `'...'`
        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
            i += 1;
            let mut value = String::new();
            let mut closed = false;
            while i < len {
                let ch = input[i..].chars().next().unwrap();
                if ch as u32 == quote as u32 {
                    i += ch.len_utf8();
                    closed = true;
                    break;
                }
                if ch == '\\' && i + 1 < len {
                    i += 1;
                    let esc = input[i..].chars().next().unwrap();
                    match esc {
                        'n' => value.push('\n'),
                        't' => value.push('\t'),
                        other => value.push(other),
                    }
                    i += esc.len_utf8();
                } else {
                    value.push(ch);
                    i += ch.len_utf8();
                }
            }
            if !closed {
                return Err(DataviewError(format!(
                    "Cadena sin cerrar en posición {start}"
                )));
            }
            tokens.push(Token {
                kind: TokenKind::StringLit(value),
                pos: start,
            });
            continue;
        }

        // Etiquetas `#tag/subtag`
        if b == b'#' {
            let start = i;
            i += 1;
            let tag_start = i;
            while i < len {
                let ch = input[i..].chars().next().unwrap();
                if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '/' {
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
            let raw_tag = &input[tag_start..i];
            if raw_tag.is_empty() {
                return Err(DataviewError(format!(
                    "Etiqueta '#' vacía en posición {start}"
                )));
            }
            tokens.push(Token {
                kind: TokenKind::TagLit(raw_tag.to_string()),
                pos: start,
            });
            continue;
        }

        // Signos de puntuación y operadores
        match b {
            b',' => {
                tokens.push(Token {
                    kind: TokenKind::Comma,
                    pos: i,
                });
                i += 1;
                continue;
            }
            b'(' => {
                tokens.push(Token {
                    kind: TokenKind::LParen,
                    pos: i,
                });
                i += 1;
                continue;
            }
            b')' => {
                tokens.push(Token {
                    kind: TokenKind::RParen,
                    pos: i,
                });
                i += 1;
                continue;
            }
            b'!' => {
                if i + 1 < len && bytes[i + 1] == b'=' {
                    tokens.push(Token {
                        kind: TokenKind::Ne,
                        pos: i,
                    });
                    i += 2;
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Bang,
                        pos: i,
                    });
                    i += 1;
                }
                continue;
            }
            b'=' => {
                let pos = i;
                if i + 1 < len && bytes[i + 1] == b'=' {
                    i += 2;
                } else {
                    i += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Eq,
                    pos,
                });
                continue;
            }
            b'<' => {
                if i + 1 < len && bytes[i + 1] == b'=' {
                    tokens.push(Token {
                        kind: TokenKind::Le,
                        pos: i,
                    });
                    i += 2;
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Lt,
                        pos: i,
                    });
                    i += 1;
                }
                continue;
            }
            b'>' => {
                if i + 1 < len && bytes[i + 1] == b'=' {
                    tokens.push(Token {
                        kind: TokenKind::Ge,
                        pos: i,
                    });
                    i += 2;
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Gt,
                        pos: i,
                    });
                    i += 1;
                }
                continue;
            }
            _ => {}
        }

        // Números (positivos o negativos simples si empiezan por dígito)
        if b.is_ascii_digit()
            || (b == b'-' && i + 1 < len && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            if b == b'-' {
                i += 1;
            }
            let mut seen_dot = false;
            while i < len {
                let nb = bytes[i];
                if nb.is_ascii_digit() {
                    i += 1;
                } else if nb == b'.' && !seen_dot && i + 1 < len && bytes[i + 1].is_ascii_digit() {
                    seen_dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            let raw_num = &input[start..i];
            let parsed = raw_num.parse::<f64>().map_err(|_| {
                DataviewError(format!("Número inválido '{raw_num}' en posición {start}"))
            })?;
            tokens.push(Token {
                kind: TokenKind::NumberLit(parsed),
                pos: start,
            });
            continue;
        }

        // `-` como negación de fuente en `FROM -tag`
        if b == b'-' {
            tokens.push(Token {
                kind: TokenKind::Bang,
                pos: i,
            });
            i += 1;
            continue;
        }

        // Identificadores y rutas con punto/guión (`file.name`, `this.estado`, `fecha-limite`)
        let ch = input[i..].chars().next().unwrap();
        if is_ident_start(ch) {
            let start = i;
            while i < len {
                let cur = input[i..].chars().next().unwrap();
                if is_ident_continue(cur) {
                    i += cur.len_utf8();
                } else {
                    break;
                }
            }
            tokens.push(Token {
                kind: TokenKind::Ident(input[start..i].to_string()),
                pos: start,
            });
            continue;
        }

        return Err(DataviewError(format!(
            "Carácter inesperado '{ch}' en posición {i}"
        )));
    }

    Ok(tokens)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

// ============================================================================
// Parser sintáctico de consultas DQL
// ============================================================================

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, cursor: 0 }
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.cursor).map(|t| &t.kind)
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(TokenKind::Ident(s)) if s.eq_ignore_ascii_case(kw))
    }

    fn peek_any_clause_keyword(&self) -> bool {
        self.peek_keyword("FROM")
            || self.peek_keyword("WHERE")
            || self.peek_keyword("SORT")
            || self.peek_keyword("LIMIT")
    }

    fn next(&mut self) -> Option<Token> {
        if self.cursor < self.tokens.len() {
            let tok = self.tokens[self.cursor].clone();
            self.cursor += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn consume_keyword(&mut self, kw: &str) -> bool {
        if self.peek_keyword(kw) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn parse_query(&mut self) -> Result<DataviewQuery, DataviewError> {
        let head = self
            .next()
            .ok_or_else(|| DataviewError("La consulta DQL está vacía".to_string()))?;

        let kind = match head.kind {
            TokenKind::Ident(kw) if kw.eq_ignore_ascii_case("TABLE") => {
                let without_id = if self.consume_keyword("WITHOUT") {
                    if !self.consume_keyword("ID") {
                        return Err(DataviewError(
                            "Se esperaba 'ID' después de 'WITHOUT'".to_string(),
                        ));
                    }
                    true
                } else {
                    false
                };

                let mut columns = Vec::new();
                while self.peek().is_some() && !self.peek_any_clause_keyword() {
                    let expr = self.parse_expr()?;
                    let header = if self.consume_keyword("AS") {
                        self.parse_alias_name()?
                    } else {
                        default_header_for_expr(&expr)
                    };
                    columns.push(TableColumn { expr, header });
                    if matches!(self.peek(), Some(TokenKind::Comma)) {
                        self.cursor += 1;
                    } else {
                        break;
                    }
                }
                DataviewQueryKind::Table {
                    columns,
                    without_id,
                }
            }
            TokenKind::Ident(kw) if kw.eq_ignore_ascii_case("LIST") => {
                let without_id = if self.consume_keyword("WITHOUT") {
                    if !self.consume_keyword("ID") {
                        return Err(DataviewError(
                            "Se esperaba 'ID' después de 'WITHOUT'".to_string(),
                        ));
                    }
                    true
                } else {
                    false
                };
                let expr = if self.peek().is_some() && !self.peek_any_clause_keyword() {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                DataviewQueryKind::List { expr, without_id }
            }
            TokenKind::Ident(kw) if kw.eq_ignore_ascii_case("TASK") => DataviewQueryKind::Task,
            _ => {
                return Err(DataviewError(
                    "La consulta DQL debe comenzar por TABLE, LIST o TASK".to_string(),
                ))
            }
        };

        let mut from = None;
        let mut where_clause = None;
        let mut sort = Vec::new();
        let mut limit = None;

        while self.peek().is_some() {
            if self.consume_keyword("FROM") {
                if from.is_some() {
                    return Err(DataviewError("Cláusula FROM duplicada".to_string()));
                }
                from = Some(self.parse_source_or()?);
            } else if self.consume_keyword("WHERE") {
                if where_clause.is_some() {
                    return Err(DataviewError("Cláusula WHERE duplicada".to_string()));
                }
                where_clause = Some(self.parse_expr()?);
            } else if self.consume_keyword("SORT") {
                loop {
                    let expr = self.parse_expr()?;
                    let order = if self.consume_keyword("DESC") {
                        SortOrder::Desc
                    } else {
                        let _ = self.consume_keyword("ASC");
                        SortOrder::Asc
                    };
                    sort.push(SortClause { expr, order });
                    if matches!(self.peek(), Some(TokenKind::Comma)) {
                        self.cursor += 1;
                    } else {
                        break;
                    }
                }
            } else if self.consume_keyword("LIMIT") {
                match self.next() {
                    Some(Token {
                        kind: TokenKind::NumberLit(n),
                        ..
                    }) if n >= 0.0 => {
                        limit = Some(n as usize);
                    }
                    _ => {
                        return Err(DataviewError(
                            "Se esperaba un número entero positivo después de LIMIT".to_string(),
                        ))
                    }
                }
            } else {
                let unexpected = self.next().unwrap();
                return Err(DataviewError(format!(
                    "Token inesperado cerca de posición {}: {:?}",
                    unexpected.pos, unexpected.kind
                )));
            }
        }

        Ok(DataviewQuery {
            kind,
            from,
            where_clause,
            sort,
            limit,
        })
    }

    fn parse_alias_name(&mut self) -> Result<String, DataviewError> {
        match self.next() {
            Some(Token {
                kind: TokenKind::StringLit(s),
                ..
            })
            | Some(Token {
                kind: TokenKind::Ident(s),
                ..
            }) => Ok(s),
            _ => Err(DataviewError(
                "Se esperaba un alias después de 'AS'".to_string(),
            )),
        }
    }

    fn parse_source_or(&mut self) -> Result<SourceExpr, DataviewError> {
        let mut left = self.parse_source_and()?;
        while self.consume_keyword("OR") {
            let right = self.parse_source_and()?;
            left = SourceExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_source_and(&mut self) -> Result<SourceExpr, DataviewError> {
        let mut left = self.parse_source_primary()?;
        while self.consume_keyword("AND") {
            let right = self.parse_source_primary()?;
            left = SourceExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_source_primary(&mut self) -> Result<SourceExpr, DataviewError> {
        if matches!(self.peek(), Some(TokenKind::Bang)) || self.peek_keyword("NOT") {
            self.cursor += 1;
            let inner = self.parse_source_primary()?;
            return Ok(SourceExpr::Not(Box::new(inner)));
        }
        if matches!(self.peek(), Some(TokenKind::LParen)) {
            self.cursor += 1;
            let inner = self.parse_source_or()?;
            if !matches!(self.next().map(|t| t.kind), Some(TokenKind::RParen)) {
                return Err(DataviewError("Falta ')' en cláusula FROM".to_string()));
            }
            return Ok(inner);
        }

        match self.next() {
            Some(Token {
                kind: TokenKind::TagLit(raw),
                ..
            }) => Ok(SourceExpr::Tag(normalize_tag(&raw).unwrap_or(raw.to_lowercase()))),
            Some(Token {
                kind: TokenKind::StringLit(folder),
                ..
            }) => Ok(SourceExpr::Folder(folder)),
            Some(Token {
                kind: TokenKind::LinkLit(target),
                ..
            }) => Ok(SourceExpr::IncomingLink(target)),
            Some(Token {
                kind: TokenKind::Ident(fn_name),
                ..
            }) if fn_name.eq_ignore_ascii_case("outgoing")
                && matches!(self.peek(), Some(TokenKind::LParen)) =>
            {
                self.cursor += 1;
                let target = match self.next() {
                    Some(Token {
                        kind: TokenKind::LinkLit(t),
                        ..
                    })
                    | Some(Token {
                        kind: TokenKind::StringLit(t),
                        ..
                    }) => t,
                    _ => {
                        return Err(DataviewError(
                            "Se esperaba [[Nota]] dentro de outgoing(...)".to_string(),
                        ))
                    }
                };
                if !matches!(self.next().map(|t| t.kind), Some(TokenKind::RParen)) {
                    return Err(DataviewError("Falta ')' en outgoing(...)".to_string()));
                }
                Ok(SourceExpr::OutgoingLink(target))
            }
            _ => Err(DataviewError(
                "Fuente FROM no válida (use #etiqueta, \"carpeta\", [[Nota]] o outgoing([[Nota]]))"
                    .to_string(),
            )),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, DataviewError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, DataviewError> {
        let mut left = self.parse_and()?;
        while self.consume_keyword("OR") {
            let right = self.parse_and()?;
            left = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, DataviewError> {
        let mut left = self.parse_comparison()?;
        while self.consume_keyword("AND") {
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, DataviewError> {
        let left = self.parse_unary()?;
        let op = match self.peek() {
            Some(TokenKind::Eq) => Some(BinaryOp::Eq),
            Some(TokenKind::Ne) => Some(BinaryOp::Ne),
            Some(TokenKind::Lt) => Some(BinaryOp::Lt),
            Some(TokenKind::Le) => Some(BinaryOp::Le),
            Some(TokenKind::Gt) => Some(BinaryOp::Gt),
            Some(TokenKind::Ge) => Some(BinaryOp::Ge),
            _ => None,
        };

        if let Some(op) = op {
            self.cursor += 1;
            let right = self.parse_unary()?;
            Ok(Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        } else {
            Ok(left)
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, DataviewError> {
        if matches!(self.peek(), Some(TokenKind::Bang)) || self.peek_keyword("NOT") {
            self.cursor += 1;
            let inner = self.parse_unary()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, DataviewError> {
        if matches!(self.peek(), Some(TokenKind::LParen)) {
            self.cursor += 1;
            let inner = self.parse_expr()?;
            if !matches!(self.next().map(|t| t.kind), Some(TokenKind::RParen)) {
                return Err(DataviewError("Falta paréntesis de cierre ')'".to_string()));
            }
            return Ok(inner);
        }

        let tok = self
            .next()
            .ok_or_else(|| DataviewError("Expresión incompleta".to_string()))?;

        match tok.kind {
            TokenKind::StringLit(s) => Ok(Expr::Text(s)),
            TokenKind::NumberLit(n) => Ok(Expr::Number(n)),
            TokenKind::TagLit(raw) => Ok(Expr::Tag(normalize_tag(&raw).unwrap_or(raw.to_lowercase()))),
            TokenKind::LinkLit(target) => Ok(Expr::Link(target)),
            TokenKind::Ident(ident) => {
                if matches!(self.peek(), Some(TokenKind::LParen)) {
                    self.cursor += 1;
                    let mut args = Vec::new();
                    while !matches!(self.peek(), Some(TokenKind::RParen)) {
                        if self.peek().is_none() {
                            return Err(DataviewError(format!(
                                "Falta ')' en llamada a {ident}(...)"
                            )));
                        }
                        args.push(self.parse_expr()?);
                        if matches!(self.peek(), Some(TokenKind::Comma)) {
                            self.cursor += 1;
                        } else {
                            break;
                        }
                    }
                    if !matches!(self.next().map(|t| t.kind), Some(TokenKind::RParen)) {
                        return Err(DataviewError(format!(
                            "Se esperaba ')' al cerrar {ident}(...)"
                        )));
                    }
                    Ok(Expr::Call {
                        name: ident.to_lowercase(),
                        args,
                    })
                } else if ident.eq_ignore_ascii_case("true") {
                    Ok(Expr::Bool(true))
                } else if ident.eq_ignore_ascii_case("false") {
                    Ok(Expr::Bool(false))
                } else if ident.eq_ignore_ascii_case("null") {
                    Ok(Expr::Null)
                } else {
                    Ok(Expr::Field(ident))
                }
            }
            other => Err(DataviewError(format!(
                "Token inesperado en expresión: {other:?}"
            ))),
        }
    }
}

fn default_header_for_expr(expr: &Expr) -> String {
    match expr {
        Expr::Field(name) => name.clone(),
        Expr::Call { name, .. } => name.clone(),
        Expr::Text(s) | Expr::Tag(s) | Expr::Link(s) => s.clone(),
        _ => "Valor".to_string(),
    }
}

/// Parsea una consulta DQL declarativa en un AST tipado sin ejecutarla.
pub fn parse_dataview_query(raw_dql: &str) -> Result<DataviewQuery, DataviewError> {
    let tokens = tokenize_dql(raw_dql)?;
    let mut parser = Parser::new(tokens);
    parser.parse_query()
}

/// Parsea una expresión inline tipo `` `= this.file.name` `` o `this.estado`.
pub fn parse_dataview_inline_expr(raw_expr: &str) -> Result<Expr, DataviewError> {
    let trimmed = raw_expr.trim();
    let without_ticks = trimmed
        .strip_prefix('`')
        .and_then(|s| s.strip_suffix('`'))
        .unwrap_or(trimmed)
        .trim();
    let expr_str = without_ticks
        .strip_prefix('=')
        .unwrap_or(without_ticks)
        .trim();

    if expr_str.is_empty() {
        return Err(DataviewError("La expresión inline está vacía".to_string()));
    }

    let tokens = tokenize_dql(expr_str)?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;
    if parser.peek().is_some() {
        return Err(DataviewError(format!(
            "Contenido inesperado tras la expresión inline '{expr_str}'"
        )));
    }
    Ok(expr)
}

// ============================================================================
// Contexto de evaluación y ejecución sobre SQLite / bóveda
// ============================================================================

struct EvalContext<'a> {
    page: &'a DataviewPageRecord,
    size_bytes: u64,
    task: Option<&'a IndexedTask>,
}

/// Ejecuta una consulta DQL (`TABLE`, `LIST`, `TASK`) de forma segura sobre `VaultIndex`.
pub fn execute_dataview_query(
    index: &VaultIndex,
    vault: Option<&Vault>,
    raw_dql: &str,
) -> Result<DataviewQueryResult, DataviewError> {
    let query = parse_dataview_query(raw_dql)?;
    let pages = index.list_dataview_pages()?;
    let size_map = collect_file_sizes(vault, &pages);

    let mut matched_pages: Vec<&DataviewPageRecord> = pages
        .iter()
        .filter(|page| match &query.from {
            Some(src) => matches_source(page, src, &pages),
            None => true,
        })
        .collect();

    match query.kind {
        DataviewQueryKind::Table {
            columns,
            without_id,
        } => {
            if let Some(ref where_expr) = query.where_clause {
                matched_pages.retain(|page| {
                    let ctx = EvalContext {
                        page,
                        size_bytes: *size_map.get(&page.path).unwrap_or(&0),
                        task: None,
                    };
                    eval_expr(&ctx, where_expr).is_truthy()
                });
            }

            if !query.sort.is_empty() {
                sort_pages(&mut matched_pages, &query.sort, &size_map);
            }

            let total_count = matched_pages.len();
            if let Some(limit) = query.limit {
                matched_pages.truncate(limit);
            }

            let mut headers = Vec::new();
            if !without_id {
                headers.push("Archivo".to_string());
            }
            for col in &columns {
                headers.push(col.header.clone());
            }

            let mut rows = Vec::with_capacity(matched_pages.len());
            for page in matched_pages {
                let ctx = EvalContext {
                    page,
                    size_bytes: *size_map.get(&page.path).unwrap_or(&0),
                    task: None,
                };
                let mut row = Vec::with_capacity(headers.len());
                if !without_id {
                    row.push(DataviewValue::Link {
                        path: page.path.clone(),
                        label: page.title.clone(),
                    });
                }
                for col in &columns {
                    row.push(eval_expr(&ctx, &col.expr));
                }
                rows.push(row);
            }

            Ok(DataviewQueryResult::Table {
                headers,
                rows,
                total_count,
            })
        }
        DataviewQueryKind::List { expr, .. } => {
            if let Some(ref where_expr) = query.where_clause {
                matched_pages.retain(|page| {
                    let ctx = EvalContext {
                        page,
                        size_bytes: *size_map.get(&page.path).unwrap_or(&0),
                        task: None,
                    };
                    eval_expr(&ctx, where_expr).is_truthy()
                });
            }

            if !query.sort.is_empty() {
                sort_pages(&mut matched_pages, &query.sort, &size_map);
            }

            let total_count = matched_pages.len();
            if let Some(limit) = query.limit {
                matched_pages.truncate(limit);
            }

            let items = matched_pages
                .into_iter()
                .map(|page| {
                    let value = expr.as_ref().map(|e| {
                        let ctx = EvalContext {
                            page,
                            size_bytes: *size_map.get(&page.path).unwrap_or(&0),
                            task: None,
                        };
                        eval_expr(&ctx, e)
                    });
                    DataviewListItem {
                        path: page.path.clone(),
                        title: page.title.clone(),
                        value,
                    }
                })
                .collect();

            Ok(DataviewQueryResult::List { items, total_count })
        }
        DataviewQueryKind::Task => {
            let page_by_path: BTreeMap<&str, &DataviewPageRecord> =
                matched_pages.iter().map(|p| (p.path.as_str(), *p)).collect();

            let all_tasks = index.list_all_tasks()?;
            let mut filtered_tasks: Vec<(&IndexedTask, &DataviewPageRecord)> = all_tasks
                .iter()
                .filter_map(|task| {
                    let page = *page_by_path.get(task.path.as_str())?;
                    if let Some(ref where_expr) = query.where_clause {
                        let ctx = EvalContext {
                            page,
                            size_bytes: *size_map.get(&page.path).unwrap_or(&0),
                            task: Some(task),
                        };
                        if !eval_expr(&ctx, where_expr).is_truthy() {
                            return None;
                        }
                    }
                    Some((task, page))
                })
                .collect();

            if !query.sort.is_empty() {
                filtered_tasks.sort_by(|(ta, pa), (tb, pb)| {
                    let ca = EvalContext {
                        page: pa,
                        size_bytes: *size_map.get(&pa.path).unwrap_or(&0),
                        task: Some(ta),
                    };
                    let cb = EvalContext {
                        page: pb,
                        size_bytes: *size_map.get(&pb.path).unwrap_or(&0),
                        task: Some(tb),
                    };
                    compare_by_sort_clauses(&ca, &cb, &query.sort)
                });
            }

            let total_count = filtered_tasks.len();
            if let Some(limit) = query.limit {
                filtered_tasks.truncate(limit);
            }

            let items = filtered_tasks
                .into_iter()
                .map(|(t, _)| DataviewTaskItem {
                    path: t.path.clone(),
                    note_title: t.note_title.clone(),
                    line: t.line,
                    completed: t.completed,
                    text: t.text.clone(),
                    tags: t.tags.clone(),
                })
                .collect();

            Ok(DataviewQueryResult::Task { items, total_count })
        }
    }
}

/// Evalúa una expresión inline `` `= this.file.*` `` o `` `= this.<campo>` `` en una nota específica.
pub fn eval_dataview_inline(
    index: &VaultIndex,
    vault: Option<&Vault>,
    note_relative_path: &str,
    expression: &str,
) -> Result<DataviewValue, DataviewError> {
    let expr = parse_dataview_inline_expr(expression)?;
    let pages = index.list_dataview_pages()?;
    let target_norm = note_relative_path.replace('\\', "/").to_lowercase();
    let page = pages
        .iter()
        .find(|p| p.path.to_lowercase() == target_norm)
        .ok_or_else(|| DataviewError(format!("Nota no indexada: {note_relative_path}")))?;

    let size_bytes = vault
        .and_then(|v| fs::metadata(v.root_path().join(&page.path)).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let ctx = EvalContext {
        page,
        size_bytes,
        task: None,
    };
    Ok(eval_expr(&ctx, &expr))
}

/// Evalúa un lote de expresiones inline sobre la misma nota en una sola lectura del índice.
pub fn eval_dataview_inline_batch(
    index: &VaultIndex,
    vault: Option<&Vault>,
    note_relative_path: &str,
    expressions: &[String],
) -> Result<Vec<DataviewInlineEvaluation>, DataviewError> {
    let pages = index.list_dataview_pages()?;
    let target_norm = note_relative_path.replace('\\', "/").to_lowercase();
    let page = pages
        .iter()
        .find(|p| p.path.to_lowercase() == target_norm)
        .ok_or_else(|| DataviewError(format!("Nota no indexada: {note_relative_path}")))?;

    let size_bytes = vault
        .and_then(|v| fs::metadata(v.root_path().join(&page.path)).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let ctx = EvalContext {
        page,
        size_bytes,
        task: None,
    };

    expressions
        .iter()
        .map(|raw| {
            let expr = parse_dataview_inline_expr(raw)?;
            Ok(DataviewInlineEvaluation {
                expression: raw.clone(),
                value: eval_expr(&ctx, &expr),
            })
        })
        .collect()
}

fn collect_file_sizes(vault: Option<&Vault>, pages: &[DataviewPageRecord]) -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    let Some(v) = vault else {
        return map;
    };
    for page in pages {
        if let Ok(meta) = fs::metadata(v.root_path().join(&page.path)) {
            map.insert(page.path.clone(), meta.len());
        }
    }
    map
}

fn sort_pages(
    pages: &mut Vec<&DataviewPageRecord>,
    clauses: &[SortClause],
    size_map: &BTreeMap<String, u64>,
) {
    pages.sort_by(|a, b| {
        let ca = EvalContext {
            page: a,
            size_bytes: *size_map.get(&a.path).unwrap_or(&0),
            task: None,
        };
        let cb = EvalContext {
            page: b,
            size_bytes: *size_map.get(&b.path).unwrap_or(&0),
            task: None,
        };
        compare_by_sort_clauses(&ca, &cb, clauses)
    });
}

fn compare_by_sort_clauses(a: &EvalContext<'_>, b: &EvalContext<'_>, clauses: &[SortClause]) -> Ordering {
    for clause in clauses {
        let va = eval_expr(a, &clause.expr);
        let vb = eval_expr(b, &clause.expr);
        let cmp = compare_values_for_sort(&va, &vb);
        if cmp != Ordering::Equal {
            return match clause.order {
                SortOrder::Asc => cmp,
                SortOrder::Desc => cmp.reverse(),
            };
        }
    }
    a.page
        .path
        .to_lowercase()
        .cmp(&b.page.path.to_lowercase())
}

fn compare_values_for_sort(a: &DataviewValue, b: &DataviewValue) -> Ordering {
    match (a, b) {
        (DataviewValue::Null, DataviewValue::Null) => Ordering::Equal,
        (DataviewValue::Null, _) => Ordering::Greater,
        (_, DataviewValue::Null) => Ordering::Less,
        (DataviewValue::Number { value: na }, DataviewValue::Number { value: nb }) => {
            na.partial_cmp(nb).unwrap_or(Ordering::Equal)
        }
        (DataviewValue::Bool { value: ba }, DataviewValue::Bool { value: bb }) => ba.cmp(bb),
        _ => a
            .as_display_string()
            .to_lowercase()
            .cmp(&b.as_display_string().to_lowercase()),
    }
}

fn matches_source(
    page: &DataviewPageRecord,
    source: &SourceExpr,
    all_pages: &[DataviewPageRecord],
) -> bool {
    match source {
        SourceExpr::Tag(req_tag) => {
            let prefix = format!("{req_tag}/");
            page.tags
                .iter()
                .any(|t| t == req_tag || t.starts_with(&prefix))
        }
        SourceExpr::Folder(raw_folder) => {
            let clean = raw_folder.trim().replace('\\', "/").trim_matches('/').to_lowercase();
            if clean.is_empty() {
                true
            } else {
                let path_lower = page.path.to_lowercase();
                path_lower.starts_with(&format!("{clean}/"))
            }
        }
        SourceExpr::IncomingLink(raw_target) => {
            // Notas que enlazan hacia [[raw_target]] (equivalente a `FROM [[Nota]]`)
            let target_norm = normalize_link_cmp(raw_target);
            if let Some(target_page) = find_page_by_target(all_pages, &target_norm) {
                page.outlinks
                    .iter()
                    .any(|out| normalize_link_cmp(out) == normalize_link_cmp(&target_page.path)
                        || normalize_link_cmp(out) == target_norm)
            } else {
                page.outlinks
                    .iter()
                    .any(|out| normalize_link_cmp(out) == target_norm)
            }
        }
        SourceExpr::OutgoingLink(raw_source) => {
            // Notas enlazadas DESDE [[raw_source]] (`FROM outgoing([[Nota]])`)
            let source_norm = normalize_link_cmp(raw_source);
            if let Some(src_page) = find_page_by_target(all_pages, &source_norm) {
                src_page.outlinks.iter().any(|out| {
                    let out_norm = normalize_link_cmp(out);
                    out_norm == normalize_link_cmp(&page.path)
                        || out_norm == normalize_link_cmp(&page.stem)
                })
            } else {
                false
            }
        }
        SourceExpr::Not(inner) => !matches_source(page, inner, all_pages),
        SourceExpr::And(l, r) => matches_source(page, l, all_pages) && matches_source(page, r, all_pages),
        SourceExpr::Or(l, r) => matches_source(page, l, all_pages) || matches_source(page, r, all_pages),
    }
}

fn find_page_by_target<'a>(
    pages: &'a [DataviewPageRecord],
    target_norm: &str,
) -> Option<&'a DataviewPageRecord> {
    pages
        .iter()
        .find(|p| normalize_link_cmp(&p.path) == target_norm)
        .or_else(|| pages.iter().find(|p| p.stem.to_lowercase() == target_norm))
}

fn normalize_link_cmp(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .split('|')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .replace('\\', "/");
    let no_slash = trimmed.trim_matches('/');
    let no_ext = no_slash
        .strip_suffix(".md")
        .or_else(|| no_slash.strip_suffix(".MD"))
        .unwrap_or(no_slash);
    no_ext.to_lowercase()
}

fn eval_expr(ctx: &EvalContext<'_>, expr: &Expr) -> DataviewValue {
    match expr {
        Expr::Null => DataviewValue::Null,
        Expr::Bool(b) => DataviewValue::Bool { value: *b },
        Expr::Number(n) => DataviewValue::Number { value: *n },
        Expr::Text(s) => DataviewValue::Text { value: s.clone() },
        Expr::Tag(t) => DataviewValue::Text {
            value: format!("#{t}"),
        },
        Expr::Link(target) => DataviewValue::Link {
            path: target.clone(),
            label: target.clone(),
        },
        Expr::Field(name) => resolve_field_value(ctx, name),
        Expr::Not(inner) => DataviewValue::Bool {
            value: !eval_expr(ctx, inner).is_truthy(),
        },
        Expr::Binary { op, left, right } => eval_binary(ctx, *op, left, right),
        Expr::Call { name, args } => eval_call(ctx, name, args),
    }
}

fn eval_binary(ctx: &EvalContext<'_>, op: BinaryOp, left: &Expr, right: &Expr) -> DataviewValue {
    match op {
        BinaryOp::And => DataviewValue::Bool {
            value: eval_expr(ctx, left).is_truthy() && eval_expr(ctx, right).is_truthy(),
        },
        BinaryOp::Or => DataviewValue::Bool {
            value: eval_expr(ctx, left).is_truthy() || eval_expr(ctx, right).is_truthy(),
        },
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            let lv = eval_expr(ctx, left);
            let rv = eval_expr(ctx, right);
            let matched = match op {
                BinaryOp::Eq => values_equal(&lv, &rv),
                BinaryOp::Ne => !values_equal(&lv, &rv),
                BinaryOp::Lt => compare_scalar_values(&lv, &rv) == Some(Ordering::Less),
                BinaryOp::Le => matches!(
                    compare_scalar_values(&lv, &rv),
                    Some(Ordering::Less | Ordering::Equal)
                ),
                BinaryOp::Gt => compare_scalar_values(&lv, &rv) == Some(Ordering::Greater),
                BinaryOp::Ge => matches!(
                    compare_scalar_values(&lv, &rv),
                    Some(Ordering::Greater | Ordering::Equal)
                ),
                _ => false,
            };
            DataviewValue::Bool { value: matched }
        }
    }
}

fn values_equal(a: &DataviewValue, b: &DataviewValue) -> bool {
    match (a, b) {
        (DataviewValue::Null, DataviewValue::Null) => true,
        (DataviewValue::Null, _) | (_, DataviewValue::Null) => false,
        (DataviewValue::Bool { value: ba }, DataviewValue::Bool { value: bb }) => ba == bb,
        (DataviewValue::Number { value: na }, DataviewValue::Number { value: nb }) => {
            (na - nb).abs() < 1e-9
        }
        (DataviewValue::Link { path: pa, label: la }, DataviewValue::Link { path: pb, label: lb }) => {
            normalize_link_cmp(pa) == normalize_link_cmp(pb)
                || normalize_link_cmp(la) == normalize_link_cmp(lb)
        }
        (DataviewValue::Link { path, label }, other)
        | (other, DataviewValue::Link { path, label }) => {
            let other_norm = normalize_link_cmp(&other.as_display_string());
            !other_norm.is_empty()
                && (normalize_link_cmp(path) == other_norm
                    || normalize_link_cmp(label) == other_norm)
        }
        _ => {
            if let (Some(na), Some(nb)) = (a.as_number(), b.as_number()) {
                (na - nb).abs() < 1e-9
            } else {
                a.as_display_string().trim().to_lowercase()
                    == b.as_display_string().trim().to_lowercase()
            }
        }
    }
}

fn compare_scalar_values(a: &DataviewValue, b: &DataviewValue) -> Option<Ordering> {
    if matches!(a, DataviewValue::Null) || matches!(b, DataviewValue::Null) {
        return None;
    }
    if let (Some(na), Some(nb)) = (a.as_number(), b.as_number()) {
        return na.partial_cmp(&nb);
    }
    Some(
        a.as_display_string()
            .to_lowercase()
            .cmp(&b.as_display_string().to_lowercase()),
    )
}

fn eval_call(ctx: &EvalContext<'_>, name: &str, args: &[Expr]) -> DataviewValue {
    match name {
        "contains" | "icontains" | "econtains" => {
            if args.len() != 2 {
                return DataviewValue::Bool { value: false };
            }
            let haystack = eval_expr(ctx, &args[0]);
            let needle = eval_expr(ctx, &args[1]);
            let exact = name == "econtains";
            DataviewValue::Bool {
                value: value_contains(&haystack, &needle, exact),
            }
        }
        "startswith" => {
            if args.len() != 2 {
                return DataviewValue::Bool { value: false };
            }
            let h = eval_expr(ctx, &args[0]).as_display_string().to_lowercase();
            let p = eval_expr(ctx, &args[1]).as_display_string().to_lowercase();
            DataviewValue::Bool {
                value: h.starts_with(&p),
            }
        }
        "endswith" => {
            if args.len() != 2 {
                return DataviewValue::Bool { value: false };
            }
            let h = eval_expr(ctx, &args[0]).as_display_string().to_lowercase();
            let s = eval_expr(ctx, &args[1]).as_display_string().to_lowercase();
            DataviewValue::Bool {
                value: h.ends_with(&s),
            }
        }
        "length" => {
            if args.len() != 1 {
                return DataviewValue::Number { value: 0.0 };
            }
            let len = match eval_expr(ctx, &args[0]) {
                DataviewValue::List { items } => items.len(),
                DataviewValue::Text { value } => value.chars().count(),
                DataviewValue::Null => 0,
                other => other.as_display_string().chars().count(),
            };
            DataviewValue::Number { value: len as f64 }
        }
        "lower" => args
            .first()
            .map(|e| DataviewValue::Text {
                value: eval_expr(ctx, e).as_display_string().to_lowercase(),
            })
            .unwrap_or(DataviewValue::Null),
        "upper" => args
            .first()
            .map(|e| DataviewValue::Text {
                value: eval_expr(ctx, e).as_display_string().to_uppercase(),
            })
            .unwrap_or(DataviewValue::Null),
        "default" => {
            if args.is_empty() {
                return DataviewValue::Null;
            }
            let primary = eval_expr(ctx, &args[0]);
            if matches!(primary, DataviewValue::Null) {
                args.get(1).map(|e| eval_expr(ctx, e)).unwrap_or(DataviewValue::Null)
            } else {
                primary
            }
        }
        _ => DataviewValue::Null,
    }
}

fn value_contains(haystack: &DataviewValue, needle: &DataviewValue, exact: bool) -> bool {
    match haystack {
        DataviewValue::Null => false,
        DataviewValue::List { items } => items.iter().any(|item| {
            if exact {
                values_equal(item, needle)
            } else {
                let item_str = item.as_display_string().to_lowercase();
                let needle_str = needle.as_display_string().to_lowercase();
                let item_clean = item_str.trim_start_matches('#');
                let needle_clean = needle_str.trim_start_matches('#');
                values_equal(item, needle)
                    || (!needle_clean.is_empty()
                        && (item_clean == needle_clean
                            || item_clean.starts_with(&format!("{needle_clean}/"))
                            || item_str.contains(&needle_str)))
            }
        }),
        _ => {
            let h = haystack.as_display_string().to_lowercase();
            let n = needle.as_display_string().to_lowercase();
            !n.is_empty() && h.contains(&n)
        }
    }
}

fn resolve_field_value(ctx: &EvalContext<'_>, raw_name: &str) -> DataviewValue {
    let trimmed = raw_name.trim();
    let lower = trimmed.to_lowercase();
    let effective = lower.strip_prefix("this.").unwrap_or(&lower);

    // Propiedades intrínsecas de tarea cuando evaluamos dentro de `TASK`
    if let Some(task) = ctx.task {
        match effective {
            "completed" => {
                return DataviewValue::Bool {
                    value: task.completed,
                }
            }
            "status" => {
                return DataviewValue::Text {
                    value: if task.completed { "x" } else { " " }.to_string(),
                }
            }
            "text" => {
                return DataviewValue::Text {
                    value: task.text.clone(),
                }
            }
            "line" => {
                return DataviewValue::Number {
                    value: task.line as f64,
                }
            }
            "tags" => {
                return DataviewValue::List {
                    items: task
                        .tags
                        .iter()
                        .map(|t| DataviewValue::Text {
                            value: format!("#{t}"),
                        })
                        .collect(),
                }
            }
            _ => {}
        }
    }

    // Metadatos del archivo (`file.*`)
    if let Some(file_prop) = effective.strip_prefix("file.") {
        return resolve_file_property(ctx.page, ctx.size_bytes, file_prop);
    }

    // Alias comunes de Dataview sin prefijo `file.`
    if effective == "tags" {
        return DataviewValue::List {
            items: ctx
                .page
                .tags
                .iter()
                .map(|t| DataviewValue::Text {
                    value: format!("#{t}"),
                })
                .collect(),
        };
    }

    let raw_key_for_norm = trimmed
        .strip_prefix("this.")
        .or_else(|| trimmed.strip_prefix("THIS."))
        .unwrap_or(trimmed);

    let key_norm = normalize_field_key(raw_key_for_norm).unwrap_or_else(|| effective.to_string());
    match ctx.page.fields.get(&key_norm) {
        None => DataviewValue::Null,
        Some(values) if values.is_empty() => DataviewValue::Null,
        Some(values) if values.len() == 1 => parse_scalar_field_value(&values[0]),
        Some(values) => {
            let mut seen = BTreeSet::new();
            let items = values
                .iter()
                .filter(|v| seen.insert((*v).clone()))
                .map(|v| parse_scalar_field_value(v))
                .collect();
            DataviewValue::List { items }
        }
    }
}

fn resolve_file_property(
    page: &DataviewPageRecord,
    size_bytes: u64,
    file_prop: &str,
) -> DataviewValue {
    let (mtime_date, mtime_hm) = format_unix_secs_utc(page.modified_epoch_ms / 1000);
    match file_prop {
        "name" => DataviewValue::Text {
            value: page.stem.clone(),
        },
        "title" => DataviewValue::Text {
            value: page.title.clone(),
        },
        "path" => DataviewValue::Text {
            value: page.path.clone(),
        },
        "folder" => DataviewValue::Text {
            value: page
                .path
                .rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
                .unwrap_or_default(),
        },
        "ext" => DataviewValue::Text {
            value: "md".to_string(),
        },
        "link" => DataviewValue::Link {
            path: page.path.clone(),
            label: page.title.clone(),
        },
        "size" => DataviewValue::Number {
            value: size_bytes as f64,
        },
        "mtime" => DataviewValue::Text {
            value: format!("{mtime_date} {mtime_hm}"),
        },
        "mday" => DataviewValue::Text { value: mtime_date },
        "tags" | "etags" => DataviewValue::List {
            items: page
                .tags
                .iter()
                .map(|t| DataviewValue::Text {
                    value: format!("#{t}"),
                })
                .collect(),
        },
        "outlinks" => DataviewValue::List {
            items: page
                .outlinks
                .iter()
                .map(|l| DataviewValue::Link {
                    path: l.clone(),
                    label: l.clone(),
                })
                .collect(),
        },
        "inlinks" => DataviewValue::List {
            items: page
                .inlinks
                .iter()
                .map(|l| DataviewValue::Link {
                    path: l.clone(),
                    label: l.clone(),
                })
                .collect(),
        },
        _ => DataviewValue::Null,
    }
}

fn parse_scalar_field_value(raw: &str) -> DataviewValue {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DataviewValue::Text {
            value: String::new(),
        };
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return DataviewValue::Bool { value: true };
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return DataviewValue::Bool { value: false };
    }
    if let Ok(num) = trimmed.parse::<f64>() {
        if num.is_finite() {
            return DataviewValue::Number { value: num };
        }
    }
    if trimmed.starts_with("[[") && trimmed.ends_with("]]") && trimmed.len() >= 4 {
        let inner = &trimmed[2..trimmed.len() - 2];
        let target = inner.split('|').next().unwrap_or(inner).trim().to_string();
        let label = inner
            .split_once('|')
            .map(|(_, alias)| alias.trim().to_string())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| target.clone());
        return DataviewValue::Link {
            path: target,
            label,
        };
    }
    DataviewValue::Text {
        value: trimmed.to_string(),
    }
}

// ============================================================================
// Pruebas unitarias
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempVault {
        dir: std::path::PathBuf,
    }

    impl TempVault {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = env::temp_dir().join(format!(
                "romsidian_dv_test_{}_{}_{}",
                prefix,
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }
    }

    impl Drop for TempVault {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn seed_dataview_vault() -> (TempVault, Vault, VaultIndex) {
        let temp = TempVault::new("seed");
        let vault = Vault::open(&temp.dir).unwrap();

        vault
            .create_note(
                "Proyectos/Alfa.md",
                "---\nestado: En progreso\nprioridad: 2\n---\n# Proyecto Alfa\nresponsable:: Ana\n#proyecto/activo\n- [ ] Definir arquitectura #urgente\n- [x] Crear repositorio\nEnlace a [[Índice]].\n",
            )
            .unwrap();

        vault
            .create_note(
                "Proyectos/Beta.md",
                "---\nestado: Completado\nprioridad: 1\n---\n# Proyecto Beta\n[responsable:: Luis]\n#proyecto\n- [ ] Publicar versión\n",
            )
            .unwrap();

        vault
            .create_note(
                "Notas/Idea.md",
                "---\nestado: Borrador\nprioridad: 5\n---\n# Idea suelta\n#idea\nSin tareas por ahora.\n",
            )
            .unwrap();

        vault
            .create_note("Índice.md", "# Índice\nLista central [[Proyectos/Alfa]].\n")
            .unwrap();

        let (index, _) = VaultIndex::open_and_sync(&vault).unwrap();
        (temp, vault, index)
    }

    #[test]
    fn executes_table_query_with_from_where_sort_and_inline_fields() {
        let (_temp, vault, index) = seed_dataview_vault();

        let dql = r#"
            TABLE estado AS "Estado", responsable AS Resp, prioridad
            FROM #proyecto AND !"Notas"
            WHERE prioridad >= 1 AND estado != "Archivado"
            SORT prioridad DESC
        "#;

        let res = execute_dataview_query(&index, Some(&vault), dql).expect("Debe ejecutar TABLE");
        match res {
            DataviewQueryResult::Table {
                headers,
                rows,
                total_count,
            } => {
                assert_eq!(headers, vec!["Archivo", "Estado", "Resp", "prioridad"]);
                assert_eq!(total_count, 2);
                assert_eq!(rows.len(), 2);
                // Orden DESC por prioridad: Alfa (2) antes que Beta (1)
                assert_eq!(
                    rows[0][1],
                    DataviewValue::Text {
                        value: "En progreso".to_string()
                    }
                );
                assert_eq!(
                    rows[0][2],
                    DataviewValue::Text {
                        value: "Ana".to_string()
                    }
                );
                assert_eq!(rows[0][3], DataviewValue::Number { value: 2.0 });
                assert_eq!(
                    rows[1][2],
                    DataviewValue::Text {
                        value: "Luis".to_string()
                    }
                );
            }
            other => panic!("Se esperaba Table, obtuvo {other:?}"),
        }
    }

    #[test]
    fn executes_list_and_task_queries() {
        let (_temp, vault, index) = seed_dataview_vault();

        let list_res = execute_dataview_query(
            &index,
            Some(&vault),
            r#"LIST estado FROM "Proyectos" OR #idea SORT file.name ASC LIMIT 2"#,
        )
        .expect("Debe ejecutar LIST");
        match list_res {
            DataviewQueryResult::List { items, total_count } => {
                assert_eq!(total_count, 3);
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].path, "Proyectos/Alfa.md");
            }
            other => panic!("Se esperaba List, obtuvo {other:?}"),
        }

        let task_res = execute_dataview_query(
            &index,
            Some(&vault),
            r##"TASK FROM #proyecto WHERE !completed AND contains(tags, "#urgente")"##,
        )
        .expect("Debe ejecutar TASK");
        match task_res {
            DataviewQueryResult::Task { items, total_count } => {
                assert_eq!(total_count, 1);
                assert_eq!(items[0].path, "Proyectos/Alfa.md");
                assert!(items[0].text.contains("Definir arquitectura"));
                assert!(!items[0].completed);
            }
            other => panic!("Se esperaba Task, obtuvo {other:?}"),
        }
    }

    #[test]
    fn evaluates_inline_this_expressions() {
        let (_temp, vault, index) = seed_dataview_vault();

        let name = eval_dataview_inline(
            &index,
            Some(&vault),
            "Proyectos/Alfa.md",
            "`= this.file.name`",
        )
        .unwrap();
        assert_eq!(
            name,
            DataviewValue::Text {
                value: "Alfa".to_string()
            }
        );

        let resp = eval_dataview_inline(
            &index,
            Some(&vault),
            "Proyectos/Alfa.md",
            "= this.responsable",
        )
        .unwrap();
        assert_eq!(
            resp,
            DataviewValue::Text {
                value: "Ana".to_string()
            }
        );

        let size = eval_dataview_inline(
            &index,
            Some(&vault),
            "Proyectos/Alfa.md",
            "this.file.size",
        )
        .unwrap();
        match size {
            DataviewValue::Number { value } => assert!(value > 10.0),
            other => panic!("Se esperaba Number para size, obtuvo {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_or_malformed_dql() {
        let err_kw = parse_dataview_query("QUERY campos FROM #tag").unwrap_err();
        assert!(err_kw.0.contains("TABLE, LIST o TASK"));

        assert!(parse_dataview_query("SELECT * FROM notes").is_err());

        let err_unclosed =
            parse_dataview_query(r#"TABLE estado WHERE estado = "abierto"#).unwrap_err();
        assert!(err_unclosed.0.contains("sin cerrar"));
    }
}
