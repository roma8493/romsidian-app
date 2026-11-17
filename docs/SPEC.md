# Especificación Técnica del Proyecto (PRD / Spec) — ROMSIDIAN (v0.1 .. v0.5 Agente IA, Auto-Conexión PKM y Servidor MCP)

## 1. Visión y Objetivo
- **Qué se construye**: **ROMSIDIAN**, una aplicación de notas en Markdown de código local primero (*local-first*) donde cada bóveda es una carpeta común de archivos `.md` en texto plano, con soporte nativo para `[[wikilinks]]`, backlinks, etiquetas `#tag`, propiedades YAML (`frontmatter`), búsqueda instantánea de texto completo (SQLite FTS5), editor con vista previa y sincronización segura mediante Git contra un repositorio privado de GitHub.
- **Para quién**: Uso personal del autor entre su PC con Windows (AMD Ryzen 3 7320U, 7,21 GB de RAM utilizables) y, en la siguiente etapa de despliegue móvil, su teléfono Android.
- **Por qué**: Disponer de una alternativa propia, ligera en RAM, rápida y sin suscripción paga para sincronizar notas Markdown manteniendo el control total de los archivos `.md` y garantizando **cero pérdida de datos** ante conflictos de edición.

---

## 2. Requerimientos Funcionales
1. **[RF-01] Gestión de Bóveda Local**: Abrir cualquier carpeta del sistema como bóveda, listar el árbol de directorios y notas `.md`, y crear, leer, editar, renombrar y eliminar notas en disco preservando codificación UTF-8 (tildes, eñes, símbolos y nombres con espacios).
2. **[RF-02] Carpeta de Metadatos `.romsidian/`**: Inicializar automáticamente dentro de la bóveda la subcarpeta oculta `.romsidian/` (para ajustes locales e índice SQLite) y asegurar que `.romsidian/` figure siempre en el `.gitignore` de la bóveda.
3. **[RF-03] Parser de Markdown Conectado (`[[wikilinks]]`, `#tags`, `frontmatter`)**:
   - Detectar enlaces internos `[[Nombre de Nota]]` y con alias `[[Nombre de Nota|Texto visible]]`.
   - Extraer etiquetas `#etiqueta` (incluyendo jerárquicas `#tema/subtema`) en cuerpo y frontmatter.
   - Parsear el bloque YAML superior (`--- ... ---`) como mapa de propiedades clave-valor.
4. **[RF-04] Enlaces Entrantes (Backlinks) y Navegación**:
   - Al hacer clic en un `[[wikilink]]`, abrir la nota destino (o crearla si aún no existe).
   - Mostrar en un panel lateral o inferior todas las notas que enlazan hacia la nota activa (*backlinks*), junto con el fragmento de contexto.
5. **[RF-05] Índice y Búsqueda Instantánea (SQLite FTS5)**:
   - Mantener un índice incremental en `.romsidian/index.db` usando SQLite FTS5 (`unicode61`).
   - Permitir búsqueda instantánea por título, contenido y filtro por `#etiqueta` en menos de 50 ms para bóvedas de hasta 1.000 notas.
6. **[RF-06] Editor Markdown con Vista Previa y Temas (`apps/desktop`)**:
   - Editor basado en **CodeMirror 6** con resaltado de sintaxis Markdown, autocompletado de `[[wikilinks]]`, modo edición / vista previa renderizada y conmutador de tema claro/oscuro persistido.
7. **[RF-07] Sincronización Git con Cero Pérdida de Datos (`crates/sync-client`)**:
   - Implementar el trait `SyncBackend` con un backend Git (`GitSyncBackend` basado en `git2`) que ejecute `commit` local, `fetch`, fusión de 3 vías y `push` hacia un repositorio privado de GitHub.
   - **Resolución de conflictos sin sobrescritura**: si dos dispositivos modifican las mismas líneas de una nota o hay conflicto de edición/borrado, conservar la versión remota en `nota.md` y guardar la versión local íntegra en `nota (conflicto <dispositivo> <fecha>).md`, informando al usuario.
8. **[RF-08] Seguridad de Credenciales y Límites de Archivo**:
   - Almacenar el Personal Access Token (PAT) de GitHub exclusivamente en el almacén de credenciales del sistema operativo (`keyring` -> Windows Credential Manager), nunca en texto plano dentro de la bóveda.
   - Bloquear preventivamente el `commit` de archivos individuales mayores a 50 MB para evitar rechazos o bloqueos de GitHub.
9. **[RF-09] Paleta de Comandos (`Ctrl+P`), Notas Diarias (`/daily`) y Plantillas (`/templates`) (v0.2)**:
   - Ejecutar comandos del núcleo desde `Ctrl+P` / `Cmd+P` y barra lateral rápida (*Ribbon*).
   - Crear/abrir la nota diaria explícitamente en `/daily/YYYY-MM-DD.md` aplicando plantilla desde `/templates` con variables (`{{date}}`, `{{time}}`, `{{title}}`).
