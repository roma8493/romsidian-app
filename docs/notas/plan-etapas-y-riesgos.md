# Plan de Etapas y Matriz de Riesgos

## 1. Hoja de Ruta por Etapas
| Etapa | Nombre | Entregable Principal | Criterio de Aceptación (DoD) |
|---|---|---|---|
| **0** | Medición y cierre de stack | Elección de UI (Svelte 5 vs React) y estrategia de cliente Git | Compilación base verificada dentro de los 7,21 GB de RAM |
| **1** | Repo y arnés | Estructura del monorepo (`crates/`, `apps/`), exclusiones y scripts | `cargo check` y verificación de arnés (<100 líneas en `AGENTS.md`) |
| **2** | Núcleo (`crates/core`) | Parser de `.md`, `[[wikilinks]]`, backlinks, `#tags`, frontmatter y búsqueda FTS5 | `cargo test` pasando con corpus sintético de 1.000 notas con tildes y espacios |
| **3** | Escritorio (`apps/desktop`) | App Tauri 2 + CodeMirror 6: árbol de archivos, editor, vista previa, backlinks, búsqueda y tema claro/oscuro | Abrir bóveda de prueba, editar, navegar enlaces y buscar sin corromper archivos |
| **4** | Sincronización (`crates/sync-client`) | Trait `SyncBackend`, backend Git/GitHub, token en Credential Manager y copias en conflicto | Superar tests de edición concurrente en 2 clones, conflicto en misma línea, borrado vs edición y corte de red |
| **5** | Prototipo móvil Android | App mínima Android conectada a `crates/core` y `crates/sync-client` | Leer, editar y sincronizar 1 nota en el celular real contra GitHub |
| **6** | App móvil Android | UI adaptada a pantalla táctil (navegación, editor y botón de sync) | Uso fluido entre PC y Android sobre bóveda de prueba |
| **7** | Robustez e instalador | Historial local de recuperación, manejo de errores y empaquetado `.msi`/`.nsis` y `.apk` | Instaladores funcionales listos para uso diario |

## 2. Matriz de Riesgos Principales y Mitigaciones
- **Pérdida o corrupción de notas durante sincronización**:
  - *Mitigación*: Prohibido usar `git reset --hard` o sobrescritura silenciosa; ante cualquier divergencia no integrable automáticamente, generar archivo `nota (conflicto ...).md` y preservar ambos textos.
- **Agotamiento de RAM (7,21 GB útiles) al compilar Rust + Tauri**:
  - *Mitigación*: Limitar jobs paralelos de Cargo (`CARGO_BUILD_JOBS=2` si hay presión de memoria), usar UI ligera (Svelte 5 recomendado) y evitar dependencias nativas pesadas innecesarias.
- **Fugas de credenciales o subida accidental de configuración local**:
  - *Mitigación*: Token guardado exclusivamente en el almacén del SO (`keyring`); `.romsidian/` ignorado automáticamente al inicializar cualquier bóveda.
- **Exceso de consumo de contexto del agente IA**:
  - *Mitigación*: Prohibición de lectura en `FUENTES/`, `target/` y `node_modules/`; ejecución de 1 sola tarea del `SPEC.md` por sesión con cierre en `PROGRESS.md`.
