# Índice de Conocimiento del Proyecto (formato llms.txt)

> Mapa ligero del proyecto. Leer este índice primero y abrir únicamente el archivo necesario para la tarea actual.

## Estado y Especificaciones
- [Bitácora de Progreso](PROGRESS.md): estado entre sesiones, decisiones tomadas y próxima tarea exacta.
- [Especificación Activa](SPEC.md): requerimientos `[RF-01..18]`, arquitectura, criterios de aceptación y checklist atómica (Tareas 1 a 24).

## Notas del Segundo Cerebro (`docs/notas/`)
- [Arquitectura Base y Stack](notas/arquitectura-y-stack.md): estructura de bóveda `.md`, Tauri 2 + Svelte 5 + CodeMirror 6, `crates/core` en Rust con SQLite FTS5 y restricciones de RAM (7,21 GB).
- [Sincronización y Seguridad](notas/sincronizacion-y-seguridad.md): trait `SyncBackend`, flujo Git (`git2`) contra GitHub privado, fusión 3 vías con copia en conflicto y resguardo de tokens en el SO.
- [Estrategia Móvil en Android](notas/estrategia-movil-android.md): riesgos de sistema de archivos en Android, opciones de cliente Git móvil y prototipo de validación temprana.
- [Plan de Etapas y Matriz de Riesgos](notas/plan-etapas-y-riesgos.md): hoja de ruta de las Etapas 0 a 7 con criterios de aceptación y mitigación de riesgos críticos.
- [Funcionalidades del Manual de Obsidian (v0.2)](notas/manual-funcionalidades-obsidian.md): destilación de `docs/MANUAL CON FUNCIONES DESEADAS.txt` (Canvas `.canvas`, Grafo 2D, Dataview DQL, Unlinked Mentions, Daily Notes y desambiguación frente a Ozona Enterprise 2.2).
- [Agente IA, Auto-Conexión y Servidor MCP (v0.5)](notas/agente-ia-y-mcp.md): arquitectura híbrida Rust + IA (puente CLI Claude/Gemini, Gemini Free Tier móvil y servidor MCP stdio) para auto-enlazar `[[wikilinks]]`, `#tags` y `frontmatter`.

## Referencias Externas
- `README.txt` y `docs/MANUAL CON FUNCIONES DESEADAS.txt`: documentos técnicos de origen (ya destilados en `docs/notas/`; no releer completos).