10. **[RF-10] Transclusiones (`![[...]]`) y Enlaces a Encabezados/Bloques (v0.2)**:
    - Soportar enlaces y transclusiones a notas completas (`![[Nota]]`), encabezados (`![[Nota#Encabezado]]`), bloques (`![[Nota#^id-bloque]]`) y recursos multimedia locales (`![[imagen.png]]`).
11. **[RF-11] Retroenlaces Completos (`Linked` + `Unlinked Mentions`) y Ubicaciones (v0.2)**:
    - Detectar menciones no enlazadas del título de la nota activa en toda la bóveda (respetando `Excluded files`) con acción **"Enlazar"** para convertirlas en `[[Nota]]`.
    - Proveer controles de contexto, orden y filtro, y 3 modos de visualización: panel derecho, vista incrustada al final del documento activo y modo fijado.
12. **[RF-12] Campos Inline y Motor de Consultas `Dataview` (DQL Sandboxed) (v0.2)**:
    - Indexar propiedades YAML tipadas y campos inline (`Clave:: Valor`, `[clave:: valor]`, `(clave:: valor)`) y tareas (`- [ ]` / `- [x]`).
    - Evaluar en sandbox sobre SQLite bloques ```` ```dataview ```` (`TABLE`, `LIST`, `TASK` con `FROM`, `WHERE`, `SORT`) y expresiones inline `` `= this.file.name` `` / `` `= this.<campo>` `` (manteniendo `DataviewJS` desactivado por defecto con interruptor opcional).
13. **[RF-13] Vista de Grafo 2D Interactiva (Global y Local) (v0.2)**:
    - Renderizar en HTML5 Canvas el grafo relacional de toda la bóveda (Global) o del vecindario de la nota activa (Local) con físicas de fuerzas, filtros por `#etiqueta`/carpeta/huérfanas, grupos de color y navegación por clic.
14. **[RF-14] Lienzo Visual JSON Canvas (`.canvas`) (v0.2)**:
    - Crear y editar archivos `.canvas` bajo el estándar abierto JSON Canvas: tarjetas de Texto (con conversión a `.md`), Notas (con *narrow to heading/block*), Medios, Páginas Web y Grupos; conectores con etiqueta, direccionalidad (`Nondirectional`, `Unidirectional`, `Bidirectional`) y `Follow connection`; herramientas de alineación/distribución y exportación a PNG con `Privacy mode`.
15. **[RF-15] Motor de Mapa de Conocimiento y Auto-Conexión PKM (`crates/core/src/ai_linker.rs`) (v0.5)**:
    - Generar un mapa compacto de toda la bóveda (`build_vault_knowledge_map`) desde SQLite (rutas, títulos, `aliases`, taxonomía de `#tags` con frecuencias, claves de `frontmatter`, encabezados `H1..H3` y notas huérfanas) con 0 tokens y latencia < 50 ms.
    - Detectar conexiones candidatas (`suggest_note_connections`) combinando menciones exactas/alias (`unlinked_mentions_for`), solapamiento de `#tags` y similitud léxica BM25 (FTS5).
    - Previsualizar (*diff*) y aplicar de forma idempotente (`preview_ai_enrichment` / `apply_ai_enrichment`) metadatos `frontmatter`, `#tags` y `[[wikilinks]]` (o sección `## Conexiones Relacionadas`) respetando bloques de código, URLs, enlaces existentes y frontmatter previo.
16. **[RF-16] Conectores Multi-Proveedor de IA sin Costo Extra de API (v0.5)**:
    - Definir un adaptador unificado de proveedores de IA en backend:
      - `LocalCliProvider` (Windows): puente `stdio` hacia el CLI local autenticado con la suscripción del usuario (`claude -p` de Claude Pro o `gemini` CLI).
      - `GeminiHttpProvider` / `CompatibleHttpProvider` (Windows y Android): cliente HTTPS liviano (`ureq` + `rustls`) compatible con Gemini Free Tier (Google AI Studio) para operar también desde Android, guardando cualquier credencial en el almacén seguro del SO (`keyring` / almacenamiento privado Android), nunca dentro de la bóveda.
      - `DeterministicRustProvider` (Offline): motor local en Rust de 0 tokens para enlazar menciones exactas/alias y completar metadatos básicos sin conexión.
17. **[RF-17] Servidor MCP Nativo (`crates/mcp-server` / `romsidian-mcp`) (v0.5)**:
    - Exponer el motor `crates/core` mediante **Model Context Protocol (MCP)** sobre `stdin`/`stdout` (JSON-RPC 2.0) para operar la bóveda directamente desde Claude Desktop, Claude Code o Antigravity.
    - Proveer herramientas de alto nivel: `vault_knowledge_map`, `search_notes_fts`, `run_dataview_dql`, `read_note`, `write_note`, `suggest_note_connections`, `auto_connect_notes` y `create_knowledge_canvas`.
18. **[RF-18] Panel Copiloto PKM y Auto-Conexión en Lote (`apps/desktop`) (v0.5)**:
    - Incorporar `AiCopilotPanel.svelte` (accesible desde Ribbon, `Ctrl+P` y `Ctrl+Shift+A`) con acciones rápidas (*Auto-Conectar Nota Activa*, *Analizar y Conectar Carpeta/Fuentes en Lote* como `Inbox/`, *Crear Mapa `.canvas` de Fuentes*) y chat libre con contexto de la bóveda.
    - Incluir interruptor **"Modo Vista Previa vs. Automático"** para revisar cambios de `frontmatter`, `#tags` y `[[wikilinks]]` antes de guardar o aplicarlos directamente con respaldo Git y reindexado inmediato en SQLite.

