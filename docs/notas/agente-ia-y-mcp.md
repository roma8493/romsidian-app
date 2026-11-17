# Agente IA, Auto-Conexión PKM y Servidor MCP (Iteración v0.5)

> Diseño técnico de la arquitectura híbrida Rust + IA para analizar fuentes, generar `frontmatter`, asignar `#tags` coherentes y tejer `[[wikilinks]]` automáticamente dentro de ROMSIDIAN o desde clientes MCP externos sin costo adicional por token de API.

---

## 1. Objetivo y Problema a Resolver

- **Objetivo**: Permitir que un agente de IA (Claude Sonnet, Gemini Hydrogen/Flash/Pro u otros) analice las notas y fuentes de la bóveda (por ejemplo, videos de YouTube guardados en `Inbox/`, apuntes o PDFs destilados), genere metadatos YAML estandarizados, proponga `#etiquetas` consistentes con el historial de la bóveda y cree enlaces `[[wikilinks]]` entre conceptos relacionados de forma individual o en lote.
- **Restricción económica clave**: Evitar el pago de API por uso de tokens aprovechando:
  1. La suscripción mensual existente del usuario (ej. Claude Pro USD 20 vía CLI local autenticado o Claude Desktop).
  2. La cuota gratuita de **Gemini Free Tier (Google AI Studio)** compatible tanto en Windows como en Android.
  3. Un motor determinístico offline en Rust (**0 tokens**, < 50 ms) para menciones exactas, alias y normalización base.

---

## 2. Arquitectura Híbrida de 2 Capas

Para no agotar la ventana de contexto ni los límites de mensajes de la suscripción leyendo toda la bóveda en cada consulta, la solución separa el trabajo pesado de indexación (Rust/SQLite) del razonamiento semántico (IA).

### Capa 1: Motor Determinístico en Rust (`crates/core/src/ai_linker.rs` — 0 tokens, < 50 ms)

1. **`build_vault_knowledge_map()`**:
   - Extrae un mapa compacto de la bóveda desde SQLite (`notes`, `note_tags`, `note_fields`, `note_headings`, `links`) sin leer cuerpos completos.
   - Incluye: lista de notas (`rel_path`, `title`, `aliases`, enlaces entrantes/salientes), taxonomía de `#tags` ordenada por frecuencia, claves de `frontmatter` más usadas, encabezados principales (`H1..H3`) y listado de notas huérfanas.
2. **`suggest_note_connections(rel_path, max_candidates)`**:
   - Combina 3 señales locales en SQLite:
     - Coincidencias exactas o por `aliases` no enlazadas (`unlinked_mentions_for`).
     - Solapamiento de `#tags` y campos de `frontmatter`.
     - Similitud léxica BM25 vía FTS5 (`unicode61`) usando palabras clave del título, encabezados y cuerpo de la nota activa.
   - Devuelve únicamente las notas candidatas más relevantes con fragmentos acotados para que el modelo razone con contexto mínimo.
