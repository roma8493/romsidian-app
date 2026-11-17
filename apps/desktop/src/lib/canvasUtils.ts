import type {
  CanvasEdgeEnd,
  CanvasFileNode,
  CanvasJsonData,
  CanvasJsonEdge,
  CanvasJsonNode,
  CanvasSide,
} from './types';

export interface CanvasColorOption {
  code: string;
  label: string;
  hex: string;
}

export const CANVAS_COLOR_OPTIONS: CanvasColorOption[] = [
  { code: '1', label: 'Rojo', hex: '#ef4444' },
  { code: '2', label: 'Naranja', hex: '#f97316' },
  { code: '3', label: 'Amarillo', hex: '#eab308' },
  { code: '4', label: 'Verde', hex: '#22c55e' },
  { code: '5', label: 'Cian', hex: '#06b6d4' },
  { code: '6', label: 'Púrpura', hex: '#a855f7' },
];

const PRESET_HEX_MAP = new Map(CANVAS_COLOR_OPTIONS.map((c) => [c.code, c.hex]));

export function createCanvasId(prefix = 'node'): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return `${prefix}-${crypto.randomUUID().slice(0, 8)}`;
  }
  return `${prefix}-${Math.random().toString(36).slice(2, 10)}`;
}

export function resolveCanvasColor(color: string | undefined, fallbackHex: string): string {
  if (!color) return fallbackHex;
  const trimmed = color.trim();
  const preset = PRESET_HEX_MAP.get(trimmed);
  if (preset) return preset;
  if (/^#[0-9a-fA-F]{3,8}$/.test(trimmed)) return trimmed;
  return fallbackHex;
}

export function hexToRgba(hex: string, alpha: number): string {
  const clean = hex.replace('#', '');
  let r = 100;
  let g = 116;
  let b = 139;
  if (clean.length === 3) {
    r = parseInt(clean[0] + clean[0], 16);
    g = parseInt(clean[1] + clean[1], 16);
    b = parseInt(clean[2] + clean[2], 16);
  } else if (clean.length >= 6) {
    r = parseInt(clean.slice(0, 2), 16);
    g = parseInt(clean.slice(2, 4), 16);
    b = parseInt(clean.slice(4, 6), 16);
  }
  if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) {
    return `rgba(100, 116, 139, ${alpha})`;
  }
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

export function snapCoord(value: number, enabled: boolean, step = 20): number {
  if (!enabled) return Math.round(value);
  return Math.round(value / step) * step;
}

export function isNoteFileNode(node: CanvasFileNode): boolean {
  if (node.mediaKind === 'note') return true;
  if (node.mediaKind === 'media') return false;
  return node.file.trim().toLowerCase().endsWith('.md');
}

export function detectMediaKindLabel(path: string): 'Imagen' | 'PDF' | 'Audio' | 'Video' | 'Archivo' {
  const lower = path.trim().toLowerCase();
  if (
    lower.startsWith('data:image/') ||
    /\.(png|jpe?g|gif|webp|svg|bmp|avif)(\?|#|$)/.test(lower)
  ) {
    return 'Imagen';
  }
  if (/\.pdf(\?|#|$)/.test(lower)) return 'PDF';
  if (/\.(mp3|wav|ogg|m4a|flac)(\?|#|$)/.test(lower)) return 'Audio';
  if (/\.(mp4|webm|mov)(\?|#|$)/.test(lower)) return 'Video';
  return 'Archivo';
}

export function canRenderImageDirectly(path: string): boolean {
  const clean = path.trim();
  const lower = clean.toLowerCase();
  return (
    (lower.startsWith('https://') ||
      lower.startsWith('http://') ||
      lower.startsWith('data:image/')) &&
    detectMediaKindLabel(clean) === 'Imagen'
  );
}

export function isSafeExternalUrl(url: string): boolean {
  const trimmed = url.trim().toLowerCase();
  return trimmed.startsWith('https://') || trimmed.startsWith('http://');
}

export function parseCanvasJson(raw: string): CanvasJsonData {
  try {
    const parsed = JSON.parse(raw) as {
      nodes?: unknown[];
      edges?: unknown[];
    };
    const rawNodes = Array.isArray(parsed?.nodes) ? parsed.nodes : [];
    const rawEdges = Array.isArray(parsed?.edges) ? parsed.edges : [];

    const nodes: CanvasJsonNode[] = [];
    for (const item of rawNodes) {
      if (!item || typeof item !== 'object') continue;
      const obj = item as Record<string, unknown>;
      const id = typeof obj.id === 'string' && obj.id ? obj.id : createCanvasId('n');
      const x = typeof obj.x === 'number' && Number.isFinite(obj.x) ? obj.x : 0;
      const y = typeof obj.y === 'number' && Number.isFinite(obj.y) ? obj.y : 0;
      const width =
        typeof obj.width === 'number' && Number.isFinite(obj.width)
          ? Math.max(120, obj.width)
          : 260;
      const height =
        typeof obj.height === 'number' && Number.isFinite(obj.height)
          ? Math.max(80, obj.height)
          : 150;
      const color = typeof obj.color === 'string' ? obj.color : undefined;

      if (obj.type === 'text') {
        nodes.push({
          id,
          type: 'text',
          x,
          y,
          width,
          height,
          color,
          text: typeof obj.text === 'string' ? obj.text : '',
        });
      } else if (obj.type === 'file') {
        const file = typeof obj.file === 'string' ? obj.file : '';
        const mediaKind =
          obj.mediaKind === 'note' || obj.mediaKind === 'media'
            ? obj.mediaKind
            : file.toLowerCase().endsWith('.md')
              ? 'note'
              : 'media';
        nodes.push({
          id,
          type: 'file',
          x,
          y,
          width,
          height,
          color,
          file,
          subpath: typeof obj.subpath === 'string' ? obj.subpath : undefined,
          mediaKind,
        });
      } else if (obj.type === 'link') {
        nodes.push({
          id,
          type: 'link',
          x,
          y,
          width,
          height,
          color,
          url: typeof obj.url === 'string' ? obj.url : '',
        });
      } else if (obj.type === 'group') {
        nodes.push({
          id,
          type: 'group',
          x,
          y,
          width: Math.max(200, width),
          height: Math.max(140, height),
          color,
          label: typeof obj.label === 'string' ? obj.label : 'Grupo',
          background: typeof obj.background === 'string' ? obj.background : undefined,
          backgroundStyle:
            obj.backgroundStyle === 'cover' ||
            obj.backgroundStyle === 'ratio' ||
            obj.backgroundStyle === 'repeat'
              ? obj.backgroundStyle
              : 'cover',
        });
      }
    }

    const nodeIds = new Set(nodes.map((n) => n.id));
    const edges: CanvasJsonEdge[] = [];
    for (const item of rawEdges) {
      if (!item || typeof item !== 'object') continue;
      const obj = item as Record<string, unknown>;
      const fromNode = typeof obj.fromNode === 'string' ? obj.fromNode : '';
      const toNode = typeof obj.toNode === 'string' ? obj.toNode : '';
      if (!nodeIds.has(fromNode) || !nodeIds.has(toNode)) continue;
      edges.push({
        id: typeof obj.id === 'string' && obj.id ? obj.id : createCanvasId('e'),
        fromNode,
        fromSide: isValidSide(obj.fromSide) ? obj.fromSide : undefined,
        fromEnd: isValidEnd(obj.fromEnd) ? obj.fromEnd : 'none',
        toNode,
        toSide: isValidSide(obj.toSide) ? obj.toSide : undefined,
        toEnd: isValidEnd(obj.toEnd) ? obj.toEnd : 'arrow',
        color: typeof obj.color === 'string' ? obj.color : undefined,
        label: typeof obj.label === 'string' ? obj.label : undefined,
      });
    }

    return { nodes, edges };
  } catch {
    return { nodes: [], edges: [] };
  }
}

function isValidSide(v: unknown): v is CanvasSide {
  return v === 'top' || v === 'right' || v === 'bottom' || v === 'left';
}

function isValidEnd(v: unknown): v is CanvasEdgeEnd {
  return v === 'none' || v === 'arrow';
}

export function serializeCanvasJson(data: CanvasJsonData): string {
  // Groups before foreground cards preserves natural z-order across JSON Canvas tools.
  const sortedNodes = [...data.nodes].sort((a, b) => {
    if (a.type === 'group' && b.type !== 'group') return -1;
    if (a.type !== 'group' && b.type === 'group') return 1;
    return 0;
  });
  return `${JSON.stringify({ nodes: sortedNodes, edges: data.edges }, null, 2)}\n`;
}

export function getSideAnchor(
  node: CanvasJsonNode,
  side: CanvasSide,
): { x: number; y: number; nx: number; ny: number } {
  switch (side) {
    case 'top':
      return { x: node.x + node.width / 2, y: node.y, nx: 0, ny: -1 };
    case 'right':
      return { x: node.x + node.width, y: node.y + node.height / 2, nx: 1, ny: 0 };
    case 'bottom':
      return { x: node.x + node.width / 2, y: node.y + node.height, nx: 0, ny: 1 };
    case 'left':
      return { x: node.x, y: node.y + node.height / 2, nx: -1, ny: 0 };
  }
}

export function chooseBestSides(
  fromNode: CanvasJsonNode,
  toNode: CanvasJsonNode,
): { fromSide: CanvasSide; toSide: CanvasSide } {
  const dx = toNode.x + toNode.width / 2 - (fromNode.x + fromNode.width / 2);
  const dy = toNode.y + toNode.height / 2 - (fromNode.y + fromNode.height / 2);
  if (Math.abs(dx) >= Math.abs(dy)) {
    return dx >= 0
      ? { fromSide: 'right', toSide: 'left' }
      : { fromSide: 'left', toSide: 'right' };
  }
  return dy >= 0
    ? { fromSide: 'bottom', toSide: 'top' }
    : { fromSide: 'top', toSide: 'bottom' };
}

export interface EdgeGeometry {
  sx: number;
  sy: number;
  c1x: number;
  c1y: number;
  c2x: number;
  c2y: number;
  ex: number;
  ey: number;
  mx: number;
  my: number;
  pathD: string;
}

export function computeEdgeGeometry(
  fromNode: CanvasJsonNode,
  toNode: CanvasJsonNode,
  fromSideOpt?: CanvasSide,
  toSideOpt?: CanvasSide,
): EdgeGeometry {
  const auto = chooseBestSides(fromNode, toNode);
  const fromSide = fromSideOpt ?? auto.fromSide;
  const toSide = toSideOpt ?? auto.toSide;
  const start = getSideAnchor(fromNode, fromSide);
  const end = getSideAnchor(toNode, toSide);
  const dist = Math.hypot(end.x - start.x, end.y - start.y);
  const curve = Math.max(44, Math.min(180, dist * 0.38));
  const c1x = start.x + start.nx * curve;
  const c1y = start.y + start.ny * curve;
  const c2x = end.x + end.nx * curve;
  const c2y = end.y + end.ny * curve;

  // Midpoint t=0.5 on cubic Bezier
  const mx = 0.125 * start.x + 0.375 * c1x + 0.375 * c2x + 0.125 * end.x;
  const my = 0.125 * start.y + 0.375 * c1y + 0.375 * c2y + 0.125 * end.y;
  const pathD = `M ${start.x.toFixed(1)} ${start.y.toFixed(1)} C ${c1x.toFixed(1)} ${c1y.toFixed(1)}, ${c2x.toFixed(1)} ${c2y.toFixed(1)}, ${end.x.toFixed(1)} ${end.y.toFixed(1)}`;

  return {
    sx: start.x,
    sy: start.y,
    c1x,
    c1y,
    c2x,
    c2y,
    ex: end.x,
    ey: end.y,
    mx,
    my,
    pathD,
  };
}

export type CanvasAlignAction =
  | 'left'
  | 'center-h'
  | 'right'
  | 'top'
  | 'middle-v'
  | 'bottom'
  | 'distribute-h'
  | 'distribute-v';

export function alignNodesBySelection(
  nodes: CanvasJsonNode[],
  selectedIds: Set<string>,
  action: CanvasAlignAction,
  snapEnabled: boolean,
): CanvasJsonNode[] {
  const targets = nodes.filter((n) => selectedIds.has(n.id));
  if (targets.length < 2) return nodes;

  const minX = Math.min(...targets.map((n) => n.x));
  const maxRight = Math.max(...targets.map((n) => n.x + n.width));
  const minY = Math.min(...targets.map((n) => n.y));
  const maxBottom = Math.max(...targets.map((n) => n.y + n.height));
  const centerX = (minX + maxRight) / 2;
  const centerY = (minY + maxBottom) / 2;

  const updates = new Map<string, { x: number; y: number }>();

  if (action === 'distribute-h' && targets.length >= 3) {
    const ordered = [...targets].sort((a, b) => a.x - b.x);
    const totalWidths = ordered.reduce((acc, n) => acc + n.width, 0);
    const totalSpan =
      ordered[ordered.length - 1].x + ordered[ordered.length - 1].width - ordered[0].x;
    const gap = (totalSpan - totalWidths) / (ordered.length - 1);
    let cursor = ordered[0].x;
    for (const item of ordered) {
      updates.set(item.id, { x: snapCoord(cursor, snapEnabled), y: item.y });
      cursor += item.width + gap;
    }
  } else if (action === 'distribute-v' && targets.length >= 3) {
    const ordered = [...targets].sort((a, b) => a.y - b.y);
    const totalHeights = ordered.reduce((acc, n) => acc + n.height, 0);
    const totalSpan =
      ordered[ordered.length - 1].y + ordered[ordered.length - 1].height - ordered[0].y;
    const gap = (totalSpan - totalHeights) / (ordered.length - 1);
    let cursor = ordered[0].y;
    for (const item of ordered) {
      updates.set(item.id, { x: item.x, y: snapCoord(cursor, snapEnabled) });
      cursor += item.height + gap;
    }
  } else {
    for (const n of targets) {
      let nx = n.x;
      let ny = n.y;
      if (action === 'left') nx = minX;
      else if (action === 'center-h') nx = centerX - n.width / 2;
      else if (action === 'right') nx = maxRight - n.width;
      else if (action === 'top') ny = minY;
      else if (action === 'middle-v') ny = centerY - n.height / 2;
      else if (action === 'bottom') ny = maxBottom - n.height;
      updates.set(n.id, {
        x: snapCoord(nx, snapEnabled),
        y: snapCoord(ny, snapEnabled),
      });
    }
  }

  return nodes.map((n) => {
    const u = updates.get(n.id);
    return u ? { ...n, x: u.x, y: u.y } : n;
  });
}

export function arrangeNodesInGrid(
  nodes: CanvasJsonNode[],
  selectedIds: Set<string>,
  snapEnabled: boolean,
): CanvasJsonNode[] {
  const candidates =
    selectedIds.size >= 2
      ? nodes.filter((n) => selectedIds.has(n.id) && n.type !== 'group')
      : nodes.filter((n) => n.type !== 'group');
  if (candidates.length === 0) return nodes;

  const sorted = [...candidates].sort((a, b) => a.y - b.y || a.x - b.x);
  const cols = Math.max(1, Math.ceil(Math.sqrt(sorted.length)));
  const maxW = Math.max(...sorted.map((n) => n.width), 260);
  const maxH = Math.max(...sorted.map((n) => n.height), 150);
  const originX = snapCoord(Math.min(...sorted.map((n) => n.x)), snapEnabled);
  const originY = snapCoord(Math.min(...sorted.map((n) => n.y)), snapEnabled);
  const gapX = 40;
  const gapY = 40;

  const updates = new Map<string, { x: number; y: number }>();
  sorted.forEach((node, index) => {
    const col = index % cols;
    const row = Math.floor(index / cols);
    updates.set(node.id, {
      x: snapCoord(originX + col * (maxW + gapX), snapEnabled),
      y: snapCoord(originY + row * (maxH + gapY), snapEnabled),
    });
  });

  return nodes.map((n) => {
    const u = updates.get(n.id);
    return u ? { ...n, x: u.x, y: u.y } : n;
  });
}

export function nodeDisplayHeader(node: CanvasJsonNode): string {
  switch (node.type) {
    case 'text':
      return 'TEXTO MD';
    case 'file':
      return isNoteFileNode(node) ? 'NOTA BÓVEDA' : `MULTIMEDIA · ${detectMediaKindLabel(node.file).toUpperCase()}`;
    case 'link':
      return 'PÁGINA WEB';
    case 'group':
      return node.label?.trim() || 'GRUPO';
  }
}

export function nodeDisplayBodyLines(
  node: CanvasJsonNode,
  noteSnippets: Record<string, string>,
): string[] {
  switch (node.type) {
    case 'text':
      return node.text.trim() ? node.text.split(/\r?\n/) : ['(Tarjeta de texto vacía)'];
    case 'file': {
      if (isNoteFileNode(node)) {
        const lines: string[] = [node.file || '(Sin nota asignada)'];
        if (node.subpath) lines.push(`Acotación: ${node.subpath}`);
        const snippet = noteSnippets[node.file];
        if (snippet) lines.push(...snippet.split(/\r?\n/).slice(0, 6));
        return lines;
      }
      return [
        node.file || '(Sin recurso multimedia)',
        `Tipo: ${detectMediaKindLabel(node.file)}`,
      ];
    }
    case 'link':
      return [node.url || 'https://...'];
    case 'group':
      return [];
  }
}

export async function renderCanvasToPngBlob(
  data: CanvasJsonData,
  options: {
    theme: 'light' | 'dark';
    privacyMode: boolean;
    noteSnippets?: Record<string, string>;
  },
): Promise<Blob | null> {
  if (typeof document === 'undefined') return null;
  const { theme, privacyMode, noteSnippets = {} } = options;
  const isDark = theme === 'dark';

  const pad = 72;
  let minX = 0;
  let minY = 0;
  let maxX = 800;
  let maxY = 500;
  if (data.nodes.length > 0) {
    minX = Math.min(...data.nodes.map((n) => n.x)) - pad;
    minY = Math.min(...data.nodes.map((n) => n.y)) - pad;
    maxX = Math.max(...data.nodes.map((n) => n.x + n.width)) + pad;
    maxY = Math.max(...data.nodes.map((n) => n.y + n.height)) + pad;
  }

  const worldW = Math.max(400, maxX - minX);
  const worldH = Math.max(300, maxY - minY);
  // Cap canvas dimensions cleanly for low-RAM devices while maintaining high sharpness.
  const maxPx = 3840;
  const scale = Math.min(2, maxPx / worldW, maxPx / worldH);

  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(worldW * scale));
  canvas.height = Math.max(1, Math.round(worldH * scale));
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;

  ctx.save();
  ctx.scale(scale, scale);
  ctx.translate(-minX, -minY);

  // Background + subtle dot grid
  ctx.fillStyle = isDark ? '#0f1419' : '#f8fafc';
  ctx.fillRect(minX, minY, worldW, worldH);

  ctx.fillStyle = isDark ? 'rgba(148, 163, 184, 0.16)' : 'rgba(100, 116, 139, 0.18)';
  const step = 20;
  const startGridX = Math.floor(minX / step) * step;
  const startGridY = Math.floor(minY / step) * step;
  for (let gx = startGridX; gx <= maxX; gx += step) {
    for (let gy = startGridY; gy <= maxY; gy += step) {
      ctx.fillRect(gx - 0.8, gy - 0.8, 1.6, 1.6);
    }
  }

  const defaultNodeStroke = isDark ? '#475569' : '#94a3b8';
  const cardBg = isDark ? '#171f2b' : '#ffffff';
  const textPrimary = isDark ? '#e2e8f0' : '#0f172a';
  const textMuted = isDark ? '#94a3b8' : '#475569';

  const groups = data.nodes.filter((n) => n.type === 'group');
  const cards = data.nodes.filter((n) => n.type !== 'group');

  // 1. Groups
  for (const grp of groups) {
    const stroke = resolveCanvasColor(grp.color, defaultNodeStroke);
    ctx.fillStyle = hexToRgba(stroke, isDark ? 0.11 : 0.08);
    roundRectPath(ctx, grp.x, grp.y, grp.width, grp.height, 14);
    ctx.fill();
    ctx.strokeStyle = hexToRgba(stroke, 0.72);
    ctx.lineWidth = 2;
    ctx.setLineDash([6, 4]);
    ctx.stroke();
    ctx.setLineDash([]);

    const label = grp.label?.trim() || 'Grupo';
    ctx.font = '600 14px Inter, system-ui, sans-serif';
    if (privacyMode) {
      drawRedactedBar(ctx, grp.x + 14, grp.y + 12, Math.min(180, grp.width - 28), 14, stroke);
    } else {
      ctx.fillStyle = stroke;
      ctx.fillText(label, grp.x + 14, grp.y + 24, grp.width - 28);
    }
  }

  // 2. Edges
  const nodeById = new Map(data.nodes.map((n) => [n.id, n]));
  for (const edge of data.edges) {
    const fromNode = nodeById.get(edge.fromNode);
    const toNode = nodeById.get(edge.toNode);
    if (!fromNode || !toNode) continue;
    const geom = computeEdgeGeometry(fromNode, toNode, edge.fromSide, edge.toSide);
    const edgeColor = resolveCanvasColor(edge.color, isDark ? '#64748b' : '#475569');

    ctx.strokeStyle = edgeColor;
    ctx.lineWidth = 2.2;
    ctx.beginPath();
    ctx.moveTo(geom.sx, geom.sy);
    ctx.bezierCurveTo(geom.c1x, geom.c1y, geom.c2x, geom.c2y, geom.ex, geom.ey);
    ctx.stroke();

    if (edge.toEnd !== 'none') {
      drawArrowHead(ctx, geom.c2x, geom.c2y, geom.ex, geom.ey, edgeColor);
    }
    if (edge.fromEnd === 'arrow') {
      drawArrowHead(ctx, geom.c1x, geom.c1y, geom.sx, geom.sy, edgeColor);
    }

    if (edge.label?.trim()) {
      ctx.font = '600 11.5px Inter, system-ui, sans-serif';
      const labelText = edge.label.trim();
      const boxW = Math.min(200, Math.max(48, ctx.measureText(labelText).width + 18));
      const boxH = 22;
      ctx.fillStyle = isDark ? '#1e293b' : '#ffffff';
      roundRectPath(ctx, geom.mx - boxW / 2, geom.my - boxH / 2, boxW, boxH, 999);
      ctx.fill();
      ctx.strokeStyle = edgeColor;
      ctx.lineWidth = 1.2;
      ctx.stroke();
      if (privacyMode) {
        drawRedactedBar(ctx, geom.mx - boxW / 2 + 8, geom.my - 5, boxW - 16, 10, textMuted);
      } else {
        ctx.fillStyle = textPrimary;
        ctx.textAlign = 'center';
        ctx.fillText(labelText, geom.mx, geom.my + 4, boxW - 12);
        ctx.textAlign = 'left';
      }
    }
  }

  // 3. Foreground Cards
  for (const node of cards) {
    const stroke = resolveCanvasColor(node.color, defaultNodeStroke);
    ctx.fillStyle = cardBg;
    roundRectPath(ctx, node.x, node.y, node.width, node.height, 10);
    ctx.fill();

    // Subtle colored top tint band
    ctx.fillStyle = hexToRgba(stroke, isDark ? 0.16 : 0.11);
    roundRectPath(ctx, node.x, node.y, node.width, 28, 10);
    ctx.fill();

    ctx.strokeStyle = stroke;
    ctx.lineWidth = 1.8;
    roundRectPath(ctx, node.x, node.y, node.width, node.height, 10);
    ctx.stroke();

    ctx.font = '700 10.5px Inter, system-ui, sans-serif';
    ctx.fillStyle = stroke;
    ctx.fillText(nodeDisplayHeader(node), node.x + 12, node.y + 18, node.width - 24);

    const lines = nodeDisplayBodyLines(node, noteSnippets);
    const wrapped = wrapCanvasLines(ctx, lines, node.width - 24, '12.5px Inter, system-ui, sans-serif');
    ctx.font = '12.5px Inter, system-ui, sans-serif';
    let cursorY = node.y + 48;
    const maxYLine = node.y + node.height - 14;
    for (const line of wrapped) {
      if (cursorY > maxYLine) break;
      if (privacyMode) {
        const barW = Math.max(40, Math.min(node.width - 28, line.length * 6.5));
        drawRedactedBar(ctx, node.x + 12, cursorY - 10, barW, 10, textMuted);
      } else {
        ctx.fillStyle = line.startsWith('Acotación:') ? stroke : textPrimary;
        ctx.fillText(line, node.x + 12, cursorY, node.width - 24);
      }
      cursorY += 17;
    }
  }

  // Watermark footer in privacy mode
  if (privacyMode) {
    ctx.font = '600 11px Inter, system-ui, sans-serif';
    ctx.fillStyle = textMuted;
    ctx.fillText('ROMSIDIAN Canvas · Modo Privacidad activo', minX + 16, maxY - 14);
  }

  ctx.restore();

  return await new Promise<Blob | null>((resolve) => {
    canvas.toBlob((blob) => resolve(blob), 'image/png');
  });
}

function roundRectPath(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  const rad = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rad, y);
  ctx.lineTo(x + w - rad, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + rad);
  ctx.lineTo(x + w, y + h - rad);
  ctx.quadraticCurveTo(x + w, y + h, x + w - rad, y + h);
  ctx.lineTo(x + rad, y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - rad);
  ctx.lineTo(x, y + rad);
  ctx.quadraticCurveTo(x, y, x + rad, y);
  ctx.closePath();
}

function drawArrowHead(
  ctx: CanvasRenderingContext2D,
  fromX: number,
  fromY: number,
  tipX: number,
  tipY: number,
  color: string,
) {
  const angle = Math.atan2(tipY - fromY, tipX - fromX);
  const len = 11;
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.moveTo(tipX, tipY);
  ctx.lineTo(tipX - len * Math.cos(angle - Math.PI / 6), tipY - len * Math.sin(angle - Math.PI / 6));
  ctx.lineTo(tipX - len * Math.cos(angle + Math.PI / 6), tipY - len * Math.sin(angle + Math.PI / 6));
  ctx.closePath();
  ctx.fill();
}

function drawRedactedBar(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  baseColor: string,
) {
  ctx.fillStyle = hexToRgba(baseColor, 0.34);
  roundRectPath(ctx, x, y, Math.max(24, w), h, 4);
  ctx.fill();
}

function wrapCanvasLines(
  ctx: CanvasRenderingContext2D,
  lines: string[],
  maxWidth: number,
  font: string,
): string[] {
  ctx.font = font;
  const out: string[] = [];
  for (const rawLine of lines) {
    const trimmed = rawLine.trim();
    if (!trimmed) {
      out.push('');
      continue;
    }
    const words = trimmed.split(/\s+/);
    let current = '';
    for (const word of words) {
      const candidate = current ? `${current} ${word}` : word;
      if (ctx.measureText(candidate).width <= maxWidth) {
        current = candidate;
      } else {
        if (current) out.push(current);
        current = word;
      }
    }
    if (current) out.push(current);
  }
  return out;
}
