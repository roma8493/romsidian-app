# ROMSIDIAN: plan técnico de una app de notas tipo Obsidian con sincronización entre dispositivos

## Contexto y conclusión

Objetivo: una app de notas en Markdown, con enlaces `[[wikilink]]`, backlinks, búsqueda y grafo, cuyas bóvedas se sincronicen entre PC y celular. El plan sigue la forma del [[Transcriptor local - Plan técnico]]: el desarrollo es por etapas pequeñas, con evidencia y aprobación tuya en cada una, y trabajan agentes de [[Claude Code]] dentro de un arnés ([[Harness Engineering]]).

**Recomendación en una línea:** la bóveda es una carpeta de archivos `.md` comunes (formato abierto, como Obsidian). Primero se construye una app de escritorio Tauri 2 con CodeMirror 6, y después un servicio de sincronización por archivo con fusión de tres vías y cifrado extremo a extremo. El móvil queda como etapa propia, condicionada por un prototipo de riesgo.

Cómo leer este documento: **Parte A** es lo que dicen tus fuentes, **B** lo que busqué afuera, **C** mis recomendaciones, **D** riesgos.

---

## Parte A. Información respaldada por la bóveda

La bóveda no tiene ninguna fuente sobre editores Markdown, sincronización, CRDT, Tauri ni apps móviles. Lo que aporta es **cómo construir con agentes**, no **qué construir**. Notas leídas: [[Harness Engineering]], [[Archivos de instrucciones para agentes]], [[Orquestación multiagente]], [[Code Review]], [[Gestión de contexto]], [[Claude]], el [[Transcriptor local - Plan técnico]] y `AGENTS.md`.

