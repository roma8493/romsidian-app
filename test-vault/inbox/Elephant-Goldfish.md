---
title: Elephant-Goldfish
created: 2026-11-15
tags:
  - Elephant-Goldfis
---

# Elephant-Goldfish

https://youtu.be/TBcgwD9sygM?si=aizo0uniL9w7AFzN

Este video presenta el **Elephant-Goldfish Model (EGM)**, un sistema de código abierto de *Google* diseñado para mejorar la programación con Inteligencia Artificial y evitar la "contaminación de contexto" (1:01). El método se basa en dividir el trabajo en tres roles clave (1:23-1:49):

1.  **El Elefante:** Un agente con memoria a largo plazo que mantiene el contexto global del proyecto y diseña la arquitectura.
2.  **El Pez Dorado (Goldfish):** Un escuadrón de subagentes sin memoria previa que revisa el trabajo en frío, buscando fallos y puntos ciegos técnicos.
3.  **Tú (el arquitecto):** El humano que toma la decisión final sobre las propuestas generadas.

**Herramientas y flujo de trabajo:**

*   Se integra en entornos como *Claude Code*, *Codex CLI* y *Antigravity CLI* (1:53-2:03).
*   **Comandos principales:** `/egm-brainstorm` para ideas (2:54), `/egm-draft` para documentación (3:50), `/egm-review` y `/egm-resolve` para auditorías técnicas (4:11-4:50).
*   **Proyectos existentes:** Permite usar `/egm-bootstrap` para documentar repositorios antiguos y `/egm-drift-report` para verificar si la implementación se ha desviado del diseño original (6:45-7:30).
*   **Hack de productividad:** El comando `/egm-review-loop` permite usar esta metodología de auditoría para cualquier tipo de documento, no solo código (7:43).

**Consideraciones importantes:**

*   El modelo tiene limitaciones, como la necesidad de entornos que soporten subagentes puros, el consumo de tokens y la persistencia de la necesidad de pruebas automatizadas (5:16-6:12).
*   El valor real no está en la generación masiva de código, sino en la **claridad del diseño** y la capacidad de cuestionar las suposiciones propias antes de escribir una sola línea de código (9:47-10:00).

## Conexiones Relacionadas
- [[open claw]] — Términos afines: código, trabajo, memoria
- [[harnes]] — Términos afines: trabajo
