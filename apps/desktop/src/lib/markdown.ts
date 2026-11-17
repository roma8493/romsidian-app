import DOMPurify from 'dompurify';
import { marked } from 'marked';
import type {
  DataviewQueryResult,
  DataviewValue,
  TransclusionResolution,
} from './types';

export interface ParsedWikilinkMatch {
  raw: string;
  target: string;
  heading: string | null;
  alias: string | null;
  displayText: string;
  from: number;
  to: number;
}

export interface PreviewTransclusionDirective {
  key: string;
  target: string;
  subtarget: string | null;
}

export interface PreviewDirectives {
  transclusions: PreviewTransclusionDirective[];
  inlineExprs: string[];
  dqlQueries: string[];
  dataviewJsSnippets: string[];
}

export interface MarkdownRenderContext {
  transclusions?: Map<string, TransclusionResolution | null>;
  inlineValues?: Map<string, DataviewValue>;
  dqlResults?: Map<string, DataviewQueryResult | { error: string }>;
  dataviewJsEnabled?: boolean;
  dataviewJsOutputs?: Map<string, string>;
  visitedPaths?: Set<string>;
  depth?: number;
}

const WIKILINK_REGEX = /(!?)\[\[([^\]\n|#]+)(?:#([^\]\n|]+))?(?:\|([^\]\n]+))?\]\]/g;
const FRONTMATTER_REGEX = /^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/;
const FENCED_BLOCK_SPLIT_REGEX = /(^```[^\n]*\r?\n[\s\S]*?^```[ \t]*$)/gm;
const INLINE_CODE_SPLIT_REGEX = /(``+[\s\S]*?``+|`[^`\n]+`)/g;
const INLINE_BRACKET_FIELD_REGEX =
  /\[([A-Za-zÀ-ÿ0-9_-][A-Za-zÀ-ÿ0-9 _-]{0,38})::\s*((?:\[\[[^\]\n]+\]\]|[^\]\n])+)\]/g;
const INLINE_PAREN_FIELD_REGEX = /\(([A-Za-zÀ-ÿ0-9_-][A-Za-zÀ-ÿ0-9 _-]{0,38})::\s*([^)\n]+)\)/g;
const LINE_FIELD_REGEX = /^(\s*(?:[-*]\s+)?)([A-Za-zÀ-ÿ0-9_-][A-Za-zÀ-ÿ0-9 _-]{0,38})::\s*(.+)$/;

marked.use({
  renderer: {
    link({ href, title, tokens }) {
      const text = this.parser.parseInline(tokens);
      const isExternal = /^https?:\/\//i.test(href) || /^mailto:/i.test(href);
      const titleAttr = title ? ` title="${escapeHtmlAttr(title)}"` : '';
      if (isExternal) {
        return `<a href="${escapeHtmlAttr(href)}" target="_blank" rel="noopener noreferrer" class="external-link"${titleAttr}>${text} ↗</a>`;
      }
      return `<a href="${escapeHtmlAttr(href)}"${titleAttr}>${text}</a>`;
    },
  },
});

/**
 * Normalizes a note title/stem for case-insensitive resolution against known notes.
 */
export function normalizeNoteKey(value: string): string {
  return value.trim().replace(/\.md$/i, '').toLowerCase();
}

/**
 * Canonical cache key for resolving `![[target#subtarget]]` embeds.
 */
export function makeTransclusionKey(target: string, subtarget: string | null): string {
  return `${target.trim().toLowerCase()}::${(subtarget ?? '').trim().toLowerCase()}`;
}

/**
 * Strips leading YAML frontmatter block so the Markdown preview displays
 * clean prose while the Properties panel handles structured metadata.
 */
export function stripFrontmatter(markdown: string): string {
  return markdown.replace(FRONTMATTER_REGEX, '');
}

/**
 * Finds all `[[wikilink]]` and `![[transclusion]]` ranges in plain text (CodeMirror).
 */
export function findWikilinksInText(text: string): ParsedWikilinkMatch[] {
  const matches: ParsedWikilinkMatch[] = [];
  WIKILINK_REGEX.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = WIKILINK_REGEX.exec(text)) !== null) {
    const target = match[2].trim();
    const heading = match[3]?.trim() || null;
    const alias = match[4]?.trim() || null;
    if (!target) continue;
    const displayText = alias ?? (heading ? `${target} › ${heading}` : target);
    matches.push({
      raw: match[0],
      target,
      heading,
      alias,
      displayText,
      from: match.index,
      to: match.index + match[0].length,
    });
  }
  return matches;
}

/**
 * Extracts dynamic directives (`![[...]]`, `` `= ...` ``, ```` ```dataview ````,
 * ```` ```dataviewjs ````) from Markdown outside ordinary fenced/inline code spans.
 */
export function extractPreviewDirectives(markdown: string): PreviewDirectives {
  const body = stripFrontmatter(markdown);
  const transclusionsMap = new Map<string, PreviewTransclusionDirective>();
  const inlineSet = new Set<string>();
  const dqlSet = new Set<string>();
  const dvjsSet = new Set<string>();

  const segments = body.split(FENCED_BLOCK_SPLIT_REGEX);
  for (const segment of segments) {
    const fenceMatch = /^```([^\r\n]*)\r?\n([\s\S]*?)\r?\n?```[ \t]*$/.exec(segment);
    if (fenceMatch) {
      const lang = fenceMatch[1].trim().toLowerCase();
      const code = fenceMatch[2].trim();
      if (lang === 'dataview' && code) {
        dqlSet.add(code);
      } else if (lang === 'dataviewjs' && code) {
        dvjsSet.add(code);
      }
      continue;
    }

    const inlineParts = segment.split(INLINE_CODE_SPLIT_REGEX);
    for (let i = 0; i < inlineParts.length; i++) {
      const part = inlineParts[i];
      if (i % 2 === 1) {
        if (part.startsWith('`=') && !part.startsWith('``') && part.endsWith('`')) {
          const expr = part.slice(2, -1).trim();
          if (expr) inlineSet.add(expr);
        }
        continue;
      }

      WIKILINK_REGEX.lastIndex = 0;
      let match: RegExpExecArray | null;
      while ((match = WIKILINK_REGEX.exec(part)) !== null) {
        if (match[1] !== '!') continue;
        const target = match[2].trim();
        if (!target) continue;
        const subtarget = match[3]?.trim() || null;
        const key = makeTransclusionKey(target, subtarget);
        if (!transclusionsMap.has(key)) {
          transclusionsMap.set(key, { key, target, subtarget });
        }
      }
    }
  }

  return {
    transclusions: Array.from(transclusionsMap.values()),
    inlineExprs: Array.from(inlineSet),
    dqlQueries: Array.from(dqlSet),
    dataviewJsSnippets: Array.from(dvjsSet),
  };
}

function escapeHtmlAttr(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

export function escapeHtmlText(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function renderWikilinkButtonHtml(
  target: string,
  heading: string | null,
  alias: string | null,
  knownNoteKeys: Set<string>,
): string {
  const cleanTarget = target.trim();
  const cleanHeading = heading?.trim() ?? '';
  const cleanAlias = alias?.trim() ?? '';
  const resolved = knownNoteKeys.has(normalizeNoteKey(cleanTarget));
  const label = cleanAlias || (cleanHeading ? `${cleanTarget} › ${cleanHeading}` : cleanTarget);
  const classes = resolved ? 'md-wikilink resolved' : 'md-wikilink pending';
  const titleAttr = resolved
    ? `Abrir nota: ${cleanTarget}`
    : `Nota inexistente — clic para crear "${cleanTarget}.md"`;
  return `<button type="button" class="${classes}" data-wikilink="${escapeHtmlAttr(
    cleanTarget,
  )}" data-heading="${escapeHtmlAttr(cleanHeading)}" title="${escapeHtmlAttr(
    titleAttr,
  )}">${escapeHtmlText(label)}</button>`;
}

function renderFieldValueHtml(rawValue: string, knownNoteKeys: Set<string>): string {
  let out = '';
  let lastIdx = 0;
  WIKILINK_REGEX.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = WIKILINK_REGEX.exec(rawValue)) !== null) {
    out += escapeHtmlText(rawValue.slice(lastIdx, match.index));
    out += renderWikilinkButtonHtml(match[2], match[3] ?? null, match[4] ?? null, knownNoteKeys);
    lastIdx = match.index + match[0].length;
  }
  out += escapeHtmlText(rawValue.slice(lastIdx));
  return out;
}

export function formatDataviewValueHtml(
  value: DataviewValue | null | undefined,
  knownNoteKeys: Set<string>,
): string {
  if (!value || value.kind === 'null') {
    return '<span class="md-dv-null">—</span>';
  }
  if (value.kind === 'bool') {
    return `<span class="md-dv-bool">${value.value ? 'Sí' : 'No'}</span>`;
  }
  if (value.kind === 'number') {
    return `<span class="md-dv-num">${escapeHtmlText(String(value.value))}</span>`;
  }
  if (value.kind === 'text') {
    return escapeHtmlText(value.value);
  }
  if (value.kind === 'link') {
    const resolved =
      knownNoteKeys.has(normalizeNoteKey(value.path)) ||
      knownNoteKeys.has(normalizeNoteKey(value.label));
    const classes = resolved ? 'md-wikilink resolved' : 'md-wikilink pending';
    return `<button type="button" class="${classes}" data-wikilink="${escapeHtmlAttr(
      value.path || value.label,
    )}" data-heading="">${escapeHtmlText(value.label)}</button>`;
  }
  if (value.items.length === 0) {
    return '<span class="md-dv-null">[]</span>';
  }
  return value.items.map((item) => formatDataviewValueHtml(item, knownNoteKeys)).join(', ');
}

function renderDataviewResultCard(
  query: string,
  result: DataviewQueryResult | { error: string } | undefined,
  knownNoteKeys: Set<string>,
): string {
  if (!result) {
    return `<div class="md-dv-card loading"><div class="md-dv-header"><span>Dataview DQL</span><span class="md-dv-meta">Consultando…</span></div><pre><code>${escapeHtmlText(
      query,
    )}</code></pre></div>`;
  }

  if ('error' in result) {
    return `<div class="md-dv-card error"><div class="md-dv-header"><span>Dataview DQL · Error</span></div><p class="md-dv-error-msg">${escapeHtmlText(
      result.error,
    )}</p><pre><code>${escapeHtmlText(query)}</code></pre></div>`;
  }

  if (result.view === 'table') {
    const headHtml = result.headers.map((h) => `<th>${escapeHtmlText(h)}</th>`).join('');
    const bodyHtml =
      result.rows.length === 0
        ? `<tr><td colspan="${Math.max(1, result.headers.length)}" class="md-dv-empty">Sin resultados para esta consulta.</td></tr>`
        : result.rows
            .map(
              (row) =>
                `<tr>${row
                  .map((cell) => `<td>${formatDataviewValueHtml(cell, knownNoteKeys)}</td>`)
                  .join('')}</tr>`,
            )
            .join('');
    return `<div class="md-dv-card"><div class="md-dv-header"><span>Dataview TABLE</span><span class="md-dv-meta">${result.total_count} fila(s)</span></div><div class="md-dv-table-wrap"><table><thead><tr>${headHtml}</tr></thead><tbody>${bodyHtml}</tbody></table></div></div>`;
  }

  if (result.view === 'list') {
    const listHtml =
      result.items.length === 0
        ? '<p class="md-dv-empty">Sin notas coincidentes.</p>'
        : `<ul class="md-dv-list">${result.items
            .map((item) => {
              const linkHtml = renderWikilinkButtonHtml(item.path, null, item.title, knownNoteKeys);
              const valHtml = item.value
                ? `: <span class="md-dv-list-val">${formatDataviewValueHtml(
                    item.value,
                    knownNoteKeys,
                  )}</span>`
                : '';
              return `<li>${linkHtml}${valHtml}</li>`;
            })
            .join('')}</ul>`;
    return `<div class="md-dv-card"><div class="md-dv-header"><span>Dataview LIST</span><span class="md-dv-meta">${result.total_count} elemento(s)</span></div>${listHtml}</div>`;
  }

  const tasksHtml =
    result.items.length === 0
      ? '<p class="md-dv-empty">No se encontraron tareas.</p>'
      : `<ul class="md-dv-tasks">${result.items
          .map((task) => {
            const checkMark = task.completed ? '☑' : '☐';
            const sourceLink = renderWikilinkButtonHtml(
              task.path,
              null,
              task.note_title,
              knownNoteKeys,
            );
            return `<li class="${task.completed ? 'done' : ''}"><span class="md-dv-check">${checkMark}</span><span class="md-dv-task-text">${escapeHtmlText(
              task.text,
            )}</span> <span class="md-dv-task-source">(${sourceLink})</span></li>`;
          })
          .join('')}</ul>`;

  return `<div class="md-dv-card"><div class="md-dv-header"><span>Dataview TASK</span><span class="md-dv-meta">${result.total_count} tarea(s)</span></div>${tasksHtml}</div>`;
}

function renderTransclusionCard(
  target: string,
  subtarget: string | null,
  alias: string | null,
  knownNoteKeys: Set<string>,
  context: MarkdownRenderContext,
): string {
  const depth = context.depth ?? 0;
  const key = makeTransclusionKey(target, subtarget);
  const resolved = context.transclusions?.get(key);
  const label = alias || (subtarget ? `${target} › ${subtarget}` : target);
  const headerLink = renderWikilinkButtonHtml(target, subtarget, label, knownNoteKeys);

  if (depth >= 2) {
    return `<div class="md-transclusion"><div class="md-transclusion-header"><span class="md-transclusion-badge">Transclusión</span>${headerLink}</div><div class="md-transclusion-body"><p class="md-dv-empty">Límite de anidamiento alcanzado.</p></div></div>`;
  }

  if (resolved === undefined) {
    return `<div class="md-transclusion loading"><div class="md-transclusion-header"><span class="md-transclusion-badge">Transclusión</span>${headerLink}</div><div class="md-transclusion-body"><p class="md-dv-empty">Cargando contenido incrustado…</p></div></div>`;
  }

  if (resolved === null) {
    return `<div class="md-transclusion missing"><div class="md-transclusion-header"><span class="md-transclusion-badge">Sin resolver</span>${headerLink}</div><div class="md-transclusion-body"><p class="md-dv-empty">No se encontró la nota o sección "${escapeHtmlText(
      label,
    )}".</p></div></div>`;
  }

  const visited = new Set(context.visitedPaths ?? []);
  if (visited.has(resolved.resolved_path)) {
    return `<div class="md-transclusion"><div class="md-transclusion-header"><span class="md-transclusion-badge">Referencia circular</span>${headerLink}</div><div class="md-transclusion-body"><p class="md-dv-empty">Se omitió recursión sobre ${escapeHtmlText(
      resolved.resolved_path,
    )}.</p></div></div>`;
  }

  visited.add(resolved.resolved_path);
  const innerHtml = renderMarkdownBodyInternal(resolved.content, knownNoteKeys, {
    ...context,
    visitedPaths: visited,
    depth: depth + 1,
  });

  return `<div class="md-transclusion"><div class="md-transclusion-header"><span class="md-transclusion-badge">Incrustado</span>${headerLink}</div><div class="md-transclusion-body">${innerHtml}</div></div>`;
}

function renderMarkdownBodyInternal(
  markdown: string,
  knownNoteKeys: Set<string>,
  context: MarkdownRenderContext,
): string {
  const body = stripFrontmatter(markdown);
  const blockSlots: string[] = [];
  const inlineSlots: string[] = [];

  const pushBlock = (html: string): string => {
    const idx = blockSlots.length;
    blockSlots.push(html);
    return `\n\nROMSIDIANBLOCKPH${idx}END\n\n`;
  };

  const pushInline = (html: string): string => {
    const idx = inlineSlots.length;
    inlineSlots.push(html);
    return `ROMSIDIANINLINEPH${idx}END`;
  };

  const segments = body.split(FENCED_BLOCK_SPLIT_REGEX);
  const transformed = segments
    .map((segment) => {
      const fenceMatch = /^```([^\r\n]*)\r?\n([\s\S]*?)\r?\n?```[ \t]*$/.exec(segment);
      if (fenceMatch) {
        const lang = fenceMatch[1].trim().toLowerCase();
        const code = fenceMatch[2].trim();
        if (lang === 'dataview') {
          return pushBlock(renderDataviewResultCard(code, context.dqlResults?.get(code), knownNoteKeys));
        }
        if (lang === 'dataviewjs') {
          if (!context.dataviewJsEnabled) {
            return pushBlock(
              `<div class="md-dv-card blocked"><div class="md-dv-header"><span>DataviewJS · Bloqueado por seguridad</span></div><p class="md-dv-empty">El interruptor experimental de <code>DataviewJS</code> está desactivado por defecto. Actívalo en Ajustes solo para bóvedas de confianza o usa bloques <code>dataview</code> DQL.</p></div>`,
            );
          }
          const customOutput = context.dataviewJsOutputs?.get(code);
          return pushBlock(
            customOutput ??
              `<div class="md-dv-card"><div class="md-dv-header"><span>DataviewJS</span></div><p class="md-dv-empty">Sin salida generada.</p></div>`,
          );
        }
        return segment;
      }

      const inlineParts = segment.split(INLINE_CODE_SPLIT_REGEX);
      return inlineParts
        .map((part, idx) => {
          if (idx % 2 === 1) {
            if (part.startsWith('`=') && !part.startsWith('``') && part.endsWith('`')) {
              const expr = part.slice(2, -1).trim();
              if (expr) {
                const val = context.inlineValues?.get(expr);
                const valHtml =
                  val !== undefined
                    ? formatDataviewValueHtml(val, knownNoteKeys)
                    : '<span class="md-dv-null">…</span>';
                return pushInline(
                  `<span class="md-dv-inline" title="${escapeHtmlAttr(`= ${expr}`)}">${valHtml}</span>`,
                );
              }
            }
            return part;
          }

          // Process line-by-line for bare `Clave:: Valor` fields while preserving prose
          const lines = part.split(/\r?\n/).map((line) => {
            const trimmed = line.trim();
            if (!trimmed.startsWith('#') && !trimmed.startsWith('>')) {
              const bare = LINE_FIELD_REGEX.exec(line);
              if (bare && !bare[2].includes('http')) {
                const prefix = bare[1];
                const key = bare[2].trim();
                const val = bare[3].trim();
                return (
                  prefix +
                  pushInline(
                    `<span class="md-inline-field line"><span class="md-field-key">${escapeHtmlText(
                      key,
                    )}</span><span class="md-field-sep">::</span><span class="md-field-val">${renderFieldValueHtml(
                      val,
                      knownNoteKeys,
                    )}</span></span>`,
                  )
                );
              }
            }

            let outLine = line.replace(
              INLINE_BRACKET_FIELD_REGEX,
              (_full, rawKey: string, rawVal: string) =>
                pushInline(
                  `<span class="md-inline-field compact"><span class="md-field-key">${escapeHtmlText(
                    rawKey.trim(),
                  )}</span><span class="md-field-val">${renderFieldValueHtml(
                    rawVal.trim(),
                    knownNoteKeys,
                  )}</span></span>`,
                ),
            );

            outLine = outLine.replace(
              INLINE_PAREN_FIELD_REGEX,
              (_full, rawKey: string, rawVal: string) =>
                pushInline(
                  `<span class="md-inline-field subtle" title="${escapeHtmlAttr(
                    `${rawKey.trim()}:: ${rawVal.trim()}`,
                  )}"><span class="md-field-val">${renderFieldValueHtml(
                    rawVal.trim(),
                    knownNoteKeys,
                  )}</span></span>`,
                ),
            );

            return outLine.replace(
              WIKILINK_REGEX,
              (_full, embed: string, rawTarget: string, rawHeading?: string, rawAlias?: string) => {
                const target = rawTarget.trim();
                const subtarget = rawHeading?.trim() || null;
                const alias = rawAlias?.trim() || null;
                if (!target) return _full;
                if (embed === '!') {
                  return pushBlock(
                    renderTransclusionCard(target, subtarget, alias, knownNoteKeys, context),
                  );
                }
                return pushInline(
                  renderWikilinkButtonHtml(target, subtarget, alias, knownNoteKeys),
                );
              },
            );
          });

          return lines.join('\n');
        })
        .join('');
    })
    .join('');

  let html = marked.parse(transformed, {
    breaks: true,
    gfm: true,
  }) as string;

  for (let i = 0; i < blockSlots.length; i++) {
    const token = `ROMSIDIANBLOCKPH${i}END`;
    html = html
      .replace(`<p>${token}</p>`, blockSlots[i])
      .replace(token, blockSlots[i]);
  }
  for (let i = 0; i < inlineSlots.length; i++) {
    html = html.replace(`ROMSIDIANINLINEPH${i}END`, inlineSlots[i]);
  }

  return html;
}

/**
 * Executes an opt-in `DataviewJS` snippet against vault query data in a constrained sandbox.
 */
export function executeOptInDataviewJs(
  code: string,
  pagesResult: DataviewQueryResult,
  knownNoteKeys: Set<string>,
): string {
  const outputs: string[] = [];
  const pages =
    pagesResult.view === 'table'
      ? pagesResult.rows.map((row) => {
          const link = row[0];
          const path = link?.kind === 'link' ? link.path : '';
          const name = link?.kind === 'link' ? link.label : '';
          const rawTags = row[1];
          const tags =
            rawTags?.kind === 'list'
              ? rawTags.items.map((item) => (item.kind === 'text' ? item.value : ''))
              : [];
          return {
            file: { path, name, tags },
            title: row[2]?.kind === 'text' ? row[2].value : name,
            estado: row[3]?.kind === 'text' ? row[3].value : null,
            prioridad: row[4]?.kind === 'text' ? row[4].value : null,
          };
        })
      : [];

  const dv = {
    pages: (filter?: string) => {
      if (!filter || !filter.trim()) return pages;
      const clean = filter.trim().toLowerCase();
      if (clean.startsWith('#')) {
        return pages.filter((p) => p.file.tags.some((t) => t.toLowerCase() === clean));
      }
      const folder = clean.replace(/^["']|["']$/g, '');
      return pages.filter((p) => p.file.path.toLowerCase().startsWith(folder));
    },
    paragraph: (text: unknown) => {
      outputs.push(`<p>${escapeHtmlText(String(text ?? ''))}</p>`);
    },
    span: (text: unknown) => {
      outputs.push(`<span>${escapeHtmlText(String(text ?? ''))}</span>`);
    },
    header: (level: number, text: unknown) => {
      const clamped = Math.min(6, Math.max(1, Number(level) || 3));
      outputs.push(`<h${clamped}>${escapeHtmlText(String(text ?? ''))}</h${clamped}>`);
    },
    list: (items: unknown[]) => {
      const arr = Array.isArray(items) ? items : [];
      outputs.push(
        `<ul class="md-dv-list">${arr
          .map((it) => `<li>${escapeHtmlText(String(it ?? ''))}</li>`)
          .join('')}</ul>`,
      );
    },
    table: (headers: unknown[], rows: unknown[][]) => {
      const hs = Array.isArray(headers) ? headers : [];
      const rs = Array.isArray(rows) ? rows : [];
      outputs.push(
        `<div class="md-dv-table-wrap"><table><thead><tr>${hs
          .map((h) => `<th>${escapeHtmlText(String(h ?? ''))}</th>`)
          .join('')}</tr></thead><tbody>${rs
          .map(
            (r) =>
              `<tr>${(Array.isArray(r) ? r : [r])
                .map((c) => {
                  if (c && typeof c === 'object' && 'path' in c && typeof (c as { path: unknown }).path === 'string') {
                    const p = (c as { path: string; name?: string }).path;
                    const label = (c as { name?: string }).name || p.replace(/\.md$/i, '');
                    return `<td>${renderWikilinkButtonHtml(p, null, label, knownNoteKeys)}</td>`;
                  }
                  return `<td>${escapeHtmlText(String(c ?? ''))}</td>`;
                })
                .join('')}</tr>`,
          )
          .join('')}</tbody></table></div>`,
      );
    },
  };

  try {
    const runner = new Function('dv', `'use strict';\n${code}`);
    runner(dv);
    return `<div class="md-dv-card"><div class="md-dv-header"><span>DataviewJS (experimental)</span></div>${
      outputs.join('') || '<p class="md-dv-empty">Script ejecutado sin salida.</p>'
    }</div>`;
  } catch (err) {
    return `<div class="md-dv-card error"><div class="md-dv-header"><span>DataviewJS · Error</span></div><p class="md-dv-error-msg">${escapeHtmlText(
      String(err),
    )}</p></div>`;
  }
}

/**
 * Renders full Markdown preview including `[[wikilinks]]`, transclusions `![[...]]`,
 * inline fields, inline expressions `` `= ...` ``, and Dataview DQL/JS blocks,
 * sanitized strictly with `DOMPurify`.
 */
export function renderMarkdownPreview(
  markdown: string,
  knownNoteKeys: Set<string>,
  context: MarkdownRenderContext = {},
): string {
  const rawHtml = renderMarkdownBodyInternal(markdown, knownNoteKeys, {
    ...context,
    visitedPaths: context.visitedPaths ?? new Set<string>(),
    depth: context.depth ?? 0,
  });

  return DOMPurify.sanitize(rawHtml, {
    ADD_TAGS: ['button', 'mark'],
    ADD_ATTR: ['type', 'class', 'data-wikilink', 'data-heading', 'title', 'target', 'rel'],
  });
}

/**
 * Sanitizes SQLite FTS5 snippets keeping only `<mark>` highlight tags.
 */
export function sanitizeSearchSnippet(snippet: string): string {
  return DOMPurify.sanitize(snippet, {
    ALLOWED_TAGS: ['mark'],
    ALLOWED_ATTR: [],
  });
}