---

## 3. Requerimientos Técnicos y Arquitectura
- **Stack / Dependencias**:
  - **Lenguaje de núcleo y backend**: Rust (`rustc 1.98.1`, edición 2021).
  - **Crates principales de `crates/core`**: `rusqlite` (con feature `bundled` para incluir SQLite + FTS5 sin dependencias externas del sistema), `pulldown-cmark` o parser dedicado + `regex`, `serde`, `serde_json`, `serde_yaml`, `thiserror`, `sha2` / `walkdir`.
  - **Crates principales de `crates/sync-client`**: `git2` (`libgit2` embebido para portabilidad entre Windows y Android sin requerir `git.exe`), `keyring` (credenciales del SO).
  - **Shell y Frontend (`apps/desktop`)**: **Tauri v2** + **Svelte 5** + **Vite** + **TypeScript** + **CodeMirror 6** + `marked` / `dompurify` (para vista previa segura). Diseño CSS responsivo preparado tanto para ventana de escritorio como para pantalla táctil móvil.
- **Estructura de Módulos**:
  - `Cargo.toml` (raíz): Workspace de Cargo integrando `crates/core`, `crates/sync-client`, `crates/mcp-server` y `apps/desktop/src-tauri`.
  - `crates/core`: Librería pura de dominio (bóveda, parser, grafo de enlaces, motor DQL, motor `ai_linker` e índice SQLite FTS5).
  - `crates/sync-client`: Librería de sincronización (`trait SyncBackend`, implementación Git, detector de conflictos y gestor de credenciales).
  - `crates/mcp-server`: Binario servidor Model Context Protocol (`romsidian-mcp`) sobre `stdio` (JSON-RPC 2.0) que expone `crates/core` a Claude Desktop, Claude Code y Antigravity.
  - `apps/desktop`: Aplicación Tauri 2 (frontend Svelte 5 en `apps/desktop/src/` y puente de comandos Tauri en `apps/desktop/src-tauri/`).
  - `test-vault/`: Bóveda de prueba local con notas de ejemplo (con tildes, eñes, wikilinks y etiquetas) para pruebas automatizadas y manuales.

---

## 4. Casos Borde y Limitaciones
- **Nombres con tildes, eñes y espacios**: Los enlaces `[[Análisis Macroeconómico]]` deben resolver correctamente al archivo `Análisis Macroeconómico.md` sin sensibilidad a mayúsculas/minúsculas en Windows, pero preservando el nombre exacto para compatibilidad con Linux/Android.
- **Notas huérfanas o enlaces rotos**: Un `[[wikilink]]` hacia una nota que aún no existe no debe fallar; debe mostrarse como enlace pendiente y crear el archivo `.md` al hacer clic sobre él.
- **Corrupción o borrado de `.romsidian/index.db`**: Si el archivo SQLite no existe o se elimina, `crates/core` debe reconstruirlo automáticamente a partir de los archivos `.md` de la bóveda (los `.md` son la única fuente de la verdad).
- **Corte de red durante el `push` / `fetch`**: La operación debe abortar limpiamente devolviendo un estado de error legible sin dejar el repositorio Git en estado corrupto ni perder el commit local.
- **Compilación en 7,21 GB de RAM**: Se prioriza dependencias livianas y, en caso de presión de memoria durante compilaciones pesadas de Tauri/Rust, se limita el paralelismo con `CARGO_BUILD_JOBS=2`.
- **Desambiguación de Dominio (v0.2)**: Se descartan los módulos de *Obsidian Enterprise 2.2 de Ozona* (monitoreo ITIL/SLA con Nagios/Zabbix en RHEL/openSUSE) presentes en el documento fuente, implementando exclusivamente el ecosistema **Obsidian PKM** más empaquetado Android en una fase posterior con Android SDK/NDK.

---