3. **`preview_ai_enrichment()` y `apply_ai_enrichment()`**:
   - Recibe una propuesta estructurada (`AiEnrichmentPlan`: propiedades `frontmatter` a agregar/actualizar, `#tags` sugeridos, reemplazos de texto a `[[wikilinks]]` y sección opcional `## Conexiones Relacionadas`).
   - **Previsualización (*diff*)**: calcula los cambios exactos sin tocar el disco.
   - **Aplicación idempotente y segura**:
     - Respeta zonas protegidas: no reescribe dentro de `frontmatter`, bloques cercados (` ``` ` / `~~~`), código inline (`` `...` ``), URLs (`https://...`), enlaces Markdown `[texto](url)` ni `[[wikilinks]]` ya existentes.
     - Fusiona `frontmatter` preservando claves previas del usuario y normaliza `#tags` sin duplicados.
     - Respalda con sincronización Git previa o posterior para reversión segura ante cambios en lote.

### Capa 2: Proveedores de IA (Cero costo extra de API)

Definidos detrás de un trait común (`AiProvider`) en el backend:

1. **`LocalCliProvider` (Windows / Escritorio)**:
   - Ejecuta por `stdio` (`std::process::Command`) el CLI oficial autenticado con la suscripción del usuario (`claude -p` para Claude Pro USD 20 o `gemini` CLI), pasando el prompt estructurado y esperando JSON validado.
   - Cero configuración de facturación API en la app; usa directamente la sesión activa en la PC.
2. **`GeminiHttpProvider` / `CompatibleHttpProvider` (Windows y Android)**:
   - Cliente HTTPS liviano (`ureq` + `rustls`, reutilizando el stack TLS sin OpenSSL ya validado en Android) apuntando a Gemini Free Tier (Google AI Studio) o endpoint OpenAI-compatible opcional.
   - Permite ejecutar el Copiloto PKM y la auto-conexión también desde el **Samsung Galaxy A34**.
   - La clave se guarda exclusivamente en el almacén seguro del SO (`keyring` en Windows / almacenamiento privado interno en Android), nunca dentro de la bóveda ni en `.romsidian/config` versionado.
3. **`DeterministicRustProvider` (Offline / 0 ms / 0 tokens)**:
   - Funciona sin conexión ni credenciales externas: convierte menciones exactas/alias en `[[wikilinks]]`, completa campos básicos de `frontmatter` (`title`, `created`, `tags`) y sugiere etiquetas por coincidencia con la taxonomía local.

---

## 3. Servidor MCP Nativo (`crates/mcp-server` / `romsidian-mcp`)

- Binario Rust independiente dentro del workspace Cargo (`crates/mcp-server`) que comparte directamente `romsidian-core`.
- Implementa **Model Context Protocol (MCP)** sobre `stdin`/`stdout` (JSON-RPC 2.0) sin dependencias pesadas para mantenerse dentro del presupuesto de **7,21 GB de RAM**.
- Permite conectar **Claude Desktop** (suscripción de USD 20), **Claude Code** o **Antigravity** directamente a cualquier bóveda de ROMSIDIAN.
- **Herramientas MCP expuestas**:
  - `vault_knowledge_map`: mapa compacto de notas, tags, claves de frontmatter y huérfanas.
  - `search_notes_fts`: búsqueda BM25 en SQLite FTS5 con filtro por etiqueta.
  - `run_dataview_dql`: ejecución segura de consultas `TABLE` / `LIST` / `TASK`.
  - `read_note` / `write_note`: lectura y escritura segura dentro de la bóveda (`resolve_safe_path_with_ext`).
  - `suggest_note_connections`: candidatos exactos, por tags y por similitud FTS5.
  - `auto_connect_notes`: previsualización o aplicación automática de `frontmatter`, `#tags` y `[[wikilinks]]` en nota única o carpeta en lote.
  - `create_knowledge_canvas`: genera un archivo `.canvas` (JSON Canvas 1.0) conectando automáticamente fuentes, resúmenes y notas relacionadas.

---

## 4. Interfaz en `apps/desktop` (`AiCopilotPanel.svelte`)

- **Acceso**: botón en la barra rápida (*Ribbon*), Paleta de Comandos (`Ctrl+P`) y atajo `Ctrl+Shift+A`.
- **Modos de operación**:
  1. **Auto-Conectar Nota Activa**: analiza la nota abierta y sugiere o aplica `frontmatter`, `#tags` y `[[wikilinks]]`.
  2. **Procesamiento en Lote (Carpeta / Bóveda)**: selecciona una carpeta (por defecto `Inbox/` o toda la bóveda), procesa notas nuevas o huérfanas y genera sus conexiones cruzadas y notas índice opcionales.
  3. **Crear Mapa `.canvas` de Fuentes**: genera un lienzo visual conectando notas de un tema o carpeta.
  4. **Chat Libre con el Agente**: conversación natural con acceso al mapa compacto de la bóveda y ejecución de herramientas locales.
- **Interruptor "Modo Vista Previa vs. Automático"**:
  - **Vista Previa**: muestra el diff de `frontmatter`, etiquetas y enlaces propuestos con checkboxes para aceptar/descartar antes de escribir en disco.
  - **Automático**: aplica los cambios de inmediato, reindexa SQLite en vivo y actualiza editor, backlinks y grafo 2D al instante.

---

## 5. Guía Rápida de Registro MCP (`romsidian-mcp`)

Compilar el binario liviano en release:

```powershell
$env:CARGO_BUILD_JOBS=2; cargo build --release -p romsidian-mcp
# Binario generado: target\release\romsidian-mcp.exe
```

### Claude Desktop (Windows — suscripción Claude Pro USD 20)

Editar `%APPDATA%\Claude\claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "romsidian": {
      "command": "C:\\Users\\Roma\\Downloads\\ROMSIDIAN\\target\\release\\romsidian-mcp.exe",
      "args": ["--vault", "C:\\Users\\Roma\\Downloads\\ROMSIDIAN\\test-vault"]
    }
  }
}
```

### Claude Code / Antigravity / Otros clientes MCP

Registrar desde consola o en `mcp_config.json` / `.mcp.json`:

```powershell
claude mcp add romsidian -- "C:\Users\Roma\Downloads\ROMSIDIAN\target\release\romsidian-mcp.exe" --vault "C:\Ruta\A\Tu\Boveda"
```

También acepta la variable de entorno `ROMSIDIAN_VAULT` si no se pasa `--vault`.
