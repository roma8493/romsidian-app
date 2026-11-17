# Sincronización y Seguridad (`crates/sync-client`)

## 1. Abstracción `SyncBackend`
- El núcleo y la interfaz nunca llaman a Git directamente; consumen un trait Rust `SyncBackend` con operaciones atómicas (`status`, `pull`, `commit_and_push`, `resolve_conflict`).
- Permite usar **Git + GitHub privado** en el MVP y cambiar o agregar un backend propio (VPS Hostinger KVM 1) en una fase futura sin reescribir la app.

## 2. Estrategia Git en el MVP
- **Flujo de sincronización**:
  1. Guardar cambios locales en disco.
  2. `commit` local automático o manual con mensaje estructurado (`sync(device): YYYY-MM-DD HH:MM`).
  3. `fetch` + `merge` de 3 vías (`diff3`) contra `origin/main`.
  4. `push` al repositorio privado de GitHub.
- **Política de Cero Pérdida de Datos en Conflictos**:
  - Si la fusión de 3 vías resuelve cambios en líneas distintas del mismo archivo, se fusiona automáticamente.
  - Si hay colisión en las mismas líneas o borrado vs. edición, **nunca se sobrescribe en silencio**: se conserva la versión remota en `nota.md` y se guarda la versión local como `nota (conflicto <dispositivo> <fecha>).md`, avisando en la UI.

## 3. Por qué NO se usa CRDT (Yjs/Automerge) en el MVP
- Un CRDT requiere almacenar historial binario adicional o cambiar el formato plano `.md`, complicando la interoperabilidad con Git y otras herramientas.
- Solo se reevaluará si en el uso real ocurre más de 1 conflicto cada 100 sincronizaciones o si se requiere edición colaborativa simultánea en vivo.

## 4. Seguridad y Gestión de Credenciales
- **Sin E2EE en el MVP**: las notas viajan por HTTPS/TLS y residen en un repositorio privado de GitHub exclusivo del usuario.
- **Almacenamiento del Token (PAT Fine-Grained)**:
  - Restringido a **un único repositorio privado** con permiso `Contents: Read and write`.
  - Guardado exclusivamente en el almacén de credenciales del sistema operativo (Windows Credential Manager via crate `keyring` / Android Keystore).
  - **Prohibido** guardar tokens en `.romsidian/`, variables en texto plano o dentro de la bóveda.
- **Protecciones operativas**:
  - `.romsidian/` siempre en `.gitignore` de la bóveda.
  - Bloqueo preventivo antes de hacer `commit` de archivos mayores a 50–100 MB (límite de GitHub).
