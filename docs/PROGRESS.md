# Bitácora de Progreso del Proyecto (memoria entre sesiones)

## Última Actualización
- **Fecha**: 2026-11-17
- **Fase**: **Iteración v0.6 COMPLETADA (Tareas 25 y 26): ROMSIDIAN Web Serverless (`sessionStorage` + GitHub REST API + Multi-Bóveda + GitHub Pages)**.
- **Resumen**:
  - **Motor Web Multi-Bóveda y Sincronización GitHub REST API ([webGithubSync.ts](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/src/lib/webGithubSync.ts) + [api.ts](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/src/lib/api.ts))**:
    - Persistencia efímera 100% en `window.sessionStorage` (`romsidian-web-vaults-v1`): todas las bóvedas, notas `.md`, lienzos `.canvas`, `baseShas`, cambios locales y el token `PAT` viven únicamente en la pestaña activa y se borran automáticamente al cerrarla (más función `logoutAndClearWebSession()` para borrado manual en 1 clic).
    - Cliente HTTPS directo contra `https://api.github.com` (`listUserGithubRepos` y `syncWebVaultWithGithub`) con codificación/decodificación Base64 UTF-8: descarga árboles recursivos y blobs `.md`/`.canvas`, fusiona cambios remotos, genera copias de conflicto (`(conflicto web <ts>)`) si el mismo archivo fue editado en ambos lados, y sube creaciones/ediciones/borrados mediante Git Data API (`blobs` -> `trees` -> `commits` -> `refs/heads/{branch}`) o inicializa repositorios vacíos vía Contents API.
    - Árbol jerárquico de carpetas (`buildBrowserFileTree`) y conteo dinámico de etiquetas (`buildBrowserTagCounts`) conectados a todas las funciones PKM en navegador (Editor CodeMirror 6, Vista Previa, `[[wikilinks]]`, Backlinks, *Unlinked Mentions*, Búsqueda, Notas Diarias, Plantillas, Dataview DQL, Grafo 2D, Canvas `.canvas` y Copiloto IA).
  - **UX Web y Despliegue Estático ([VaultSwitcherModal.svelte](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/src/lib/components/VaultSwitcherModal.svelte), [SyncModal.svelte](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/src/lib/components/SyncModal.svelte), [vite.config.ts](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/vite.config.ts), [deploy-web.yml](file:///c:/Users/Roma/Downloads/ROMSIDIAN/.github/workflows/deploy-web.yml))**:
    - En `VaultSwitcherModal.svelte` y `SyncModal.svelte`, en modo Web se muestra el banner de seguridad `sessionStorage`, el conector **"☁️ Conectar repositorio de GitHub (1 clic)"** (pegar PAT -> listar repositorios privados/públicos -> abrir y sincronizar bóveda) y el botón **"🔒 Cerrar sesión web"**.
    - Configurado `base: './'` en `apps/desktop/vite.config.ts`, script `npm --prefix apps/desktop run build:web` (`apps/desktop/dist`) y workflow `.github/workflows/deploy-web.yml` para despliegue automático en GitHub Pages.
    - **Repositorio de Código de la App**: [github.com/roma8493/romsidian-app](https://github.com/roma8493/romsidian-app).
    - **URL Pública GitHub Pages**: `https://roma8493.github.io/romsidian-app/`.
    - **Bóveda de Notas Remota**: [github.com/roma8493/repoROMSIDIANMOBILE](https://github.com/roma8493/repoROMSIDIANMOBILE).
  - **Verificación**:
    - `npx --prefix apps/desktop tsc -p apps/desktop/tsconfig.json --noEmit` (código `0`).
    - `npm --prefix apps/desktop run build:web` (`168 modules transformed`, bundle estático listo en `apps/desktop/dist`).
- **Fecha previa**: 2026-11-16
- **Fase**: **Post-v0.5: Gestor / Selector Multi-Bóveda (Android y PC)**.
- **Resumen**:
  - **Backend Rust (`apps/desktop/src-tauri/src/lib.rs`)**:
    - Se agregó `LocalVaultEntry` y los comandos IPC `list_local_vaults` y `create_local_vault` (con `sanitize_vault_folder_name`) para gestionar múltiples bóvedas en `<app_data_dir>/vaults/<nombre>` manteniendo intacta la bóveda inicial (`<app_data_dir>/vault` como `Principal`).
    - Se confirmó que `romsidian-sync` (`git_backend.rs`) aísla el token PAT por ruta (`vault::<root_path>`) y hace `fast_forward_to` automático cuando el repo local está vacío (`unborn`).
  - **Frontend (`types.ts`, `api.ts`, `VaultSwitcherModal.svelte`, `App.svelte`)**:
    - Se creó [VaultSwitcherModal.svelte](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/src/lib/components/VaultSwitcherModal.svelte) y se integró el botón `🗂 Bóvedas` en la barra superior y el comando `Cambiar o crear bóveda` en la Paleta de Comandos (`Ctrl+P`).
    - Se persistió la última bóveda abierta (`romsidian-last-vault`) y el historial de recientes (`romsidian-recent-vaults`) en `localStorage` para restaurar la bóveda activa al reiniciar tanto en Android como en PC.
  - **Verificación y Empaquetado**:
    - `npx tsc -p apps/desktop/tsconfig.json --noEmit` y `npm --prefix apps/desktop run build` en verde (`167 modules transformed`).
    - `$env:CARGO_BUILD_JOBS=2; cargo test --workspace`: **45/45 tests en verde** (27 `romsidian-core`, 9 `romsidian-desktop`, 4 `romsidian-mcp`, 5 `romsidian-sync`).
    - Compilación cruzada Android ARM64 (`libromsidian_desktop_lib.so`, `17,375,120` bytes) + Gradle `assembleArm64Release` + `zipalign` + firma con nuevo keystore (`apps/desktop/src-tauri/gen/android/romsidian-release-v2.keystore`, v2/v3 verificados) exitosos, generando `ROMSIDIAN-Samsung-A34.apk` (`19,532,312` bytes).
- **Fecha previa**: 2026-11-15
- **Fase**: **Post-v0.5: Manejo Seguro de Enlaces Externos y Persistencia / Exportación de Chat Copiloto IA**.
- **Resumen**:
  - **Apertura Segura de Enlaces Externos**:
    - Se agregó el comando IPC nativo `open_external_url` (`apps/desktop/src-tauri/src/lib.rs`) que valida esquemas `http://`, `https://` y `mailto:` y abre el navegador predeterminado del sistema operativo (`cmd /C start "" <url>` con `CREATE_NO_WINDOW` en Windows / `xdg-open` en Linux) sin bloquear el proceso.
    - Se integró un listener global en fase de captura (`document.addEventListener('click', ..., true)`) en `apps/desktop/src/main.ts` para interceptar todo clic en etiquetas `<a>` externas, evitando que WebView2 navegue internamente y pierda el estado de la aplicación.
    - Se configuró el renderizador de enlaces de `marked.use` en `apps/desktop/src/lib/markdown.ts` para inyectar `target="_blank" rel="noopener noreferrer" class="external-link" ↗`, y se autorizaron los atributos `target` y `rel` en `DOMPurify.sanitize`.
  - **Exportación a Nota y Persistencia del Chat Copiloto PKM (`AiCopilotPanel.svelte` y `App.svelte`)**:
    - Se agregó el botón `[📄 Guardar como nota]` en cada respuesta del Copiloto IA, permitiendo confirmar o editar la ruta sugerida de forma inteligente (derivando el título de encabezados `#` o del prompt del usuario y sanitizando caracteres prohibidos) antes de llamar a `createNote` y abrir la nota directamente en el editor.
    - Se añadió botón `[📋 Copiar]` en cada turno del asistente para copiar la respuesta en Markdown al portapapeles.
    - Se implementó persistencia local en `localStorage` (`romsidian-copilot-chat`) para `chatTurns`, evitando que recargas de página o cambios accidentales borren el historial de conversación, junto con un botón `[🗑 Limpiar chat]` para reiniciarlo a demanda.
  - **Actualización de Icono de la Aplicación y Recompilación de Binarios (PC + Android)**:
    - Se regeneraron los iconos de escritorio, web y Android (`apps/desktop/src-tauri/icons/`, `apps/desktop/public/icon.png` y `apps/desktop/src-tauri/gen/android/app/src/main/res/mipmap-*`) mediante `npx tauri icon` a partir de `apps/desktop/app-icon.png` sin modificar el código del Copiloto IA.
    - Se recompiló en Release para Windows (`npm --prefix apps/desktop run tauri build` con `CARGO_BUILD_JOBS=2`), actualizando `C:\Users\Roma\AppData\Local\ROMSIDIAN\romsidian-desktop.exe` (`16,327,680` bytes) y generando el instalador `target\release\bundle\nsis\ROMSIDIAN_0.2.0_x64-setup.exe`.
    - Se recompiló la biblioteca nativa ARM64 (`libromsidian_desktop_lib.so`, `17,318,280` bytes), se ensambló con Gradle (`assembleArm64Release`), se alineó con `zipalign` y se firmó con `romsidian-release.keystore` generando `ROMSIDIAN-Samsung-A34.apk` (`19,479,064` bytes, verificado con `apksigner verify --verbose`: v2 y v3 válidos).
  - **Verificación y Despliegue**:
    - `npx tsc --noEmit` y `npm --prefix apps/desktop run build` exitosos con 0 errores (Vite build en 4.13 s).
    - `$env:CARGO_BUILD_JOBS=2; cargo test --workspace` ejecutado en verde (44/44 tests pasados).
    - Binario de producción `romsidian-desktop.exe` recompilado en Release e instalado en `C:\Users\Roma\AppData\Local\ROMSIDIAN\romsidian-desktop.exe`.
- **Fecha previa**: 2026-11-14
- **Fase**: **Iteración v0.5 COMPLETADA — Fase 4, Tarea 24 completada** (Panel Copiloto PKM `AiCopilotPanel.svelte`, Bindings `types.ts`/`api.ts`, Selector IA en Ajustes y Cierre v0.5).
- **Resumen**:
  - Se extendió `apps/desktop/src/lib/types.ts` con todos los DTOs de la Iteración v0.5 (`VaultKnowledgeMap`, `CandidateConnection`, `ConnectionSignal`, `AiEnrichmentPlan`, `AiEnrichmentPreview`, `KnowledgeCanvasRequest`/`Result`, `AiProviderKind`/`Config`/`Status`, `AutoConnectMode`/`Request`/`NoteOutcome`/`BatchResponse`, `AiChatMessage`/`Request`/`Response`).
  - Se extendió `apps/desktop/src/lib/api.ts` con los 7 comandos IPC v0.5 (`getAiProviderStatus`, `saveAiProviderConfig`, `getVaultKnowledgeMap`, `suggestNoteConnections`, `autoConnectNotes`, `createKnowledgeCanvas`, `askAiCopilot`) más fallback determinista completo en navegador sobre `browserVaultNotes`/`browserVaultCanvases`.
  - Se creó `apps/desktop/src/lib/components/AiCopilotPanel.svelte` con 5 pestañas (`Nota Activa`, `Lote / Carpeta`, `Mapa .canvas`, `Chat PKM`, `Proveedor IA & MCP`), interruptor **"👁 Vista Previa vs. ⚡ Automático"**, visor de difs unificados con confirmación individual o en lote (reutilizando `custom_plan`), renderizado Markdown seguro (`DOMPurify` + clic delegado en `[[wikilinks]]`), gestión de API Keys en `CredentialStore` y snippet MCP copiable para `romsidian-mcp`.
  - Se integró el Copiloto IA en `apps/desktop/src/App.svelte` (Topbar, Ribbon `✦`, atajo `Ctrl+Shift+A`, comandos en Paleta `Ctrl+P`, selector rápido de proveedor en Ajustes y refresco reactivo de árbol, nota activa, backlinks, grafo y apertura de `.canvas` generado).
  - Verificación integral en verde: `npx tsc --noEmit` (código `0`), `npm --prefix apps/desktop run build` (`vite build` en 4.13 s, `165 modules transformed`) y `$env:CARGO_BUILD_JOBS=2; cargo test --workspace` (`44 passed; 0 failed`: `27` en `romsidian-core`, `8` en `romsidian-desktop`, `4` en `romsidian-mcp`, `5` en `romsidian-sync`).
- **Ajuste post-v0.5 (Acceso autónomo y fluido del Chat Copiloto IA / `ClaudeCli`)**:
  - `LocalCliProvider::run_cli_prompt` (`apps/desktop/src-tauri/src/ai_provider.rs`) fija `cmd.current_dir(vault_root)` y, para `ClaudeCli` en modo chat (`allow_read_tools = true`), pasa `--no-session-persistence --allowedTools "Read,Glob,Grep,LS" --add-dir <vault_root>` (y `--tools ""` al generar JSON de enriquecimiento).
  - `DesktopCore::gather_chat_context_notes` (`apps/desktop/src-tauri/src/lib.rs`) pre-carga e inyecta en `ChatPromptContext.context_notes` hasta `10` notas completas por coincidencia de carpeta (`/inbox`), título/stem/alias/`#tag`, hits FTS5 y bóvedas pequeñas (`<= 12` notas), refinando `STOPWORDS` en `extract_chat_keywords` y añadiendo la ruta de la bóveda abierta al prompt del Copiloto PKM.
  - Se recompiló `target\release\romsidian-desktop.exe` (`--features custom-protocol`) y se actualizó `C:\Users\Roma\AppData\Local\ROMSIDIAN\romsidian-desktop.exe`.
- **Despliegue y Registro MCP v0.5 (`romsidian-mcp.exe`)**:
  - Se compiló `$env:CARGO_BUILD_JOBS=2; cargo build --release -p romsidian-mcp` (`1m 04s`) y se instaló [romsidian-mcp.exe](file:///C:/Users/Roma/AppData/Local/ROMSIDIAN/romsidian-mcp.exe) (`3,091,456` bytes, `14/11/2026 20:34:52`).
  - Verificado en vivo por `stdio` JSON-RPC 2.0 sobre `test-vault` (`initialize`, `tools/list` con las 8 herramientas MCP y `tools/call` `vault_knowledge_map` con `isError: false` y `4` notas indexadas).
  - Registrado globalmente en **Claude Code** (`claude mcp add --scope user romsidian`, verificado con `claude mcp get romsidian` -> `Status: ✔ Connected`) y en **Antigravity** (`C:\Users\Roma\.gemini\config\mcp_config.json` y `C:\Users\Roma\.gemini\antigravity-ide\mcp_config.json`) apuntando a `C:\Users\Roma\Downloads\ROMSIDIAN\test-vault`.
- **Empaquetado APK Android v0.5 (Compilado, Alineado y Firmado)**:
  - Frontend (`vite build`, `165` módulos), biblioteca nativa ARM64 (`target\aarch64-linux-android\release\libromsidian_desktop_lib.so`, `17,014,728` bytes) y paquete final [ROMSIDIAN-Samsung-A34.apk](file:///C:/Users/Roma/downloads/romsidian/ROMSIDIAN-Samsung-A34.apk) (`18,692,296` bytes, `14/11/2026 21:59:10`) compilados, alineados y firmados con [romsidian-release.keystore](file:///C:/Users/Roma/downloads/romsidian/apps/desktop/src-tauri/gen/android/romsidian-release.keystore) (`CN=ROMSIDIAN`, `SHA-256: e2e3fac3cddc83bb06ce7260ce653558ae86ecae5673444544fad0a723131203`).
  - Firma verificada con `apksigner verify --verbose` (`v2: true`, `v3: true`) y archivo temporal `ROMSIDIAN-Samsung-A34-aligned.apk` limpiado.
- **Próxima tarea**: Todas las tareas planificadas en `docs/SPEC.md` (Tareas 1 a 24, Iteraciones v0.1 a v0.5 + binarios Windows Desktop, `romsidian-mcp` y APK Android `ROMSIDIAN-Samsung-A34.apk`) están 100% completadas y verificadas. Pendiente únicamente probar en el Samsung Galaxy A34 o definir alcance de una eventual Iteración v0.6 (Fase 2 / Fase 3).

## Decisiones Clave y Reglas Aprendidas
- **Decisiones técnicas de Tarea 24 (`AiCopilotPanel.svelte`, `types.ts`, `api.ts`, `App.svelte`)**:
  - En `AiCopilotPanel.svelte`, el modo Vista Previa envía el `AiEnrichmentPlan` ya calculado vía `custom_plan` al aplicar cambios individuales o en lote (`handleApplyPreviewedNote`, `handleApplySingleBatchItem`, `handleApplyAllBatchPending`), evitando una segunda llamada costosa a Claude/Gemini y garantizando fidelidad exacta entre el diff inspeccionado y el archivo guardado.
  - Se aisló la inicialización de pestaña en `$effect` mediante `prevOpen` y `lastHandledNotePath` para que navegar a una nota desde Chat o Candidatos mientras el Copiloto sigue abierto actualice las sugerencias sin reiniciar la pestaña activa.
  - Los `[[wikilinks]]` en las respuestas Markdown del Chat PKM usan `renderMarkdownPreview` (`DOMPurify`) y la acción DOM `use:delegateChatLinks`, manteniendo navegación directa sin advertencias de accesibilidad en Svelte 5 ni inyección HTML sin sanear.
  - El modal de Ajustes (`App.svelte`) ofrece tanto un selector rápido de proveedor IA (`saveAiProviderConfig`) como acceso directo a la pestaña de credenciales seguras (`CredentialStore`) y configuración JSON del servidor `romsidian-mcp`.
- **Decisiones técnicas de Tarea 23 (`apps/desktop/src-tauri/src/ai_provider.rs` y `lib.rs`)**:
  - Se reutilizó `ureq = { version = "2.10", default-features = false, features = ["tls", "json"] }` ya presente en `Cargo.lock` (sin OpenSSL, 100% compatible con compilación cruzada Android ARM64 y Windows) enviando la clave de Gemini por cabecera `x-goog-api-key` y excluyendo secretos de URLs y de `.romsidian/ai_config.json`.
  - Para `LocalCliProvider` en Windows se resuelven wrappers `.cmd`/`.bat`/`.exe` en `PATH` y carpetas estándar (`%APPDATA%\npm`, `%USERPROFILE%\.local\bin`, etc.), enviando el prompt por `stdin` bajo `cmd /C` con `CREATE_NO_WINDOW` para evitar parpadeos de consola y límites de longitud en argumentos.
  - `parse_and_merge_llm_plan` tolera fences Markdown y campos parciales mediante `extract_json_object` + DTO intermedio permisivo, verifica cada `[[wikilink]]` contra las rutas reales de `VaultKnowledgeMap` y fusiona señales deterministas locales; si el CLI/HTTP falla y `fallback_to_deterministic` está activo, degrada limpiamente al motor local con `warning`.
  - Se inyectó `Arc<dyn CredentialStore>` en `DesktopCore` (`DesktopCore::with_credential_store`) para aislar completamente las pruebas unitarias usando `MemoryCredentialStore` y copia temporal de `test-vault/` sin tocar el llavero real ni la bóveda del usuario.
- **Decisiones técnicas de Tarea 22 (`crates/mcp-server` / `romsidian-mcp`)**:
  - Transporte MCP sobre `stdio` implementado en Rust estándar (`std::io::BufRead` / `Write`) sin `tokio` ni SDK pesados para cuidar el presupuesto de 7,21 GB de RAM, con detección automática de formato (`JsonLine` con limpieza de BOM en Windows y `Content-Length`) respondiendo en el mismo formato del cliente.
  - Los errores de validación de dominio en `tools/call` (rutas inseguras `../`, `.romsidian`, `C:/...`, notas inexistentes o errores de sintaxis DQL) devuelven `CallToolResult` con `"isError": true` y mensaje legible para que Claude/Gemini se autocorrijan sin romper la sesión MCP; métodos desconocidos devuelven error JSON-RPC `-32601`/`-32602`.
  - Se endureció `normalize_safe_folder_filter` mediante `std::path::Component` para rechazar `ParentDir`, prefijos de unidad Windows (`C:`), raíces y directorios ocultos antes de filtrar notas en `auto_connect_notes`.
  - En `crates/core/src/ai_linker.rs`, se corrigió `seen_suggested_tags.insert(...)` en `build_deterministic_enrichment_plan` para insertar solo tras confirmar coincidencia en texto, garantizando que Pass 1 agregue las etiquetas de candidatos fuertes (`score >= 30.0`) y Pass 2 sea 100% idempotente (`changed_count == 0`).
- **Decisiones técnicas de Tarea 21 (`crates/core/src/ai_linker.rs`)**:
  - Se implementó el motor sin añadir dependencias externas (reutilizando `VaultIndex::conn`, `compute_protected_ranges` con protección ampliada para URLs `http://`/`https://`, `detect_code_fence`, `extract_frontmatter`, `normalize_tag` y `format_unix_secs_utc`).
  - `suggest_note_connections` combina 5 señales ponderadas: menciones salientes no enlazadas (`+45`), menciones entrantes (`+25`), etiquetas compartidas exactas/jerárquicas (`+15`), campos significativos en `note_fields` (`+12`) y similitud léxica BM25 en SQLite FTS5 (`+5..+20`), priorizando notas aún no enlazadas (`already_linked == false`).
  - `preview_ai_enrichment` / `apply_ai_enrichment` garantizan 100% de idempotencia protegiendo bloques de código cercados/inline, bloques `dataview`, comentarios HTML, encabezados `#`, URLs, enlaces Markdown y `[[wikilinks]]` previos, fusionando claves nuevas de YAML `frontmatter` y `#tags` sin sobrescribir claves existentes del usuario salvo solicitud explícita.
  - Se hizo resiliente la suite de `crates/core` ante modificaciones manuales en `test-vault/` replicando copias temporales con `ensure_canonical_seed_notes` sin alterar `test-vault/` en disco.
- **Decisiones cerradas en Fase 3 (Iteración v0.5 — Agente IA / Copiloto PKM y Servidor MCP)**:
  1. **Arquitectura Híbrida de 2 Capas (Ahorro de tokens y RAM)**: `crates/core/src/ai_linker.rs` precalcula el mapa compacto de la bóveda, menciones exactas/alias y candidatos BM25 (SQLite FTS5) en <50 ms (0 tokens) antes de invocar al modelo IA o aplicar cambios idempotentes en `.md`.
  2. **Conectores sin Costo Extra de API**: Soporte híbrido mediante `LocalCliProvider` en Windows (`claude -p` / `gemini` CLI usando la suscripción existente del usuario), `GeminiHttpProvider` (`ureq` + `rustls`, compatible con Gemini Free Tier en PC y Android con credenciales guardadas exclusivamente en el almacén seguro del SO) y `DeterministicRustProvider` offline.
  3. **Servidor MCP Nativo (`crates/mcp-server`)**: Binario `romsidian-mcp` sobre `stdio` (JSON-RPC 2.0) compartiendo `romsidian-core` para conectar la bóveda a Antigravity, Claude Desktop o Claude Code con herramientas PKM dedicadas.
  4. **Seguridad de Edición y UX (`AiCopilotPanel.svelte`)**: Interruptor explícito **"Modo Vista Previa vs. Automático"** para revisar diffs de `frontmatter`, `#tags` y `[[wikilinks]]` o aplicarlos de forma directa en nota activa y en lote (`Inbox/`), protegiendo bloques de código, URLs y enlaces previos.
- **Decisiones técnicas de Tarea 20 (Android Share Target para YouTube)**:
  - Se registró `<intent-filter>` para `android.intent.action.SEND` (`text/plain`) en `MainActivity` (`launchMode="singleTask"`).
  - `MainActivity.kt` captura `EXTRA_TEXT` y `EXTRA_SUBJECT` tanto en `onCreate` (cold start) como en `onNewIntent` (warm start), los expone vía `JavascriptInterface` (`RomsidianAndroidShare.consumePendingShare()`) para evitar pérdida de datos antes de montar Svelte y dispara `CustomEvent('romsidian-android-share')` en warm start.
  - `apps/desktop/src/lib/youtubeShare.ts` extrae URL e ID de YouTube (`youtu.be/`, `watch?v=`, `shorts/`, `live/`, `embed/`), sanitiza `<título>.md` o genera `YYYY-MM-DD HH-mm YouTube <id>.md` evitando colisiones, y crea la nota en `settings.youtubeInboxFolder` (`Inbox` por defecto, editable en Ajustes) con YAML frontmatter (`title`, `source: youtube`, `url`, `created`, `tags: [youtube]`), link clickable y sección `## Resumen` lista para pegar al volver desde YouTube.
- **Decisiones técnicas de Tarea 18 (Compilación Android Samsung A34)**:
  - Se configuró target de Rust `aarch64-linux-android` (compatible con el chip ARM64 MediaTek Dimensity 1080 del Samsung Galaxy A34).
  - En `crates/sync-client/Cargo.toml`, se separó `git2` por arquitectura: Windows utiliza `features = ["https", "vendored-libgit2"]` (con Schannel/WinHTTP nativo) mientras que Android utiliza `features = ["vendored-libgit2"]` evitando la dependencia de `openssl-sys` que rompía la compilación cruzada en host Windows.
  - Se modificó `BuildTask.kt` en `apps/desktop/src-tauri/gen/android/buildSrc/` para realizar copias directas de `.so` (`copyTo`) en lugar de `CreateSymbolicLinkW`, permitiendo construir el proyecto en Windows sin privilegios de administrador ni Modo Desarrollador.
  - Se fijó `compileSdk = 36` y `targetSdk = 34` en `app/build.gradle.kts` satisfaciendo los requisitos de `tauri-android`.
  - Se generó el almacén de claves `romsidian-release.keystore` y se firmó el APK final con `apksigner` bajo los esquemas de firma v2 y v3, dejándolo listo para instalación directa vía archivo APK sin rechazo de certificados.
- **Decisiones técnicas de Tarea 15**:
  - Se instaló `@tauri-apps/cli@^2.12.4` localmente en `apps/desktop/package.json` (`"tauri": "tauri"`), evitando instalaciones globales y dejando habilitado `npm --prefix apps/desktop run tauri dev` / `build`.
  - Se actualizó `version = "0.2.0"` en `[workspace.package]` de `Cargo.toml` (propagado automáticamente vía `version.workspace = true` a `romsidian-core`, `romsidian-sync` y `romsidian-desktop`), además de `apps/desktop/package.json` y `apps/desktop/src-tauri/tauri.conf.json`.
  - Se fijó `"targets": ["nsis"]` en `apps/desktop/src-tauri/tauri.conf.json` para generar únicamente el instalador `.exe` NSIS en Windows sin requerir WiX/MSI.
- **Decisiones cerradas en Fase 3 (Iteración v0.3 — CLI, Pruebas en Vivo e Instalador Windows)**:
  1. **CLI de Tauri**: Incorporar `@tauri-apps/cli` en `devDependencies` y `"tauri": "tauri"` en `apps/desktop/package.json` para habilitar `npm --prefix apps/desktop run tauri dev` y `build` con `CARGO_BUILD_JOBS=2`.
  2. **Versión y Empaquetado Windows**: Unificar la versión de la aplicación a `0.2.0` y configurar `"targets": ["nsis"]` en `tauri.conf.json` para generar instalador `.exe` NSIS liviano sin toolchain MSI adicional.
  3. **Alcance v0.3 vs v0.4**: Enfocar v0.3 en pruebas en vivo sobre `test-vault/`, corrección de fallos detectados, validación opcional contra GitHub privado e instalador NSIS; postergar Android e historial local de recuperación a v0.4.
- **Decisiones técnicas de Tarea 14**:
  - Se modularizó la geometría SVG Bézier, parseo/serialización JSON Canvas 1.0, algoritmos de alineación/cuadrícula y renderizado offscreen HiDPI PNG en `canvasUtils.ts`, manteniendo `CanvasEditor.svelte` sin librerías externas dentro del presupuesto de 7,21 GB de RAM.
  - En la exportación PNG (`renderCanvasToPngBlob`), la escala máxima se limita a `2x` / `3840px` y el **Modo Privacidad** reemplaza todo el texto, rutas de notas, URLs y etiquetas de aristas por barras redactadas preservando estructura, colores y conexiones.
  - Se amplió `FileTreeSidebar.svelte` con listado, búsqueda rápida, creación, renombrado y borrado de archivos `.canvas` conectado a `App.svelte` y a los comandos IPC existentes.
- **Decisiones técnicas de Tarea 13**:
  - Simulación física en estado mutable no reactivo (`simNodes`/`simEdges` fuera de `$state` por cuadro) con `requestAnimationFrame` auto-detenido al estabilizarse (`alpha < 0.012`), manteniendo 60 fps sin saturar la reactividad ni la RAM de 7.21 GB.
  - Preservación de coordenadas por `node.id` entre cambios de filtros o refrescos y renderizado HiDPI con `devicePixelRatio` leyendo variables CSS (`--bg-editor`, `--accent`, `--text-primary`, `--text-secondary`) para sincronía inmediata con tema claro/oscuro.
  - Fallback `buildBrowserGraphData()` en `api.ts` para previsualización interactiva fuera de Tauri usando las notas simuladas en memoria.
- **Decisiones técnicas de Tarea 12**:
  - Se encapsuló toda la lógica de retroenlaces en `BacklinksCore.svelte` (agrupación por nota origen, carga asíncrona con guardia de secuencia `mentionsSeq`, resaltado de coincidencias no enlazadas sin inyección HTML cruda y controles uniformes), reutilizándose tanto en el panel derecho / pestaña fijada (`BacklinksPanel.svelte`) como al pie del documento activo (`MarkdownEditor.svelte`, bajo `.markdown-preview` y en cajón colapsable en modo edición).
  - Se amplió `WorkspaceSettings` en `types.ts` y `localStorage` (`romsidian-settings`) con `backlinksLocation` (`'right-panel' | 'document-footer' | 'pinned-tab'`) y `backlinksExcludedFiles` (comodines y prefijos aplicados tanto a menciones enlazadas como al comando IPC `get_unlinked_mentions`).
  - Se agregó soporte de fallback en navegador en `api.ts` para `getUnlinkedMentions` y `linkUnlinkedMention`, permitiendo probar la conversión a `[[wikilink]]` y el refresco reactivo de árbol, editor y lista de backlinks también fuera de Tauri.
- **Decisiones técnicas de Tarea 11**:
  - Se ampliaron [`types.ts`](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/src/lib/types.ts) y [`api.ts`](file:///c:/Users/Roma/Downloads/ROMSIDIAN/apps/desktop/src/lib/api.ts) con los DTOs y comandos IPC v0.2 usando argumentos camelCase (requisito de Tauri 2) e incluyendo fallback completo en navegador para Notas Diarias, Plantillas, Dataview y Transclusiones.
  - Se unificó la hidratación asíncrona de directivas dinámicas (`extractPreviewDirectives` + contador `hydrationSeq` con debounce de 120 ms en `MarkdownEditor.svelte`) antes de pasar el contexto resuelto por marcadores de posición a `marked` + `DOMPurify`, manteniendo clicables todos los `[[wikilinks]]` dentro de tablas/listas/tareas Dataview, campos inline y transclusiones anidadas (máximo 2 niveles).
  - La configuración de carpetas (`daily/`, `templates/`), plantilla diaria predeterminada y el interruptor experimental `DataviewJS` se persisten en `localStorage` (`romsidian-settings`), bloqueando por seguridad cualquier bloque ```` ```dataviewjs ```` salvo activación explícita del usuario.
- **Decisiones técnicas de Tarea 10**:
  - Se diseñó un tokenizador y parser recursivo-descendente propio en Rust puro (`dataview.rs`) sin dependencias nuevas ni ejecución de JavaScript, devolviendo resultados tipados (`DataviewQueryResult` y `DataviewValue`) listos para renderizar tablas, listas y tareas interactivas en Svelte 5.
  - Se unificó la validación de rutas seguras en `Vault::resolve_safe_path_with_ext` para `.md` y `.canvas`, validando que cada `.canvas` contenga un objeto JSON válido (`nodes`/`edges`) y evitando duplicación de lógica de seguridad.
  - Las fechas/horas por defecto de notas diarias y `file.mtime`/`file.mday` se calculan con conversión civil UTC en Rust puro (`format_unix_secs_utc`) sin agregar `chrono`, aceptando `date_str`/`time_str` opcionales desde el frontend para respetar la zona horaria local del usuario.
  - `DesktopCore` expuso en Tauri 2 los comandos IPC v0.2 (`run_dataview_query`, `eval_dataview_inline`, `eval_dataview_inline_batch`, `get_note_fields`, `get_note_headings`, `get_note_blocks`, `resolve_transclusion`, `get_unlinked_mentions`, `link_unlinked_mention`, CRUD `.canvas`, `get_or_create_daily_note`, `list_templates`, `apply_template_content` y `get_graph_data`), verificados de punta a punta con `cargo test --workspace` (`22` en core + `5` en sync + `2` en desktop = `29/29` OK).
- **Decisiones técnicas de Tarea 9**:
  - Se versionó el esquema SQLite en `VaultIndex::init_schema` con `PRAGMA user_version = 2` para que cualquier `.romsidian/index.db` previo (v0.1) reconstruya limpiamente las nuevas tablas (`note_fields`, `note_tasks`, `note_headings`, `note_blocks` y `body_start_line` en `notes`) desde los `.md` en la siguiente sincronización.
  - La tabla `note_fields` unifica propiedades del `frontmatter` (`source = "frontmatter"`, `line = 0`) y campos inline (`source = "bare" | "bracket" | "paren"`) bajo la misma clave normalizada `key_norm`, preparando la base directa para el motor DQL de la Tarea 10.
  - `unlinked_mentions_for` evalúa título, nombre de archivo y `aliases` del `frontmatter` fuera de rangos protegidos (`[[wikilinks]]`, código inline `` `...` ``, enlaces Markdown y `#tags`), respetando comodines y prefijos en `excluded_patterns`.
  - Se flexibilizaron los tests de `test-vault/` (`>= 5` notas en vez de `== 5`) para preservar notas adicionales creadas por el usuario durante sus pruebas en vivo.
- **Decisiones cerradas en Fase 3 (Iteración v0.2 — Manual Obsidian PKM)**:
  1. **Alcance de Dominio**: Implementar exclusivamente los módulos de **Obsidian PKM** del manual y descartar los capítulos de *Obsidian Enterprise 2.2 de Ozona* (monitoreo ITIL/SLA con Nagios/Zabbix en RHEL/openSUSE).
  2. **Cobertura Funcional Completa (v0.2)**: Se incluyen los 5 bloques funcionales del manual: Paleta de Comandos (`Ctrl+P`) + Daily Notes/Plantillas + Transclusiones `![[...]]`, Backlinks completos (*Linked* + *Unlinked Mentions* + *Excluded files* + vista al pie), Vista de Grafo 2D (Global y Local), Lienzo Visual JSON Canvas (`.canvas` + exportación PNG con *Privacy mode*) y Motor de Consultas Dataview.
  3. **Seguridad de Dataview**: Evaluar consultas DQL (`TABLE`, `LIST`, `TASK`) y expresiones inline `` `= this.file.*` `` en un motor declarativo seguro en Rust/SQLite (sandboxed), manteniendo la ejecución de `DataviewJS` arbitrario desactivada por defecto con interruptor opcional en ajustes.
  4. **Presupuesto de RAM (7,21 GB)**: Implementar el Grafo 2D y el editor JSON Canvas (`.canvas`) de forma nativa con Rust + Svelte 5 + HTML5 Canvas/SVG sin instalar librerías pesadas externas en `npm`.
- **Decisiones técnicas de Tarea 8**:
  - `SyncModal.svelte` nunca pre-carga ni persiste el PAT en el frontend ni en `.romsidian/`: el campo `<input type="password">` se limpia tras guardar mediante `saveSyncConfig`, delegando únicamente en Windows Credential Manager (`keyring`), y permite eliminarlo con `clear_token: true`.
  - Tras ejecutar `syncVaultNow(timestampLabel)` con sello temporal local sin caracteres prohibidos en Windows (`YYYY-MM-DD HH-mm`), `App.svelte` refresca inmediatamente el árbol/índice (`refreshSnapshotAndTag`), recarga la nota activa si sigue existiendo y muestra un banner con acceso directo a cada archivo `nota (conflicto ...).md` generado.
  - Se validó la suite completa de Rust con `CARGO_BUILD_JOBS=2` para respetar el presupuesto de memoria (7,21 GB de RAM).
- **Decisiones técnicas de Tarea 7**:
  - Se modularizó la UI bajo `apps/desktop/src/lib/` (`markdown.ts` y `components/`) usando `Compartment` de CodeMirror 6 para conmutar tema claro/oscuro (`oneDark` vs tema claro CSS) y actualizar decoraciones/autocompletado de `[[wikilinks]]` sin recrear el editor ni perder el historial `Ctrl+Z`.
  - Se sanitizó tanto el HTML de vista previa Markdown (`renderMarkdownPreview`) como los fragmentos `<mark>` de FTS5 (`sanitizeSearchSnippet`) mediante `DOMPurify`, permitiendo botones interactivos `[data-wikilink]` delegados con acción Svelte (`delegateWikilinkClicks`) sin advertencias de accesibilidad.
  - Se reemplazaron diálogos nativos del navegador (`prompt`/`confirm`) por modales accesibles propios en `App.svelte` para crear, renombrar/mover y confirmar borrado de notas en WebView2.
- **Decisiones técnicas de Tarea 6**:
  - Se desacopló la lógica de estado en la estructura pura `DesktopCore` dentro de `apps/desktop/src-tauri/src/lib.rs` (envuelta en `Mutex<DesktopCore>` para `tauri::State`), permitiendo probar toda la capa IPC en tests unitarios de Rust sin necesitar levantar una ventana WebView2.
  - Se incluyeron en `apps/desktop/package.json` las dependencias de CodeMirror 6 (`codemirror`, `@codemirror/lang-markdown`, `@codemirror/view`, `@codemirror/state`, `@codemirror/commands`, `@codemirror/autocomplete`, `@codemirror/theme-one-dark`), `marked` y `dompurify` desde esta tarea para ejecutar `npm install` una única vez en todo el proyecto.
  - El cliente `apps/desktop/src/lib/api.ts` detecta automáticamente si corre dentro del runtime nativo de Tauri (`__TAURI_INTERNALS__`) o en un navegador estándar, ofreciendo un fallback en memoria con las notas de `test-vault/` para previsualización rápida.
- **Decisiones técnicas de Tarea 5**:
  - Se configuró `git2 = { version = "0.20", default-features = false, features = ["https", "vendored-libgit2"] }` y `keyring = { version = "3.6", features = ["windows-native"] }` para usar WinHTTP y Windows Credential Manager nativos sin requerir OpenSSL ni `libssh2`, reduciendo el tiempo de compilación y uso de RAM.
  - En `GitSyncBackend::ensure_repository` se fuerza `core.autocrlf = false` en la configuración local del repositorio para preservar exactamente los saltos de línea UTF-8 (`\n`) entre Windows y Android sin conversiones espurias durante `checkout_tree`.
  - Cuando `repo.merge_trees` detecta colisión en las mismas líneas, el motor escribe la versión local (`our`) en `nota (conflicto <dispositivo> <fecha>).md`, resuelve la entrada de `nota.md` en el índice a favor de la versión remota (`their`) limpiando las etapas de conflicto (`flags &= !0x3000`) y genera el commit de fusión sin marcadores `<<<<<<<`.
- **Decisiones técnicas de Tarea 4**:
  - Se utilizó `rusqlite = { version = "0.32", features = ["bundled"] }` y `sha2 = "0.10"`, compilando `libsqlite3-sys` con `CARGO_BUILD_JOBS=2` sin superar el presupuesto de 7,21 GB de RAM.
  - Durante `sync_vault`, las rutas de notas se precargan en memoria (`LinkResolverMap`) para resolver `[[wikilinks]]` en $O(1)$ dentro de una única transacción SQLite y actualizar referencias cruzadas (`refresh_resolved_links_in_tx`) cuando se crean notas nuevas que ya eran enlazadas con anterioridad.
  - La tabla `note_tags` almacena tanto `tag` (con tildes originales) como `tag_folded` (sin diacríticos, conservando la `ñ`), permitiendo que consultas como `#economía` y `#economia` encuentren `#economia/macro` en $O(\log N)$ mediante índice B-Tree.
  - `VaultIndex::open` y `ensure_healthy_connection` detectan automáticamente si `.romsidian/index.db` fue eliminado o corrompido con bytes inválidos, recreando el esquema desde los archivos `.md`.
- **Decisiones técnicas de Tarea 3**:
  - Se implementó `parse_note` con un analizador léxico propio en Rust puro sin expresiones regulares pesadas ni crates adicionales, compilando en 1,14 s con cero consumo extra de RAM.
  - Las etiquetas se normalizan con `normalize_tag` (sin `#` inicial, en minúsculas Unicode y ordenadas en `BTreeSet`) y se provee `ParsedNote::has_tag` insensible a mayúsculas y al prefijo `#`.
  - Se enmascaran los segmentos de código inline (`` `...` ``) y se saltan bloques cercados (` ``` ` y `~~~`) antes de extraer `[[wikilinks]]` y `#etiquetas`, conservando la línea original recortada como `context` para los *backlinks*.
- **Decisiones técnicas de Tarea 2**:
  - Todas las rutas relativas dentro de `Vault` se normalizan con barras `/` (`normalize_slashes`) para garantizar compatibilidad exacta entre Windows y Android/Linux.
  - Se implementó `resolve_safe_note_path` rechazando cualquier componente `ParentDir` (`..`), rutas absolutas o carpetas ocultas (`.romsidian/`) antes de tocar el disco.
  - Se implementó sin dependencias externas adicionales (`std::fs` + `std::path`) logrando compilación en 4,37 s y ejecución de tests en 0,03 s.
- **Decisiones técnicas de Tarea 1**:
  - Dependencias compartidas en `[workspace.dependencies]` (`Cargo.toml` raíz) y bóveda `test-vault/` con 5 notas conectadas en UTF-8.
- **Decisiones cerradas en Fase 3**:
  1. **`FUENTES/UDESA/`**: Intacta y excluida al 100% en `.claude/settings.json`, `.geminiignore`, `.gitignore` y `AGENTS.md`.
  2. **Frontend UI (`apps/desktop`)**: Svelte 5 + Vite + TypeScript + CodeMirror 6 sobre Tauri 2.
  3. **Motor de sincronización (`crates/sync-client`)**: `git2` (`libgit2` embebido) detrás de `trait SyncBackend` + `keyring`.
  4. **Alcance MVP v0.1 vs Android**: Núcleo compartido Rust + app desktop responsiva en Tareas 1–8; APK Android en Release v0.2 con Command-Line Tools + NDK.

## Checklist de Tareas Activas
- [x] Fase 1: Cimientos y Guardrails (`AGENTS.md`, `docs/index.md`, `docs/PROGRESS.md`, `.claude/settings.json`, `.geminiignore`, `.gitignore`).
- [x] Fase 2: Segundo Cerebro (`docs/notas/` destilado en 4 notas temáticas y registrado en `docs/index.md`).
- [x] Fase 3: Entrevista de Diseño y redacción de `docs/SPEC.md`.
- [x] Fase 4 — Tarea 1: Inicialización del Workspace Rust y Bóveda de Prueba (`test-vault/`).
- [x] Fase 4 — Tarea 2: Gestión de Bóveda y Sistema de Archivos en `crates/core` (`vault.rs`).
- [x] Fase 4 — Tarea 3: Parser de Markdown, `[[wikilinks]]`, `#etiquetas` y `frontmatter` en `crates/core` (`parser.rs`).
- [x] Fase 4 — Tarea 4: Índice SQLite FTS5, Backlinks y Benchmark de 1.000 Notas en `crates/core` (`index.rs`).
- [x] Fase 4 — Tarea 5: Motor de Sincronización Git y Resolución de Conflictos en `crates/sync-client`.
- [x] Fase 4 — Tarea 6: Estructura Base de Tauri 2 + Svelte 5 y Comandos IPC (`apps/desktop`).
- [x] Fase 4 — Tarea 7: Interfaz de Usuario Completa con CodeMirror 6, Backlinks y Búsqueda (`apps/desktop/src`).
- [x] Fase 4 — Tarea 8: Integración de UI de Sincronización, Pruebas End-to-End y Cierre del MVP.
- [x] Iteración v0.2 — Fase 2 y Fase 3: Destilación de `docs/notas/manual-funcionalidades-obsidian.md` y especificación de `Tareas 9..14` en `docs/SPEC.md`.
- [x] Fase 4 — Tarea 9: Parser e Índice Extendido en `crates/core` (Campos Inline, Tareas, Bloques y Menciones No Enlazadas).
- [x] Fase 4 — Tarea 10: Motor de Consultas `Dataview` (DQL Sandboxed), Soporte `.canvas` y Comandos IPC v0.2 (`crates/core` y `apps/desktop/src-tauri`).
- [x] Fase 4 — Tarea 11: Paleta de Comandos (`Ctrl+P`), Notas Diarias (`/daily` + `/templates`), Transclusiones (`![[...]]`) y Bloques DQL en UI.
- [x] Fase 4 — Tarea 12: Panel Completo de Retroenlaces (`Linked` + `Unlinked Mentions`), Controles y Vista al Pie del Documento.
- [x] Fase 4 — Tarea 13: Vista de Grafo 2D Interactiva (Global y Local) (`GraphView.svelte`).
- [x] Fase 4 — Tarea 14: Lienzo Visual JSON Canvas (`.canvas`), Exportación PNG con Modo Privacidad y Cierre v0.2 (`CanvasEditor.svelte`).
- [x] Iteración v0.3 — Fase 3: Entrevista de diseño y especificación de `Tareas 15..17` en `docs/SPEC.md`.
- [x] Fase 4 — Tarea 15: Integración de `@tauri-apps/cli`, Script `tauri` y Versión `0.2.0`.
- [x] Fase 4 — Tarea 16: Prueba Funcional en Vivo (`tauri dev`) sobre `test-vault/`, Bugfixes y Sync GitHub Real.
- [x] Fase 4 — Tarea 17: Empaquetado Windows NSIS (`.exe`) y Cierre v0.3.
- [x] Fase 4 — Tarea 18: Compilación Cruzada y Empaquetado APK Android Nativo Firmado para Samsung Galaxy A34 (`arm64-v8a`).
- [x] Fase 4 — Tarea 19: Bóveda Automática Android, Sync HTTPS GitHub, `custom-protocol` y Fixes UX Móvil (`App.svelte`, `GraphView.svelte`).
- [x] Fase 4 — Tarea 20: Android Share Target para YouTube (`Inbox/` + Frontmatter + Edición Continua) y verificación de `ROMSIDIAN-Samsung-A34.apk` firmado.
- [x] Iteración v0.5 — Fase 2 y Fase 3: Destilación de `docs/notas/agente-ia-y-mcp.md` y especificación de `[RF-15..18]` + `Tareas 21..24` en `docs/SPEC.md`.
- [x] Fase 4 — Tarea 21: Motor de Mapa de Conocimiento, Sugerencia de Conexiones y Auto-Inyección (`crates/core/src/ai_linker.rs`).
- [x] Fase 4 — Tarea 22: Servidor MCP Nativo (`crates/mcp-server` / `romsidian-mcp`) para Claude Desktop, Claude Code y Antigravity.
- [x] Fase 4 — Tarea 23: Puente de Proveedores IA en Backend (`LocalCliProvider`, `GeminiHttpProvider`, `DeterministicRustProvider`) y Comandos IPC v0.5.
- [x] Fase 4 — Tarea 24: Panel Copiloto PKM en UI (`AiCopilotPanel.svelte`), Auto-Conexión en Lote (Vista Previa / Automático) y Cierre v0.5.
- [x] Iteración v0.6 — Tarea 25: Motor Web Multi-Bóveda en `sessionStorage` y Sincronización Real con GitHub REST API (`webGithubSync.ts` + `api.ts`).
- [x] Iteración v0.6 — Tarea 26: UX Web (Conector GitHub 1 clic + Cerrar Sesión) y Despliegue en GitHub Pages ([roma8493/romsidian-app](https://github.com/roma8493/romsidian-app)).