| Fuente (nota) | Qué dice | Cómo lo uso |
|---|---|---|
| [[¿Qué es esto del Harness Engineering]] (BT 06:53) | Pedir una app completa sin arnés da resultados muy malos | Desarrollo por etapas chicas |
| BT 02:51–05:36 | Un arnés más simple rinde mejor | Pocos agentes, sin MCP al inicio |
| BT 09:05–10:55 | El agente debe demostrar que terminó | Cada tarea exige evidencia |
| BT 14:18–17:07 | `AGENTS.md`, `init`, `feature_list.json`, mapa del repo | Estructura del repo |
| BT 18:42–19:25 | `progress/` para no perder información entre agentes | Memoria del proyecto |
| [[ANTHROPIC revela cómo resuelve el MAYOR PROBLEMA de programar con IA]] (XCB 01:03, 04:22, 05:39, 06:16) | Revisor con contexto limpio, máximo 3 rondas, la persona decide producto y arquitectura | Ciclo por tarea |
| [[Gestión de contexto]] | La calidad cae antes de llenar la ventana (dato citado, no verificado) | Una sesión por etapa |
| [[Orquestación multiagente]] | Líder, implementer, explorer, reviewer | Roles en `.claude/agents/` |
| [[Tu primer agente de IA con Claude Code sin programar (Tristen O'Brien)]] | `CLAUDE.md`, Plan Mode, humano en el circuito | Flujo de trabajo |
| [[Optimización de modelos de Anthropic - Fable, Opus y Sonnet]] (vía [[Claude]]) | Fable para estrategia, Opus para ejecutar, Sonnet para lo trivial | Reparto de modelos por tarea |
| [[Transcriptor local - Plan técnico]] | Tu PC tiene 7,21 GB de RAM utilizables, con Python, Git, Node y Cargo instalados | Descarta Electron; Rust y Node ya están |

### Límites
- **Ninguna fuente respalda las decisiones técnicas de la Parte C** (Tauri, CodeMirror, sincronización, cifrado). Todas vienen de la Parte B y de mi criterio.
- [[Inteligencia Artificial]], [[LLM]] y [[Mapa general]] están vacías. No usé [[Electrónica]] ni [[ESP32]].
- Las cifras de BT y XCB son comunicadas por terceros y no están verificadas.
- No sé si tenés un celular Android o iPhone, ni si querés alojar tu propio servidor. Lo dejo como decisión abierta.

---

## Parte B. Investigación externa

Hice 3 búsquedas rápidas. Las fuentes son secundarias; no verifiqué nada contra documentación primaria.

1. **Obsidian Sync.** Según el [mirror de la ayuda oficial](https://www.mintlify.com/obsidianmd/obsidian-help/sync/settings), la resolución de conflictos es una opción por dispositivo: fusión automática (por defecto, a veces duplica texto) o crear una copia "conflicted copy". Los JSON de ajustes se fusionan clave por clave. **No encontré** una descripción oficial del diseño de claves ni del algoritmo de fusión. Un [blog de terceros](https://notes-automate.pages.dev/posts/obsidian-sync-vs-syncthing-for-free-note-synchronization/) contradice la fusión por defecto; priorizo la ayuda oficial.
2. **CRDT frente a OT.** Según varios artículos ([DEV](https://dev.to/beefedai/crdt-vs-ot-choosing-the-right-collaboration-algorithm-50io), [HackerNoon](https://sia.hackernoon.com/crdts-vs-operational-transformation-a-practical-guide-to-real-time-collaboration), [handbook](https://handbook-hld.pages.dev/curriculum/architecture-patterns/crdt-applications/)), OT necesita servidor central que ordene operaciones; los CRDT convergen sin servidor y sirven para offline. Para texto, Yjs es la opción habitual y Automerge se orienta a datos tipo JSON. Las cifras de rendimiento que vi son de segunda mano y se contradicen.
3. **Apps existentes con Tauri 2 y CodeMirror 6.** [doc-md](https://gh.r-mj.com/paperhurts/doc-md), [Opensidian](https://github.ddaodan.workers.dev/markusbegerow/opensidian) y [Vaultcore](https://github.com/herox215/Vaultcore) ya hacen wikilinks, backlinks y grafo en escritorio, con Rust para archivos e índice. **El móvil es el punto débil:** [SwarmNote](https://dev.to/yexiyue/one-rust-core-two-app-shells-tauri-react-native-in-swarmnote-2aam) reporta que el sistema de archivos móvil de Tauri v2 fue difícil y pasó a React Native con un núcleo Rust compartido. No encontré ningún ejemplo publicado de este tipo de app en Tauri móvil.

**Conocimiento propio, sin consulta externa en esta sesión (verificar):** Obsidian es software propietario, así que se replica su funcionalidad, no su código ni su marca. `diff3` y la fusión de tres vías son técnicas estándar de control de versiones. Syncthing existe como sincronizador gratuito de carpetas.

---

## Parte C. Recomendaciones y plan

### 1. Decisiones de arquitectura

| Tema | Elección | Motivo | Alternativa |
|---|---|---|---|
| Formato | Carpeta de `.md` + `.romsidian/` para ajustes | Portable; abre en cualquier editor | Base de datos propia (descartada) |
| Escritorio | Tauri 2 (Rust + WebView) | Poca RAM en tus 7 GB; Rust ya está instalado | Electron (descartado por memoria) |
| Editor | CodeMirror 6 | Lo usan las apps de referencia de la búsqueda 3 | ProseMirror/Tiptap (WYSIWYG, más complejo) |
| UI | Svelte 5 o React (decidir en etapa 0) | Ambos aparecen en las referencias | — |
| Índice | Rust: enlaces, backlinks, etiquetas, búsqueda (SQLite FTS5) | Mantiene la UI liviana | — |
| Sincronización (decidido 2026-11-10) | **MVP: Git contra un repositorio privado de GitHub tuyo**, detrás de una interfaz `SyncBackend` intercambiable. Fusión de tres vías de Git; si falla, copia en conflicto | No exige servidor; Git ya da historial y detección de conflictos. Elegido por vos | Servicio propio por archivo (ver siguiente fila) |
| Sincronización, fase 2 (opcional) | Servicio propio en tu VPS, con la misma interfaz `SyncBackend` | Para adjuntos grandes, sincronización casi inmediata o cifrado | CRDT (Yjs) solo si se pide edición en vivo |
| Cifrado | **Sin E2EE en el MVP**: te da igual que GitHub lea las notas. Repositorio privado y token de acceso limitado a ese repositorio | Decisión tuya del 2026-11-10 | E2EE en la fase 2, si cambiás de opinión |
| Servidor | **No hace falta en el MVP.** Tu Hostinger es un VPS KVM 1; queda reservado para la fase 2 | Un VPS permite correr un servicio propio. No verifiqué las especificaciones del plan KVM 1 | — |
| Móvil | **Android.** Etapa 6, tras un prototipo de riesgo (ver etapa 5) | Es la parte menos probada | PWA, o React Native + núcleo Rust |

**Por qué no un CRDT desde el inicio:** las notas se editan casi siempre en un dispositivo a la vez. Un CRDT agrega complejidad y metadatos. Mi regla de decisión: se adopta Yjs si las pruebas de la etapa 4 muestran más de un conflicto por cada 100 sincronizaciones reales, o si querés edición simultánea.

Estructura del repo (fuera de la bóveda, por ejemplo `C:\Users\Roma\Documents\Proyectos\romsidian`):

```
romsidian/
  CLAUDE.md  AGENTS.md  REVIEW.md  feature_list.json
  docs/      ARCHITECTURE  SYNC-PROTOCOL  CONVENTIONS  VERIFICATION  DECISIONS
  progress/  current.md  history.md
  scripts/   init.ps1
  crates/    core (vault, parser, index)  sync-client  sync-server
  apps/      desktop (Tauri + UI)  mobile (etapa 6)
  tests/     unit  integration  sync-scenarios  ui
  .claude/   agents/  settings.json
```

### 2. Funcionalidades

| Nivel | Funciones |
|---|---|
| MVP escritorio | Abrir carpeta como bóveda, árbol de archivos, editor Markdown, vista previa, `[[wikilinks]]`, backlinks, etiquetas, frontmatter, búsqueda, tema claro y oscuro |
| MVP sincronización | Cuenta o clave, vincular dispositivos, sincronizar `.md` y adjuntos, resolver conflictos, historial de versiones, cifrado E2EE |
| Después | Grafo, plantillas, canvas, plugins, móvil completo, edición en vivo, publicación |

Fuera del MVP: plugins y compatibilidad con plugins de Obsidian.

### 3. Etapas

Cada etapa se prueba en tu PC y termina con tu aprobación (XCB 05:39).

| # | Etapa | Entregable | Criterio de salida |
|---|---|---|---|
| 0 | Medición y decisión | Probar Obsidian con Syncthing y 1–2 apps de la búsqueda 3. Spike de Tauri con CodeMirror. `DECISIONS.md` | Decidís si construir o adoptar; elegida la UI |
| 1 | Repo y arnés | Archivos del arnés, `init.ps1`, agentes, hooks | `init.ps1` pasa y falla cuando corresponde |
| 2 | Núcleo sin interfaz | Biblioteca Rust: leer bóveda, parsear enlaces, índice, búsqueda | Tests con 1.000 notas, nombres con tildes y espacios |
| 3 | Editor de escritorio | Árbol, editor, vista previa, wikilinks, backlinks | Abrir una bóveda real tuya sin alterar los archivos |
| 4 | Sincronización con GitHub | Interfaz `SyncBackend`, backend Git (pull, commit, push), manejo de conflictos, autenticación con token | Escenarios: edición sin conexión en 2 dispositivos, borrado contra edición, renombrado, red cortada a mitad del push |
| 5 | Prototipo móvil de riesgo (Android) | Mismo núcleo en tu celular; acceso a carpetas con el sistema de archivos de Android; cliente Git en el teléfono (a evaluar: libgit2/gitoxide desde Rust, isomorphic-git en la WebView o la API HTTP de GitHub) | Leer, editar y sincronizar una nota en tu teléfono |
| 6 | App móvil | Interfaz táctil sobre lo anterior | Uso diario con sincronización con la PC |
| 7 | Robustez e instalador | Recuperación tras cortes, historial, instalador | Instalar desde cero y sincronizar con una bóveda grande |
| 8 | Uso real | Un mes con tu bóveda y lista de pendientes | Vos decidís qué sigue |

Tareas de la etapa 4 (la más delicada): (1) definir `SYNC-PROTOCOL.md` y la interfaz `SyncBackend`; (2) backend Git sobre un repositorio privado; (3) token de acceso restringido a ese repositorio, guardado en el almacén de credenciales del sistema; (4) sincronización automática y manual; (5) conflictos: fusión y copia en conflicto visible; (6) tests de escenarios con dos clientes simulados; (7) prueba con red cortada a mitad de transferencia; (8) `.gitignore` para ajustes locales y aviso por adjuntos grandes.

Tareas de la fase 2 (solo si la pedís): backend propio en el VPS, con HTTPS y, si lo querés, cifrado de extremo a extremo.

### 4. Uso de Claude Code y agentes

Mismo ciclo que el Transcriptor: ejecutar `init.ps1`, tomar una tarea de `feature_list.json`, el `implementer` escribe código y tests, presenta evidencia, el `reviewer` con contexto limpio revisa (máximo 3 rondas) y vos aprobás con las preguntas de XCB 06:40. *Interpretación mía:* para la etapa 4, un agente revisor dedicado a sincronización, con casos de pérdida de datos en `REVIEW.md`.

Reparto de modelos sugerido, según [[Claude]]: Fable para diseño del protocolo, Opus para implementar y Sonnet para tareas triviales. Es una guía de una sola fuente.

---

## Parte D. Riesgos

| Riesgo | Impacto | Mitigación |
|---|---|---|
| Pérdida de datos en conflictos de sincronización | Alto | Copia en conflicto siempre disponible, historial, tests de escenarios |
| Móvil con Tauri inmaduro | Alto | Prototipo de riesgo en etapa 5, antes de comprometer la app móvil |
| Alcance: "igual a Obsidian" es enorme | Alto | MVP acotado; plugins y canvas fuera |
| Ya existen soluciones gratuitas (Obsidian + Syncthing, apps open source) | Decisión tuya | Etapa 0 |
| Token de GitHub filtrado o con permisos de más | Alto | Token limitado a un repositorio, en el almacén de credenciales, nunca dentro de la bóveda |
| Límites de GitHub: archivos de más de 100 MB bloqueados y repos pesados lentos (de memoria, verificar) | Medio | Aviso por adjuntos grandes; fase 2 en el VPS |
| Cliente Git en Android no probado | Alto | Evaluarlo en la etapa 5 antes de comprometer la app móvil |
| Sincronización por pasos, no inmediata | Bajo | Automática cada pocos minutos y botón manual |
| RAM de 7 GB durante la compilación de Rust | Medio | Builds incrementales; un agente a la vez |
| Propiedad intelectual: copiar la marca o la interfaz de Obsidian | Medio | Nombre y diseño propios |
| Tests verdes con experiencia mala | Medio | Uso real sobre tu bóveda en cada etapa |

## Próximo paso

La etapa 0 arranca cuando la pidas. Decisiones ya tomadas: celular Android, sincronización con GitHub en el MVP, sin cifrado de extremo a extremo, y un VPS KVM 1 de Hostinger reservado para la fase 2. Para la etapa 0 necesito que me digas qué repositorio privado vas a usar para las pruebas y una bóveda de ejemplo (no la principal).

---

## Instrucciones de traspaso a otro agente de IA

Esta sección se puede copiar sola. Está escrita para un agente que no tiene acceso a esta bóveda ni a la conversación que dio origen al plan.

### 1. Prompt para pegar al agente

```
Sos el agente de desarrollo del proyecto ROMSIDIAN. Leé primero todo este documento.
No escribas código hasta terminar la etapa 0 y recibir mi aprobación.

OBJETIVO
Una app de notas en Markdown, parecida a Obsidian en funcionalidad (no en código ni marca),
con sincronización entre mi PC con Windows 11 y mi celular Android.

DECISIONES YA TOMADAS (no las reabras sin pedirme permiso)
- Bóveda = carpeta de archivos .md comunes; ajustes de la app en una subcarpeta .romsidian/.
- Escritorio: Tauri 2 (Rust + WebView) con CodeMirror 6. UI: Svelte 5 o React, a decidir en la etapa 0.
- Núcleo en Rust: lectura de la bóveda, parseo de [[wikilinks]], backlinks, etiquetas, frontmatter,
  índice y búsqueda (SQLite FTS5).
- Sincronización del MVP: Git contra un repositorio privado de GitHub mío, detrás de una interfaz
  `SyncBackend` intercambiable. Sin cifrado de extremo a extremo (acepté que GitHub lea mis notas).
- Un VPS KVM 1 de Hostinger queda reservado para una fase 2 opcional. No lo uses sin que te lo pida.
- Celular: Android. El móvil se hace en la etapa 5 (prototipo de riesgo) y la 6 (app).
- Mi PC: Ryzen 3 7320U, 7,21 GB de RAM utilizables. Descartado Electron. Compilá de a poco.
- Instalado: Python 3.13, Git, Node, Cargo y winget.

FORMA DE TRABAJO
- Etapas chicas. Cada una termina con evidencia (salida de tests, captura de pantalla, archivo generado)
  y con mi aprobación explícita antes de pasar a la siguiente.
- Antes de dar una tarea por hecha, ejecutá los tests y mostrá el resultado.
- Una revisión con contexto limpio por tarea; máximo 3 rondas entre autor y revisor, y luego replanteá.
- Si un fallo se repite, convertilo en regla o en test.
- Cada decisión importante va a docs/DECISIONS.md con el motivo.
- Respondeme en español rioplatense, sin jerga innecesaria. Explicá lo que hacés en términos simples.

QUÉ HACER AHORA: etapa 0 y etapa 1 (ver abajo). Al final de cada una, detenete y pedime aprobación.
```

### 2. Etapa 0: medición y decisión (sin construir la app)

Entregable: `docs/DECISIONS.md` con las conclusiones.

1. **Probar lo que ya existe** durante un rato, con una bóveda de prueba (50 a 100 notas con tildes y adjuntos), no con la principal: Obsidian con el plugin Obsidian Git o con Syncthing, y 1 o 2 apps open source de escritorio que usan Tauri 2 y CodeMirror 6 (por ejemplo doc-md, Opensidian o Vaultcore). Anotar qué le falta a cada una respecto de lo que quiero.
2. **Decidir con datos si conviene construir o adoptar.** Si algo existente cubre lo que necesito, decirlo con claridad. La decisión es mía.
3. **Spike de Tauri 2 con CodeMirror 6:** abrir una carpeta, mostrar un archivo `.md`, editarlo y guardarlo, y medir el consumo de memoria con la bóveda de prueba. Elegir Svelte o React con una razón concreta.
4. **Spike de sincronización por Git** en PC: dos copias de la bóveda, edición en ambas, y ver qué pasa con los conflictos. Elegir la biblioteca Git de Rust (libgit2 o gitoxide) comparando con llamar al ejecutable `git`.
5. **Verificar las afirmaciones sin comprobar** de la tabla de la sección siguiente.

### 3. Qué quedó sin verificar (revisar antes de confiar)

| Afirmación | Origen | Qué hacer |
|---|---|---|
| Obsidian Sync fusiona por defecto y puede crear copia en conflicto | Espejo de la ayuda oficial, no el sitio oficial | Leer la ayuda en obsidian.md |
| Tauri 2 móvil tiene problemas con el sistema de archivos de Android | Un solo artículo (SwarmNote) | Probarlo en la etapa 5 |
| GitHub bloquea archivos de más de 100 MB y los repos grandes se vuelven lentos | Memoria del asistente | Leer la documentación de GitHub |
| Especificaciones del plan KVM 1 de Hostinger | No consultadas | Consultar a Hostinger si se llega a la fase 2 |
| Cifras de rendimiento de Yjs frente a Automerge | Blogs de segunda mano que se contradicen | Medir si se adopta un CRDT |
| Se adopta un CRDT si hay más de 1 conflicto cada 100 sincronizaciones reales | Umbral propuesto por el asistente, sin respaldo | Reevaluarlo con datos de uso |

### 4. Etapa 1: repo y arnés

Crear el repositorio **fuera de la bóveda de notas**, por ejemplo en `C:\Users\Roma\Documents\Proyectos\romsidian`, con esta estructura:

```
romsidian/
  CLAUDE.md  AGENTS.md  REVIEW.md  feature_list.json
  docs/      ARCHITECTURE  SYNC-PROTOCOL  CONVENTIONS  VERIFICATION  DECISIONS
  progress/  current.md  history.md
  scripts/   init.ps1
  crates/    core  sync-client
  apps/      desktop  mobile
  tests/     unit  integration  sync-scenarios  ui
  .claude/   agents/  settings.json
```

Contenido mínimo de cada pieza:
- **`AGENTS.md`:** punto de entrada. Primero ejecutar `scripts/init.ps1`; leer `progress/current.md`; tomar la primera tarea pendiente de `feature_list.json`; no marcar una tarea como hecha sin tests en verde; mapa del repo. Si se usa Claude Code, hacer que `CLAUDE.md` contenga `@AGENTS.md` y no pasar de unas 200 líneas (dato de la documentación oficial, a confirmar).
- **`scripts/init.ps1`:** verifica que estén Rust, Node y Git, que existan los archivos del protocolo y que los tests pasen. Si falla, no se trabaja.
- **`feature_list.json`:** cada tarea con identificador, descripción, criterios de aceptación y estado (`pendiente`, `en curso`, `hecha`). Cargar las etapas 2 a 8 de la tabla de etapas.
- **`progress/`:** `current.md` con el estado actual y `history.md` con el historial, para no perder información entre sesiones.
- **`REVIEW.md`:** qué cuenta como fallo. Prioridad máxima: cualquier pérdida de datos del usuario. Cada aviso con archivo y línea.
- **`.claude/agents/`** (si se usa Claude Code): `implementer`, `reviewer` y `explorer`, de a uno por vez para no agotar la RAM.
- **Hooks:** formateo y análisis estático (`cargo fmt`, `clippy`, `eslint`) tras editar; `init.ps1` rápido al terminar. Los hooks pueden correr en PowerShell. Si un hook puede bloquear el cierre de sesión no está confirmado.

Criterio de salida: `init.ps1` pasa en un repositorio sano y falla cuando se rompe algo a propósito.

### 5. Reglas de seguridad que el agente debe respetar

- No tocar la bóveda principal de notas en las pruebas: usar siempre copias.
- El token de GitHub debe limitarse a un único repositorio, guardarse en el almacén de credenciales del sistema y no escribirse nunca en archivos, registros ni en Git.
- No subir `vendor/`, modelos pesados ni credenciales al repositorio.
- No copiar código, nombre, marca ni diseño de Obsidian, que es software propietario.
- No usar el VPS ni instalar nada allí sin mi aprobación.
- Ante cualquier operación que pueda borrar o sobrescribir notas, probar primero con una copia y mostrarme el plan.

### 6. Lo que decido yo

Qué se construye, el gusto de la interfaz, cuándo pasar a la siguiente etapa, si adopto una herramienta existente en vez de construir, y si se activa la fase 2 en el VPS. El agente propone y demuestra; yo apruebo.

### 7. Glosario para el agente

- **Bóveda:** la carpeta de notas.
- **Fusión de tres vías:** combinar dos versiones editadas de un archivo usando su versión común anterior.
- **E2EE:** cifrado de extremo a extremo.
- **CRDT:** estructura de datos que permite editar en paralelo sin servidor central. Solo si hace falta edición en vivo.
- **Arnés:** el entorno de reglas, pruebas y archivos de contexto que rodea al agente.
