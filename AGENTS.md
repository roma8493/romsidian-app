# Reglas y Mapa del Proyecto

> Mantener este archivo por debajo de 100 líneas. Si crece, mover detalle a `docs/notas/`.

## 1. Objetivo Core
- ROMSIDIAN: app de notas en Markdown cuya bóveda es una carpeta de archivos `.md` y `.canvas` comunes (formato abierto, sin base de datos propietaria).
- Funciones MVP v0.1 (completadas): abrir carpeta como bóveda, árbol de archivos, editor CodeMirror 6 con vista previa, `[[wikilinks]]`, backlinks, etiquetas, frontmatter, búsqueda FTS5, tema claro/oscuro y sincronización Git (`SyncBackend` + `git2` + `keyring`).
- Funciones Iteración v0.2 (Manual Obsidian PKM): Paleta de Comandos (`Ctrl+P`), Notas Diarias (`/daily`) + Plantillas (`/templates`), Transclusiones (`![[...]]`), Backlinks con *Unlinked Mentions* e incrustación al pie, Motor Dataview DQL (`TABLE`/`LIST`/`TASK` y campos `Clave:: Valor`), Vista de Grafo 2D (Global y Local) y Lienzo JSON Canvas (`.canvas` con exportación PNG y modo privacidad).
- Funciones v0.4 y v0.5: APK Android nativo + Share Target YouTube (`v0.4`); Agente IA / Copiloto PKM, Auto-Conexión de `[[wikilinks]]`/`#tags`/`frontmatter` y Servidor MCP `romsidian-mcp` (`v0.5`).
- Sincronización entre PC (Windows) y celular (Android) mediante Git contra un repositorio privado de GitHub.

## 2. Reglas de Contexto y Ahorro de Tokens (CRÍTICO)
- NUNCA leer carpetas completas ni todos los archivos de `docs/` de golpe.
- Al iniciar sesión, leer únicamente `docs/PROGRESS.md` y `docs/index.md`.
- Abrir un archivo de `docs/notas/` solo cuando la tarea activa lo requiera explícitamente.
- PROHIBIDO leer o escanear `FUENTES/`, `node_modules/`, `target/`, `.git/`, `dist/`, `build/`, `*.lock`, `.env*`, PDFs, Office (`*.xlsx`, `*.docx`, `*.pptx`) o ejecutables (`*.exe`).
- Antes de escribir código o un entregable largo, verificar el criterio de aceptación en `docs/SPEC.md`.
- Si hace falta explorar más de 3 archivos, delegar en un subagente `research` en vez de leer en el hilo principal.
- Cerrar sesión al superar el 60% de contexto: persistir en `docs/PROGRESS.md` y abrir sesión nueva.

## 3. Entorno y Comandos de Verificación
- Herramientas verificadas en PC: `git 2.56.1`, `node v24.13.1`, `npm 11.8.0`, `rustc 1.98.1`, `cargo 1.98.1`.
- Núcleo Rust (`crates/core` y `crates/sync-client`): `cargo test` (se activa en Fase 4).
- App de escritorio (`apps/desktop`): `npm run build` / `pnpm build` (se activa en Fase 4).
- Arnés documental: verificar que `AGENTS.md` siga bajo 100 líneas y que `docs/index.md` liste toda nota de `docs/notas/`.

## 4. Mapa de Documentación (leer bajo demanda)
- Bitácora de estado y próxima tarea: `docs/PROGRESS.md`
- Especificación activa (PRD/Spec): `docs/SPEC.md`
- Índice del Segundo Cerebro: `docs/index.md`
- Notas destiladas por tema: `docs/notas/`

## 5. Convenciones
- Idioma: español para documentación, decisiones y mensajes de commit; inglés para identificadores de código.
- Arquitectura de referencia: bóveda = carpeta de `.md` + `.romsidian/` para ajustes e índice local (ignorada por Git).
- Nunca guardar tokens ni credenciales dentro de la bóveda ni en el repositorio (usar almacén del SO).
- Ejecutar una sola tarea de `docs/SPEC.md` por sesión limpia; nada de código de producción antes de aprobar `docs/SPEC.md` (Fase 3 -> Fase 4).