## 5. Criterios de Aceptación (Definición de Terminado)
- [x] El workspace de Rust compila sin errores (`cargo check --workspace`) y pasa toda la suite de pruebas (`cargo test --workspace`).
- [x] `crates/core` indexa y busca sobre 1.000 notas sintéticas con tildes, `[[wikilinks]]`, `#etiquetas` y `frontmatter` en menos de 1 segundo de indexación incremental y menos de 50 ms por consulta FTS5.
- [x] `crates/sync-client` supera pruebas automatizadas de sincronización entre dos clones locales simulando edición concurrente, fusión limpia en líneas distintas y creación automática de `nota (conflicto ...).md` cuando hay colisión en la misma línea.
- [x] `apps/desktop` compila su frontend Svelte 5 (`npm run build`) y su binario Tauri 2 (`cargo check` / `cargo build`), permitiendo abrir `test-vault/`, navegar el árbol de archivos, editar con CodeMirror 6, seguir `[[wikilinks]]`, ver backlinks, buscar con FTS5, alternar tema claro/oscuro y ejecutar sincronización.
- [x] **(v0.2)** `crates/core` parsea e indexa campos inline (`Clave:: Valor`, `[clave:: valor]`, `(clave:: valor)`), tareas `- [ ]`, bloques `^id`, menciones no enlazadas (`Unlinked Mentions` respetando `excluded_files`) y ejecuta consultas DQL (`TABLE`, `LIST`, `TASK`) y expresiones inline `` `= this.file.*` `` con tests unitarios en verde.
- [x] **(v0.2)** `apps/desktop` integra Paleta de Comandos (`Ctrl+P`), Notas Diarias (`/daily`) + Plantillas (`/templates`), Transclusiones (`![[...]]`), Backlinks avanzados (con botón *Enlazar* y vista al pie), Vista de Grafo 2D (Global y Local) y editor JSON Canvas (`.canvas` con alineación y exportación PNG + *Privacy mode*), compilando sin errores (`npm --prefix apps/desktop run build` y `cargo test --workspace`).
- [x] **(v0.3)** `apps/desktop` incorpora `@tauri-apps/cli` (`npm --prefix apps/desktop run tauri dev` / `build`), versión unificada `0.2.0`, supera la prueba en vivo sobre `test-vault/` (y validación GitHub si está configurada) y genera el instalador Windows NSIS (`.exe`) funcional dentro del límite de 7,21 GB de RAM (`CARGO_BUILD_JOBS=2`).
- [x] **(v0.4)** Compilación cruzada Android (`aarch64-linux-android`), bóveda automática en almacenamiento interno, sincronización HTTPS GitHub (`ureq` + `rustls`), `custom-protocol`, Share Target para videos de YouTube en `Inbox/` y generación del APK firmado `ROMSIDIAN-Samsung-A34.apk` verificado con `apksigner`.
- [x] **(v0.5)** `crates/core` (`ai_linker.rs`) genera el mapa compacto de conocimiento, sugiere conexiones (`unlinked mentions` + tags + FTS5 BM25) y previsualiza/aplica `frontmatter`, `#tags` y `[[wikilinks]]` de forma idempotente y segura; `crates/mcp-server` (`romsidian-mcp`) expone herramientas MCP sobre `stdio` (JSON-RPC 2.0); y `apps/desktop` integra los conectores de IA (`LocalCliProvider`, `GeminiHttpProvider` Desktop/Android y `DeterministicRustProvider`) junto a `AiCopilotPanel.svelte` con modo Vista Previa / Automático y ejecución en lote.

---

## 6. Plan de Ejecución (Checklist Atómica)

- [x] **Tarea 1: Inicialización del Workspace Rust y Bóveda de Prueba (`test-vault/`)**
  - Crear `Cargo.toml` de workspace en la raíz, esqueletos compilables de `crates/core` (`romsidian-core`) y `crates/sync-client` (`romsidian-sync`), y crear la bóveda de prueba `test-vault/` con 5 notas `.md` conectadas con tildes, `[[wikilinks]]`, `#etiquetas` y `frontmatter`.
  - *Verificación*: `cargo test --workspace`

- [x] **Tarea 2: Gestión de Bóveda y Sistema de Archivos en `crates/core` (`vault.rs`)**
  - Implementar apertura e inicialización de bóveda (creación de `.romsidian/` y `.gitignore`), listado recursivo de notas `.md` ignorando carpetas ocultas, y operaciones CRUD seguras (crear, leer, guardar, renombrar y eliminar notas en UTF-8).
  - *Verificación*: `cargo test -p romsidian-core vault`

- [x] **Tarea 3: Parser de Markdown, `[[wikilinks]]`, `#etiquetas` y `frontmatter` en `crates/core` (`parser.rs`)**
  - Implementar el extractor de metadatos de cada nota: bloque YAML inicial (`frontmatter`), enlaces salientes `[[destino]]` y `[[destino|alias]]` con contexto de línea, y etiquetas `#tag` / `#padre/hijo` (ignorando encabezados Markdown y bloques de código).
  - *Verificación*: `cargo test -p romsidian-core parser`

- [x] **Tarea 4: Índice SQLite FTS5, Backlinks y Benchmark de 1.000 Notas en `crates/core` (`index.rs`)**
  - Implementar la base de datos SQLite en `.romsidian/index.db` (`rusqlite` bundled + FTS5), indexación incremental por hash SHA-256, consulta de búsqueda de texto completo, listado de notas por `#etiqueta` y resolución de **backlinks** entrantes. Incluir test con 1.000 notas con tildes y espacios.
  - *Verificación*: `cargo test -p romsidian-core index`

- [x] **Tarea 5: Motor de Sincronización Git y Resolución de Conflictos en `crates/sync-client`**
  - Definir el trait `SyncBackend`, implementar gestor de credenciales seguro (`keyring` + fallback en tests), límite de tamaño de archivo (50 MB) y `GitSyncBackend` con `git2`: commit local, fetch, merge de 3 vías y creación automática de `nota (conflicto <dispositivo> <fecha>).md` cuando hay colisión.
  - *Verificación*: `cargo test -p romsidian-sync`

