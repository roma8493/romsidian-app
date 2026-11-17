# Estrategia Móvil en Android

## 1. Objetivo del MVP Móvil
- Permitir abrir la misma bóveda de notas en un teléfono Android, leer, editar notas `.md`, navegar `[[wikilinks]]` y sincronizar contra el mismo repositorio privado de GitHub que usa la PC.

## 2. Riesgos Técnicos Detectados en Android
1. **Acceso al sistema de archivos (Android Scoped Storage / SAF)**:
   - En Android 11+, el acceso a carpetas compartidas fuera del sandbox de la app requiere permisos especiales o Storage Access Framework (SAF).
   - Experiencias previas en la comunidad (*SwarmNote* con Tauri 2 móvil) reportaron fricciones al operar rutas POSIX directas sobre carpetas externas de Android.
   - **Mitigación para el MVP**: almacenar la bóveda sincronizada por Git dentro del directorio de datos de la aplicación (`app_data_dir`) en Android —donde Rust y SQLite tienen acceso POSIX completo y rápido— con opción de importar/exportar carpeta externa.
2. **Cliente Git en Android**:
   - En Android no existe el binario CLI `git` instalado en el sistema.
   - Opciones técnicas para `crates/sync-client`:
     - **Opción A (Nativa Rust)**: `git2` (`libgit2` con transporte HTTPS vía `rustls`/`vendored-openssl`) o `gix` (`gitoxide`), compilado tanto para Windows como para Android (`aarch64-linux-android`).
     - **Opción B (API/Web)**: `isomorphic-git` en el WebView o sincronización directa por Árboles Git de la API REST/GraphQL de GitHub (evita compilar C/OpenSSL para Android y descarga solo blobs modificados).

## 3. Validación Temprana (Etapa 5 — Prototipo de Riesgo)
- Antes de construir toda la interfaz táctil móvil (Etapa 6), se ejecuta un prototipo mínimo en Android que valide en el teléfono físico:
  1. Compilación cruzada de `crates/core` y `crates/sync-client`.
  2. Lectura/escritura de 1 nota con tildes y actualización del índice SQLite.
  3. Pull y push real contra el repositorio privado de GitHub.
