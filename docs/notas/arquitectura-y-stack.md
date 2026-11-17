# Arquitectura Base y Stack Tecnológico

## 1. Formato de Bóveda
- **Principio rector**: una bóveda es una carpeta común del sistema de archivos con notas `.md` en texto plano (UTF-8), sin base de datos propietaria obligatoria.
- **Metadatos locales**: la subcarpeta `.romsidian/` almacena ajustes locales de interfaz, caché e índice SQLite FTS5. Siempre se excluye de Git (`.gitignore`).

## 2. Restricciones de Hardware y Elección de Escritorio
- **Equipo de desarrollo**: AMD Ryzen 3 7320U con **7,21 GB de RAM utilizables** en Windows.
- **Herramientas instaladas verificadas**: `git 2.56.1`, `node v24.13.1`, `npm 11.8.0`, `rustc 1.98.1`, `cargo 1.98.1` (`pnpm` se activa vía `corepack` o `npm`).
- **Shell de escritorio**: **Tauri 2** (Rust + WebView nativo). Se descarta Electron por consumo de RAM.
- **Editor Markdown**: **CodeMirror 6** (modular, soporte nativo de extensiones para `[[wikilinks]]`, etiquetas `#tag`, frontmatter YAML y vista previa híbrida/dividida).
- **Framework UI**: **Svelte 5** o **React** (Svelte 5 compila a JS mínimo sin Virtual DOM y consume menos memoria; se cierra en la especificación).

## 3. Estructura Modular del Monorepo
```text
romsidian/
├── crates/
│   ├── core/          # Lectura de bóveda, parser Markdown/wikilinks/tags/frontmatter, índice SQLite FTS5
│   └── sync-client/   # Trait SyncBackend, implementación Git y resolución de conflictos
├── apps/
│   └── desktop/       # App Tauri 2 (escritorio Windows + soporte Android)
└── docs/              # Arnés de ingeniería de contexto (SPEC, PROGRESS, index, notas)
```

## 4. Responsabilidades de `crates/core` (Rust)
- Escaneo incremental de archivos `.md` y detección de cambios por hash/timestamp.
- Extracción de `[[wikilinks]]` (incluyendo alias `[[nota|texto]]`), `#etiquetas` y bloque YAML (`frontmatter`).
- Construcción del grafo de enlaces entrantes (**backlinks**) y salientes.
- Índice de búsqueda de texto completo con **SQLite FTS5** (soportando tildes, eñes y rutas con espacios).

## 5. Referencias Open-Source Analizadas
- Proyectos similares en Tauri 2 + CodeMirror 6: `doc-md`, `Opensidian`, `Vaultcore`.
