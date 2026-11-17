# Funcionalidades Deseadas del Manual de Obsidian (Iteración v0.2)

> Destilación técnica de `docs/MANUAL CON FUNCIONES DESEADAS.txt` (441 líneas) para la evolución de ROMSIDIAN hacia paridad funcional PKM.

## 1. Desambiguación Crítica del Documento Fuente
El archivo `docs/MANUAL CON FUNCIONES DESEADAS.txt` combina fuentes de dos productos homónimos llamados *"Obsidian"*:
1. **Obsidian PKM (Capítulos 1.1, 1.2, 3.1–3.4, 4.3–4.4, 5.1–5.2)**: Ecosistema de gestión personal del conocimiento sobre archivos locales `.md` y `.canvas`. **Es el objetivo real de ROMSIDIAN**.
2. **Obsidian Enterprise 2.2 de Ozona Consulting (Capítulos 1.3, 2.1–2.3, 4.1–4.2, 5.3)**: Appliance corporativo Linux (openSUSE/RHEL 7.x) de monitoreo ITIL/SLA con sondas Nagios/Zabbix/Prometheus y PinSAFE. *(Queda descartado por pertenecer a otro software homónimo salvo que se requiera algún tablero de métricas específico).*

---

## 2. Módulos PKM Extraídos del Manual y Brecha frente a ROMSIDIAN v0.1

### A. Paleta de Comandos (`Ctrl+P`), Estructura de Bóveda y Notas Diarias (`Daily Notes`)
- **Estructura recomendada**: `/` (raíz), `/daily` (notas diarias), `/templates` (plantillas), `/attachments` (adjuntos multimedia) y patrones de exclusión (`Excluded files`).
- **Paleta de Comandos (`Ctrl+P` / `Cmd+P`)**: ejecutor centralizado de acciones del núcleo (`Daily Notes: Abrir/crear nota de hoy`, `Canvas: Crear nuevo lienzo`, `Backlinks: Alternar backlinks en documento`, `Grafo: Abrir vista global/local`, etc.).
- **Notas Diarias + Plantillas**: creación explícita (nunca automática por cambio de hora del SO) en `/daily/YYYY-MM-DD.md` aplicando una plantilla de `/templates` con variables (`{{date}}`, `{{time}}`, `{{title}}`).

### B. Retroenlaces Avanzados (`Backlinks`) y Enlaces a Encabezados/Bloques
- **Linked Mentions vs Unlinked Mentions**:
  - *Menciones enlazadas*: referencias explícitas `[[Nota]]`, `[[Nota#Encabezado]]` o `[[Nota#^bloque]]` (ya base en v0.1, falta navegación fina a `#Encabezado` y `#^bloque`).
  - *Menciones no enlazadas*: escaneo FTS5/texto de apariciones literales del título de la nota activa en otras notas sin corchetes `[[]]`, con botón **"Enlazar"** para convertir la mención en `[[Nota]]`, respetando `Excluded files`.
- **Controles y ubicaciones del panel**:
  - Controles de mostrar/colapsar contexto, ordenamiento (nombre/fecha) y filtro de búsqueda dentro de los backlinks.
  - 3 ubicaciones: pestaña en panel lateral derecho, vista incrustada al final del documento activo (`Toggle backlinks in document`) y pestaña fijada.

### C. Propiedades Tipadas, Campos Inline y Motor de Consultas `Dataview` (DQL)
- **YAML Frontmatter tipado**: soporte de escalares, listas, números, booleanos (`true`/`false`), fechas (`YYYY-MM-DD`) y objetos anidados, más metadatos implícitos (`file.name`, `file.path`, `file.mtime`, `file.tags`, `file.tasks`).
- **Campos Inline en el cuerpo Markdown**:
  - `Clave:: Valor` (en línea completa o tras formato).
  - `[clave:: valor]` (clave visible entre corchetes).
  - `(clave:: valor)` (clave oculta en vista previa).
- **Motor DQL en bloques ```` ```dataview ```` y expresiones inline `` `= this.file.name` ``**:
  - Consultas declarativas seguras (sandboxed sobre SQLite + índice en Rust):
    - `TABLE campo1, campo2 AS "Alias" FROM #tag OR "carpeta" WHERE ... SORT campo ASC|DESC`
    - `LIST FROM #tag`
    - `TASK FROM #projects/active` (extracción e interacción con tareas `- [ ]` / `- [x]`).

### D. Lienzo Visual (`Canvas` — Formato Abierto `.canvas` / JSON Canvas)
- **Estándar**: archivos `.canvas` compatibles con la especificación abierta JSON Canvas (`nodes` y `edges`).
- **Tipos de tarjetas**:
  1. **Texto**: Markdown libre con opción `Convertir a archivo .md` (mientras sea tarjeta de texto no genera backlinks).
  2. **Nota de la bóveda**: incrusta una nota `.md` completa o acotada a una sección (`Narrow to heading` / `Narrow to block`).
  3. **Multimedia / Web**: imágenes (`PNG`/`JPG`), `PDF`, audio o URL externa interactiva.
  4. **Grupos**: contenedores visuales arrastrables en bloque con color o imagen de fondo.
- **Conexiones y maquetación**:
  - Arrastrar flechas entre bordes con etiqueta editable y direccionalidad (`Nondirectional`, `Unidirectional`, `Bidirectional`) y comando `Follow connection`.
  - Herramientas de alineación (`Align left/center/right`, `top/middle/bottom`, `Distribute`, `Arrange in grid`) y exportación a imagen PNG (incluyendo `Privacy mode` que difumina el texto).

### E. Vista de Grafo (`Graph View` Global y Local) y Personalización (`CSS Snippets`)
- **Grafo Global y Local 2D**: visualización interactiva en HTML5 Canvas/SVG de los nodos (`.md`) y aristas (`links`), con filtro por etiquetas/carpetas, coloreado por grupo y físicas de repulsión/enlace.
- **Incrustaciones (`![[nota]]`, `![[imagen.png]]`) y Fragmentos CSS (`CSS Snippets`)**: transclusión de notas/medios en la vista previa y carga de hojas `.css` personalizadas desde `.romsidian/snippets/`.