- [x] **Tarea 6: Estructura Base de Tauri 2 + Svelte 5 y Comandos IPC (`apps/desktop`)**
  - Crear la aplicación en `apps/desktop` (frontend Svelte 5 + Vite + TypeScript y backend Tauri 2 en `src-tauri`) exponiendo los comandos IPC hacia `romsidian-core` y `romsidian-sync` (abrir bóveda, árbol de notas, CRUD de nota, búsqueda FTS5, backlinks, etiquetas y sincronización).
  - *Verificación*: `npm --prefix apps/desktop run build` y `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml`

- [x] **Tarea 7: Interfaz de Usuario Completa con CodeMirror 6, Backlinks y Búsqueda (`apps/desktop/src`)**
  - Implementar en Svelte 5 el layout de 3 paneles (barra lateral con árbol de archivos y etiquetas, panel central con editor **CodeMirror 6** + clic en `[[wikilinks]]` + vista previa Markdown, y panel derecho de **backlinks** y propiedades), modal de búsqueda instantánea FTS5 y selector de tema claro/oscuro.
  - *Verificación*: `npm --prefix apps/desktop run build`

- [x] **Tarea 8: Integración de UI de Sincronización, Pruebas End-to-End y Cierre del MVP**
  - Conectar en la interfaz el panel/modal de configuración de repositorio Git + token seguro (Credential Manager), indicador de estado de sincronización y avisos de copias en conflicto; correr la batería completa de pruebas del workspace y documentar instrucciones de uso.
  - *Verificación*: `cargo test --workspace` y `npm --prefix apps/desktop run build`

---

### Iteración v0.2 — Paridad Funcional con el Manual de Obsidian PKM

- [x] **Tarea 9: Parser e Índice Extendido en `crates/core` (Campos Inline, Tareas, Bloques y Menciones No Enlazadas)**
  - Extender `parser.rs` e `index.rs` para extraer e indexar campos inline (`Clave:: Valor`, `[clave:: valor]`, `(clave:: valor)`), listas de tareas (`- [ ]` / `- [x]`), encabezados e IDs de bloque (`^id`), transclusiones (`![[...]]`) y consulta de **Menciones No Enlazadas** (*Unlinked Mentions* respetando patrones `excluded_files` + conversión a `[[Nota]]`).
  - *Verificación*: `cargo test -p romsidian-core`

- [x] **Tarea 10: Motor de Consultas `Dataview` (DQL Sandboxed), Soporte `.canvas` y Comandos IPC v0.2 (`crates/core` y `apps/desktop/src-tauri`)**
  - Implementar en `crates/core` (`dataview.rs`) el motor declarativo DQL (`TABLE`, `LIST`, `TASK` con `FROM`, `WHERE`, `SORT`) y evaluador de expresiones inline `` `= this.file.*` `` / `` `= this.<campo>` `` sobre SQLite; añadir CRUD de archivos `.canvas` y notas diarias/plantillas en `vault.rs`, y exponer los comandos IPC correspondientes en `apps/desktop/src-tauri/src/lib.rs`.
  - *Verificación*: `cargo test --workspace`

- [x] **Tarea 11: Paleta de Comandos (`Ctrl+P`), Notas Diarias (`/daily` + `/templates`), Transclusiones (`![[...]]`) y Bloques DQL en UI**
  - Implementar `CommandPaletteModal.svelte` (`Ctrl+P`) y barra rápida (*Ribbon*), creación/apertura de Nota Diaria aplicando plantilla (`{{date}}`, `{{time}}`, `{{title}}`), y renderizado en `MarkdownEditor.svelte` / `markdown.ts` de transclusiones `![[Nota#Encabezado]]`, campos inline, bloques ```` ```dataview ```` y expresiones inline (con interruptor opcional para `DataviewJS` desactivado por defecto).
  - *Verificación*: `npm --prefix apps/desktop run build`

- [x] **Tarea 12: Panel Completo de Retroenlaces (`Linked` + `Unlinked Mentions`), Controles y Vista al Pie del Documento**
  - Actualizar `BacklinksPanel.svelte` y `MarkdownEditor.svelte` con pestaña/sección de **Menciones No Enlazadas** y botón *"Enlazar"*, controles de colapsar/expandir contexto, ordenamiento (nombre/fecha), filtro de búsqueda, ajustes de `Excluded files` y los 3 modos de ubicación (panel derecho, incrustado al final del documento activo y pestaña fijada).
  - *Verificación*: `npm --prefix apps/desktop run build`

- [x] **Tarea 13: Vista de Grafo 2D Interactiva (Global y Local) (`GraphView.svelte`)**
  - Construir en Svelte 5 + HTML5 Canvas (`GraphView.svelte`) la vista de Grafo Global y Grafo Local (vecindario de la nota activa) con simulación de fuerzas 2D, zoom/paneo, arrastre de nodos, filtros por `#etiqueta`/carpeta/huérfanas, grupos de color y apertura de nota al hacer clic.
  - *Verificación*: `npm --prefix apps/desktop run build`

