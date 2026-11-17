# ROMSIDIAN (v0.2.0)

Aplicación de toma de notas en Markdown y Lienzos Visuales **local-first**, inspirada en Obsidian, donde tu bóveda es una carpeta común de archivos `.md` y `.canvas` sin bases de datos propietarias ni bloqueo de proveedor.

## Ejecución con Doble Clic (Sin Terminal)

Tienes dos formas inmediatas de abrir ROMSIDIAN sin tocar la terminal:

1. **Acceso directo en el Escritorio**: Ya está creado el icono **`ROMSIDIAN`** en tu Escritorio de Windows y en el Menú Inicio. Haz doble clic sobre él para abrir la aplicación directamente.
2. **Instalador oficial de Windows**: Ubicado en la raíz del proyecto como `ROMSIDIAN_0.2.0_x64-setup.exe`. Puedes hacer doble clic para instalarlo o reinstalarlo en cualquier momento.
3. **Lanzador rápido portátil**: Archivo `Iniciar ROMSIDIAN.cmd` en la raíz de la carpeta del proyecto.

## Características de ROMSIDIAN v0.2.0

- **Bóveda abierta y estándar**: carpeta local con archivos `.md` y `.canvas` en UTF-8 y subcarpeta `.romsidian/` (excluida automáticamente por `.gitignore`) para configuración de sincronización e índice SQLite.
- **Explorador y Drag & Drop**:
  - Arrastra notas y suéltalas sobre cualquier carpeta o subcarpeta.
  - Arrastra carpetas completas sobre otras para convertirlas en **subcarpetas**.
  - Arrastra a la cabecera `ARCHIVOS` para devolver cualquier archivo o subcarpeta a la raíz.
  - Botón `📁 Explorar...` para elegir carpetas con el explorador nativo de Windows.
  - Botón `+ Carpeta` para crear nuevas carpetas.
- **Lienzo Visual JSON Canvas (`.canvas`)**:
  - Compatible con el estándar abierto JSON Canvas 1.0.
  - Tarjetas de Texto (con conversión a nota `.md`), Notas (con acotación a encabezados y bloques), Multimedia (imágenes/archivos locales), Páginas Web y Grupos.
  - Conectores direccionales, cuadrícula magnética (*Snap 20px*), alineación y distribución automática.
  - Exportación en alta resolución a PNG con **Modo Privacidad** integrado.
- **Vista de Grafo 2D Interactiva (Global y Local)**:
  - Físicas de fuerzas en tiempo real, agrupamiento por etiquetas y carpetas, zoom y paneo interactivo.
- **Motor de Consultas Dataview (DQL Sandboxed)**:
  - Soporte para bloques ```` ```dataview ```` (`TABLE`, `LIST`, `TASK`) y expresiones inline `` `= this.file.name` ``.
- **Paleta de Comandos (`Ctrl+P`)**: Acceso instantáneo a todas las funciones y comandos del sistema.
- **Notas Diarias y Plantillas (`Alt+D` / `Alt+T`)**: Creación rápida de bitácoras y aplicación de plantillas dinámicas con `{{date}}`, `{{time}}` y `{{title}}`.
- **Generación automática de Frontmatter**: Opción de autocompletar cabeceras YAML (`title`, `created`, `tags`) al crear notas (`+ Nota`) o mediante `Alt+M`.
- **Transclusiones (`![[...]]`)**: Incrustación de notas completas, secciones y bloques.
- **Backlinks completos**: Menciones enlazadas y no enlazadas (*Unlinked Mentions*) con botón de enlace directo y vista al pie del documento.
- **Búsqueda instantánea FTS5 (`Ctrl+K`)**: Búsqueda ultrarrápida indexada en SQLite FTS5.
- **Sincronización Git segura (`git2` + llavero del SO)**:
  - Token PAT almacenado de forma segura en **Windows Credential Manager** (`keyring`).
  - Resolución de conflictos con cero pérdida de datos creando copias de conflicto automáticas.

## Estructura del Repositorio

```text
ROMSIDIAN/
├── Cargo.toml                 # Workspace Rust compartido
├── crates/
│   ├── core/                  # romsidian-core: Vault, Parser Markdown, SQLite FTS5
│   └── sync-client/           # romsidian-sync: trait SyncBackend, GitSyncBackend, Keyring
├── apps/
│   └── desktop/               # Frontend Svelte 5 + Vite + CodeMirror 6 y backend Tauri 2 (src-tauri/)
├── test-vault/                # Bóveda de prueba inicial con 5 notas conectadas
└── docs/                      # Especificación (SPEC.md), bitácora (PROGRESS.md) y notas de arquitectura
```

## Comandos de Desarrollo y Verificación

> En Windows con memoria limitada (~7,2 GB), se recomienda limitar los hilos de compilación de Cargo con `$env:CARGO_BUILD_JOBS=2`.

### 1. Pruebas del Workspace Rust (`core`, `sync-client` y capa IPC de `desktop`)

```powershell
$env:CARGO_BUILD_JOBS=2
cargo check --workspace
cargo test --workspace
```

### 2. Compilación y verificación del Frontend Svelte 5

```powershell
./apps/desktop/node_modules/.bin/tsc -p apps/desktop/tsconfig.json --noEmit
npm --prefix apps/desktop run build
```

### 3. Ejecutar la aplicación

- **Vista rápida web (modo desarrollo Vite con bóveda simulada)**:
  ```powershell
  npm --prefix apps/desktop run dev
  ```
- **Aplicación de escritorio Tauri 2 nativa** (inicia el frontend Vite y la ventana desktop Rust):
  1. En una terminal, levantar `npm --prefix apps/desktop run dev`.
  2. En otra terminal, ejecutar:
     ```powershell
     $env:CARGO_BUILD_JOBS=2
     cargo run --manifest-path apps/desktop/src-tauri/Cargo.toml
     ```
  *(Si tienes `tauri-cli` instalado globalmente, también puedes usar `cargo tauri dev` dentro de `apps/desktop/src-tauri`.)*

## Atajos de Teclado y Uso de Sincronización

- `Ctrl+K` / `Cmd+K`: abrir búsqueda rápida FTS5.
- `Ctrl+N` / `Cmd+N`: crear nueva nota Markdown.
- `Ctrl+S` / `Cmd+S`: guardar la nota activa inmediatamente.
- `Ctrl+Clic` / `Cmd+Clic` sobre `[[wikilink]]`: navegar a la nota o crearla si está pendiente.
- **Configurar GitHub Sync**: pulsa `⟳ Sincronizar` en la barra superior, ingresa la URL HTTPS del repositorio privado, la rama (`main`), el nombre de tu dispositivo (`pc-windows`), tu autor/correo Git y tu token PAT. Guarda la configuración y presiona **Sincronizar ahora**.

## Fuera de Alcance en MVP v0.1 (Planeado para Release v0.2)

- Compilación y empaquetado APK Android (requiere Android SDK Command-Line Tools + NDK).
- Grafo visual 2D de notas, plugins de terceros, canvas infinito y cifrado E2EE propio en VPS.