- [x] **Tarea 14: Lienzo Visual JSON Canvas (`.canvas`), Exportación PNG con Modo Privacidad y Cierre v0.2 (`CanvasEditor.svelte`)**
  - Implementar el editor interactivo de archivos `.canvas` (JSON Canvas): los 5 tipos de tarjetas (Texto con conversión a `.md`, Nota con acotación a encabezado/bloque, Multimedia, Página Web y Grupos con fondo), conectores direccionales con etiquetas y `Follow connection`, barra de alineación/distribución en cuadrícula y exportación a imagen PNG con `Privacy mode`. Ejecutar pruebas integrales y cerrar la versión v0.2.
  - *Verificación*: `cargo test --workspace` y `npm --prefix apps/desktop run build`

---

### Iteración v0.3 — Ergonomía Tauri, Pruebas Reales en Windows e Instalador NSIS

- [x] **Tarea 15: Integración de `@tauri-apps/cli`, Script `tauri` y Versión `0.2.0`**
  - Instalar `@tauri-apps/cli` en `devDependencies` y agregar `"tauri": "tauri"` en `apps/desktop/package.json`. Subir la versión a `0.2.0` (`package.json`, `tauri.conf.json` y manifiestos Cargo aplicables) y fijar `"targets": ["nsis"]` en `apps/desktop/src-tauri/tauri.conf.json`.
  - *Verificación*: `npm --prefix apps/desktop run tauri -- --version`, `npm --prefix apps/desktop run build` y `$env:CARGO_BUILD_JOBS=2; cargo test --workspace`.

- [x] **Tarea 16: Prueba Funcional en Vivo (`tauri dev`) sobre `test-vault/`, Bugfixes y Sync GitHub Real**
  - Ejecutar `npm --prefix apps/desktop run tauri dev` (`CARGO_BUILD_JOBS=2`), recorrer la lista de verificación funcional (v0.1 + v0.2) sobre `test-vault/`, corregir cualquier fallo visual, de estado o IPC que surja, y validar sincronización real contra el repositorio privado de GitHub si el usuario lo provee durante la prueba.
  - *Verificación*: `tsc --noEmit`, `npm --prefix apps/desktop run build` y `$env:CARGO_BUILD_JOBS=2; cargo test --workspace`.

- [x] **Tarea 17: Empaquetado Windows NSIS (`.exe`) y Cierre v0.3**
  - Generar el instalador Windows NSIS mediante `npm --prefix apps/desktop run tauri build` con `CARGO_BUILD_JOBS=2`, comprobar el artefacto `.exe` generado, documentar pasos de ejecución e instalación en `README.md` y cerrar la Iteración v0.3 en `docs/PROGRESS.md` (dejando Android e historial de recuperación local para v0.4).
  - *Verificación*: instalador NSIS `.exe` generado con código de salida `0` y `$env:CARGO_BUILD_JOBS=2; cargo test --workspace`.

---

### Iteración v0.4 — Empaquetado Nativo Android (Samsung Galaxy A34 / ARM64)

- [x] **Tarea 18: Cross-compilación Rust y Empaquetado APK Android Nativo Firmado**
  - Configurar Android SDK 34/36, NDK r26d y toolchain Rust `aarch64-linux-android`. Resolver incompatibilidad de `openssl-sys` en Windows host para target Android desacoplando OpenSSL en `Cargo.toml`. Parchear `BuildTask.kt` para resolver la restricción de enlaces simbólicos en Windows sin modo desarrollador. Compilar `libromsidian_desktop_lib.so` (14,3 MB) y ensamblar paquete APK con Gradle (`compileSdk = 36`). Firmar con `apksigner` con esquemas v2 y v3 generando el binario final instalable `ROMSIDIAN-Samsung-A34.apk`.
  - *Verificación*: `ROMSIDIAN-Samsung-A34.apk` generado y verificado con `apksigner verify --verbose`.

- [x] **Tarea 19: Bóveda Automática Android, Sync HTTPS GitHub (`ureq` + `rustls`), `custom-protocol` y UX Móvil**
  - Creación/apertura automática de bóveda en `app_data_dir()/vault`, almacén privado de PAT, transporte HTTPS puro en `crates/sync-client/src/android_github.rs`, habilitación de `custom-protocol` en `apps/desktop/src-tauri/Cargo.toml`, cierre automático de paneles laterales en móvil (`closeSidebarsOnMobile`) y cajón superpuesto de filtros en `GraphView.svelte`.
  - *Verificación*: APK instalado y validado en Samsung Galaxy A34 con sincronización real contra `repoROMSIDIANMOBILE`.

- [x] **Tarea 20: Android Share Target para Videos de YouTube (`Inbox/` + Frontmatter + Edición Continua)**
  - **Objetivo**: Aparecer en el menú *"Compartir"* de Android (`ACTION_SEND`, `text/plain`) para recibir links de YouTube (arranque en frío o `onNewIntent` con `singleTask`), crear una nota en la carpeta configurada (`Inbox/` por defecto, editable en Ajustes) y abrirla automáticamente en modo **Edición** para que el usuario pueda volver a YouTube, copiar el resumen y pegarlo debajo del link.
  - **Criterios de aceptación**:
    1. `AndroidManifest.xml` declara `<intent-filter>` para `android.intent.action.SEND` (`text/plain`) y `launchMode="singleTask"` en `MainActivity`.
    2. `MainActivity.kt` captura `EXTRA_TEXT` y `EXTRA_SUBJECT` tanto en `onCreate` como en `onNewIntent` y los expone al frontend/Tauri (evento + cola inicial para arranque en frío).
    3. Nombre de archivo: usa `<título>.md` sanitizado si llega título en `EXTRA_SUBJECT`/texto, o `YYYY-MM-DD HH-mm YouTube <id>.md` si solo llega URL (resolviendo colisiones sin sobrescribir).
    4. Contenido inicial con Frontmatter (`title`, `source: youtube`, `url`, `created: YYYY-MM-DD`, `tags: [youtube]`), el link clickable y la sección `## Resumen` lista para pegar.
    5. La nota se guarda en la carpeta `Inbox/` (configurable en Ajustes / `romsidian-settings`), se abre inmediatamente en modo **Edición** (`centerView = 'editor'`) cerrando los cajones laterales en móvil, y conserva el estado al cambiar a YouTube y regresar.
  - *Verificación*: `npm --prefix apps/desktop run build`, `$env:CARGO_BUILD_JOBS=2; cargo test --workspace` y recompilación + firma de `ROMSIDIAN-Samsung-A34.apk` para prueba real desde YouTube en Samsung Galaxy A34.

---

### Iteración v0.5 — Agente IA / Copiloto PKM, Auto-Conexión de Fuentes y Servidor MCP

- [x] **Tarea 21: Motor de Mapa de Conocimiento, Sugerencia de Conexiones y Auto-Inyección de `[[wikilinks]]`, `#tags` y `frontmatter` en `crates/core` (`ai_linker.rs`)**
  - Implementar `build_vault_knowledge_map()`, `suggest_note_connections(rel_path, max_candidates)` (combinando `unlinked_mentions_for`, solapamiento de `#tags`/campos y similitud BM25 en SQLite FTS5), `preview_ai_enrichment()` y `apply_ai_enrichment()` (fusión segura de `frontmatter`, normalización de `#tags`, inyección idempotente de `[[wikilinks]]` protegiendo código/URLs/enlaces existentes y sección opcional `## Conexiones Relacionadas` + generador de `.canvas` temático).
  - *Verificación*: `$env:CARGO_BUILD_JOBS=2; cargo test -p romsidian-core`

- [x] **Tarea 22: Servidor MCP Nativo (`crates/mcp-server` / `romsidian-mcp`) para Claude Desktop, Claude Code y Antigravity**
  - Crear el crate binario `crates/mcp-server` en el workspace Cargo (`Cargo.toml` raíz) implementando **Model Context Protocol** sobre `stdio` (JSON-RPC 2.0) sin dependencias pesadas.
  - Exponer las herramientas: `vault_knowledge_map`, `search_notes_fts`, `run_dataview_dql`, `read_note`, `write_note`, `suggest_note_connections`, `auto_connect_notes` y `create_knowledge_canvas`, validando rutas seguras (`resolve_safe_path_with_ext`) y dejando guía rápida de registro MCP.
  - *Verificación*: `$env:CARGO_BUILD_JOBS=2; cargo test -p romsidian-mcp` y prueba JSON-RPC `initialize` / `tools/list` sobre `stdin`/`stdout`.

- [x] **Tarea 23: Puente de Proveedores IA en Backend (`LocalCliProvider` Claude/Gemini + `GeminiHttpProvider` Desktop/Android + Almacén Seguro) y Comandos IPC v0.5**
  - Implementar el trait `AiProvider` en backend con `LocalCliProvider` (Windows: `claude -p` / `gemini` CLI por `stdio` usando la suscripción existente del usuario), `GeminiHttpProvider` / `CompatibleHttpProvider` (`ureq` + `rustls` para Windows y Android Free Tier, con clave guardada exclusivamente en `keyring` / almacenamiento privado Android) y `DeterministicRustProvider` (offline, 0 tokens).
  - Exponer los comandos IPC v0.5 en `DesktopCore` / `lib.rs` (detección de proveedores, gestión segura de credencial, mapa de bóveda, sugerencia/previsualización/aplicación de auto-conexión individual o en lote, generación de `.canvas` y chat PKM).
  - *Verificación*: `$env:CARGO_BUILD_JOBS=2; cargo test --workspace`

- [x] **Tarea 24: Panel Copiloto PKM en UI (`AiCopilotPanel.svelte`), Auto-Conexión en Lote (Vista Previa / Automático) y Cierre v0.5**
  - Implementar `AiCopilotPanel.svelte` (Ribbon, `Ctrl+P` y `Ctrl+Shift+A`), configuración del proveedor de IA en Ajustes y tipos/comandos en `types.ts` y `api.ts` (incluyendo fallback en navegador).
  - Incorporar acciones rápidas (*Auto-Conectar Nota Activa*, *Analizar y Conectar Carpeta/Fuentes en Lote* como `Inbox/`, *Crear Mapa `.canvas` de Fuentes*), chat libre con contexto de la bóveda e interruptor **"Modo Vista Previa vs. Automático"** con diff seleccionable y reindexado/refresco inmediato del editor, árbol, backlinks y grafo 2D.
  - *Verificación*: `npm --prefix apps/desktop run build`, `npx --prefix apps/desktop tsc --noEmit` y `$env:CARGO_BUILD_JOBS=2; cargo test --workspace`.

---

### Iteración v0.6 — ROMSIDIAN Web (Serverless GitHub REST API + `sessionStorage` + Multi-Bóveda)

- [x] **Tarea 25: Motor Web Multi-Bóveda en `sessionStorage` y Sincronización Real con GitHub REST API (`webGithubSync.ts` + `api.ts`)**
  - **Objetivo**: Permitir usar ROMSIDIAN desde cualquier navegador web en otra PC sin instalar nada, conectándose directamente a los repositorios privados de GitHub del usuario (mismas bóvedas que PC y Android) y guardando estado/credenciales únicamente en `sessionStorage` (se borra al cerrar la pestaña).
  - **Criterios de aceptación**:
    1. **Persistencia efímera segura (`sessionStorage`)**: Toda bóveda web, sus notas `.md`, lienzos `.canvas`, configuración Git (`remote_url`, `branch`, `auto_sync_mins`), árboles base (`baseShas`) y el token `PAT` viven exclusivamente en `window.sessionStorage` bajo claves aisladas por bóveda (y un botón explícito *"Cerrar sesión / Limpiar datos del navegador"* en la barra superior / modal de bóvedas para borrar `sessionStorage` al instante).
    2. **Multi-bóveda en navegador**: `listLocalVaults()`, `createLocalVault(name)` y `openVault(vaultPath)` en modo web (`!isTauriRuntime()`) gestionan múltiples bóvedas independientes en `sessionStorage`, permitiendo cambiar de bóveda, crear una nueva para otro repo de GitHub o descubrir/listar los repositorios de GitHub del usuario con su `PAT`.
    3. **Motor GitHub REST API en navegador (`apps/desktop/src/lib/webGithubSync.ts`)**: Implementar `pull` y `push` reales por HTTPS contra `https://api.github.com` (homólogo a `android_github.rs`):
       - **Pull**: Resuelve `owner/repo` desde URL HTTPS/SSH o formato `owner/repo`, obtiene el commit `HEAD` de la rama configurada (o `default_branch` si difiere), descarga el árbol recursivo (`git/trees/{sha}?recursive=1`) y los blobs `.md` / `.canvas` en UTF-8 (decodificando Base64 correctamente), actualiza archivos no modificados localmente y crea copias de conflicto si un mismo archivo cambió en ambos lados.
       - **Push**: Empaqueta las notas/canvas creados, editados o eliminados en la sesión web, crea los blobs, construye el nuevo árbol Git sobre el `base_tree` remoto, crea el commit (`ROMSIDIAN Web sync (...)`) y actualiza `refs/heads/{branch}` (o inicializa el repo si estaba vacío).
    4. **Paridad funcional en Web**: Árbol de archivos, editor CodeMirror 6 + vista previa, `[[wikilinks]]`, backlinks + *unlinked mentions*, búsqueda, etiquetas, propiedades/frontmatter, notas diarias, plantillas, Dataview DQL, Grafo 2D y Canvas `.canvas` operan sobre la bóveda web activa sincronizada con GitHub.
  - *Verificación*: `npx --prefix apps/desktop tsc -p apps/desktop/tsconfig.json --noEmit` y `npm --prefix apps/desktop run build`.

- [x] **Tarea 26: UX Web ("Conectar con GitHub" / Selector de Repos + Botón Cerrar Sesión) y Despliegue Automático en GitHub Pages**
  - **Objetivo**: Facilitar el ingreso rápido desde otra PC (conectar PAT + elegir repositorio o clonar bóveda en 1 clic, o cerrar sesión limpiando `sessionStorage`) y dejar configurado el despliegue estático gratuito (GitHub Pages / Vercel / Cloudflare).
  - **Criterios de aceptación**:
    1. En `VaultSwitcherModal.svelte` y `SyncModal.svelte`, cuando corre en navegador web, mostrar aviso claro de **Modo Web (`sessionStorage` — los datos y el token se borran al cerrar la pestaña)**, opción para listar los repositorios de GitHub del usuario ingresando su `PAT` para conectar una bóveda en 1 clic, y botón **"🔒 Cerrar sesión web (borrar todo)"**.
    2. Configurar `apps/desktop/vite.config.ts` con `base: './'` para que el bundle estático (`apps/desktop/dist`) funcione tanto dentro de Tauri (Windows/Android) como servido desde cualquier subruta de GitHub Pages (`https://<usuario>.github.io/<repo>/`).
    3. Crear el workflow `.github/workflows/deploy-web.yml` para publicar automáticamente la versión Web en **GitHub Pages** al hacer push, además de un script `npm --prefix apps/desktop run build:web`.
  - *Verificación*: `npm --prefix apps/desktop run build`, prueba de bundle estático y `$env:CARGO_BUILD_JOBS=2; cargo test --workspace` (garantizando que Desktop y Android siguen intactos).

