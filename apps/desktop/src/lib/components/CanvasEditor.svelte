<script lang="ts">
  import { onDestroy } from 'svelte';
  import { readNote } from '../api';
  import {
    alignNodesBySelection,
    arrangeNodesInGrid,
    CANVAS_COLOR_OPTIONS,
    canRenderImageDirectly,
    computeEdgeGeometry,
    createCanvasId,
    detectMediaKindLabel,
    isNoteFileNode,
    isSafeExternalUrl,
    nodeDisplayBodyLines,
    parseCanvasJson,
    renderCanvasToPngBlob,
    resolveCanvasColor,
    serializeCanvasJson,
    snapCoord,
    type CanvasAlignAction,
  } from '../canvasUtils';
  import type {
    CanvasDocument,
    CanvasJsonData,
    CanvasJsonEdge,
    CanvasJsonNode,
    CanvasSide,
    NoteDetail,
    NoteSummary,
  } from '../types';

  interface Props {
    canvas: CanvasDocument;
    notes: NoteSummary[];
    theme: 'light' | 'dark';
    saving?: boolean;
    onSave: (relativePath: string, content: string) => Promise<void> | void;
    onOpenNote: (relativePath: string, subpath?: string) => void;
    onConvertTextToNote: (suggestedTitle: string, text: string) => Promise<string | null>;
    onClose: () => void;
  }

  let {
    canvas,
    notes,
    theme,
    saving = false,
    onSave,
    onOpenNote,
    onConvertTextToNote,
    onClose,
  }: Props = $props();

  let viewportEl = $state<HTMLDivElement | null>(null);
  let loadedCanvasPath = $state('');
  let data = $state<CanvasJsonData>({ nodes: [], edges: [] });
  let dirty = $state(false);
  let exporting = $state(false);
  let exportStatus = $state<string | null>(null);

  let panX = $state(90);
  let panY = $state(90);
  let zoom = $state(1);
  let snapToGrid = $state(true);
  let privacyMode = $state(false);

  let selectedNodeIds = $state<Set<string>>(new Set());
  let selectedEdgeId = $state<string | null>(null);
  let editingTextNodeId = $state<string | null>(null);
  let pendingConnectFrom = $state<{ nodeId: string; side?: CanvasSide } | null>(null);

  let interaction = $state<
    | { kind: 'none' }
    | { kind: 'pan'; startX: number; startY: number; origPanX: number; origPanY: number }
    | {
        kind: 'drag-nodes';
        startX: number;
        startY: number;
        initialPositions: Map<string, { x: number; y: number }>;
      }
    | {
        kind: 'resize-node';
        nodeId: string;
        startX: number;
        startY: number;
        origW: number;
        origH: number;
      }
    | {
        kind: 'connect-drag';
        fromNodeId: string;
        fromSide: CanvasSide;
        sx: number;
        sy: number;
        cx: number;
        cy: number;
      }
  >({ kind: 'none' });

  // Cache metadata/snippets of referenced .md notes for card previews & heading/block narrowing
  let noteDetailsCache = $state<Record<string, NoteDetail>>({});
  let autosaveTimer: ReturnType<typeof setTimeout> | null = null;

  onDestroy(() => {
    if (autosaveTimer) {
      clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
    if (dirty && loadedCanvasPath) {
      void onSave(loadedCanvasPath, serializeCanvasJson(data));
    }
  });

  $effect(() => {
    if (canvas.relative_path !== loadedCanvasPath) {
      if (autosaveTimer) {
        clearTimeout(autosaveTimer);
        autosaveTimer = null;
      }
      loadedCanvasPath = canvas.relative_path;
      data = parseCanvasJson(canvas.content);
      selectedNodeIds = new Set();
      selectedEdgeId = null;
      editingTextNodeId = null;
      pendingConnectFrom = null;
      dirty = false;
      queueMicrotask(() => fitToView());
    }
  });

  $effect(() => {
    const notePaths = new Set<string>();
    for (const node of data.nodes) {
      if (node.type === 'file' && isNoteFileNode(node) && node.file.trim()) {
        notePaths.add(node.file.trim());
      }
    }
    for (const relPath of notePaths) {
      if (!noteDetailsCache[relPath]) {
        void loadNoteMeta(relPath);
      }
    }
  });

  async function loadNoteMeta(relPath: string) {
    try {
      const detail = await readNote(relPath);
      noteDetailsCache = { ...noteDetailsCache, [relPath]: detail };
    } catch {
      // Ignore missing files silently in canvas preview
    }
  }

  let primarySelectedNode = $derived.by<CanvasJsonNode | null>(() => {
    if (selectedNodeIds.size !== 1) return null;
    const onlyId = Array.from(selectedNodeIds)[0];
    return data.nodes.find((n) => n.id === onlyId) ?? null;
  });

  let selectedEdge = $derived.by<CanvasJsonEdge | null>(() => {
    if (!selectedEdgeId) return null;
    return data.edges.find((e) => e.id === selectedEdgeId) ?? null;
  });

  let noteSnippets = $derived.by<Record<string, string>>(() => {
    const out: Record<string, string> = {};
    for (const node of data.nodes) {
      if (node.type === 'file' && isNoteFileNode(node) && node.file) {
        const detail = noteDetailsCache[node.file];
        if (!detail) continue;
        out[node.file] = extractScopedNoteSnippet(detail.content, node.subpath);
      }
    }
    return out;
  });

  let sortedNodesForRender = $derived.by(() => {
    // Groups render first so foreground cards stay clickable on top
    return [...data.nodes].sort((a, b) => {
      if (a.type === 'group' && b.type !== 'group') return -1;
      if (a.type !== 'group' && b.type === 'group') return 1;
      return 0;
    });
  });

  let computedEdges = $derived.by(() => {
    const nodeById = new Map(data.nodes.map((n) => [n.id, n]));
    return data.edges
      .map((edge) => {
        const fromNode = nodeById.get(edge.fromNode);
        const toNode = nodeById.get(edge.toNode);
        if (!fromNode || !toNode) return null;
        return {
          edge,
          fromNode,
          toNode,
          geom: computeEdgeGeometry(fromNode, toNode, edge.fromSide, edge.toSide),
          stroke: resolveCanvasColor(
            edge.color,
            theme === 'dark' ? '#7dd3fc' : '#0284c7',
          ),
        };
      })
      .filter((item): item is NonNullable<typeof item> => item !== null);
  });

  function extractScopedNoteSnippet(markdown: string, subpath?: string): string {
    const lines = markdown.split(/\r?\n/);
    if (!subpath?.trim()) {
      return lines.filter((l) => !l.startsWith('---')).slice(0, 6).join('\n');
    }
    const clean = subpath.trim().replace(/^#/, '');
    if (clean.startsWith('^')) {
      const blockMarker = clean.toLowerCase();
      const hit = lines.find((line) => line.toLowerCase().includes(blockMarker));
      return hit ?? `(Bloque #${clean} no encontrado)`;
    }
    const targetHeading = clean.toLowerCase();
    const startIdx = lines.findIndex((line) => {
      const match = /^#{1,6}\s+(.+)$/.exec(line.trim());
      return match ? match[1].trim().toLowerCase() === targetHeading : false;
    });
    if (startIdx < 0) return `(Encabezado #${clean} no encontrado)`;
    const sectionLines: string[] = [];
    for (let i = startIdx; i < Math.min(lines.length, startIdx + 8); i += 1) {
      if (i > startIdx && /^#{1,6}\s+/.test(lines[i].trim())) break;
      sectionLines.push(lines[i]);
    }
    return sectionLines.join('\n');
  }

  function extractHeadingsFromContent(markdown?: string): string[] {
    if (!markdown) return [];
    const headings: string[] = [];
    for (const line of markdown.split(/\r?\n/)) {
      const match = /^#{1,6}\s+(.+)$/.exec(line.trim());
      if (match) headings.push(match[1].trim());
    }
    return headings;
  }

  function extractBlocksFromContent(markdown?: string): { id: string; text: string }[] {
    if (!markdown) return [];
    const blocks: { id: string; text: string }[] = [];
    for (const line of markdown.split(/\r?\n/)) {
      const match = /\^([a-zA-Z0-9_-]+)\s*$/.exec(line.trim());
      if (match) blocks.push({ id: match[1], text: line.trim() });
    }
    return blocks;
  }

  function scheduleSave(immediate = false) {
    dirty = true;
    if (autosaveTimer) clearTimeout(autosaveTimer);
    if (immediate) {
      void flushSave();
      return;
    }
    autosaveTimer = setTimeout(() => {
      void flushSave();
    }, 850);
  }

  async function flushSave() {
    if (autosaveTimer) {
      clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
    const targetPath = loadedCanvasPath || canvas.relative_path;
    const serialized = serializeCanvasJson(data);
    await onSave(targetPath, serialized);
    dirty = false;
  }

  function screenToWorld(clientX: number, clientY: number): { x: number; y: number } {
    if (!viewportEl) return { x: 0, y: 0 };
    const rect = viewportEl.getBoundingClientRect();
    return {
      x: (clientX - rect.left - panX) / zoom,
      y: (clientY - rect.top - panY) / zoom,
    };
  }

  function fitToView() {
    if (!viewportEl) return;
    const rect = viewportEl.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    if (data.nodes.length === 0) {
      zoom = 1;
      panX = Math.round(rect.width / 2 - 200);
      panY = Math.round(rect.height / 2 - 140);
      return;
    }
    const minX = Math.min(...data.nodes.map((n) => n.x));
    const minY = Math.min(...data.nodes.map((n) => n.y));
    const maxX = Math.max(...data.nodes.map((n) => n.x + n.width));
    const maxY = Math.max(...data.nodes.map((n) => n.y + n.height));
    const boxW = Math.max(240, maxX - minX);
    const boxH = Math.max(180, maxY - minY);
    const pad = 70;
    const nextZoom = Math.max(
      0.28,
      Math.min(1.25, Math.min((rect.width - pad * 2) / boxW, (rect.height - pad * 2) / boxH)),
    );
    zoom = Number(nextZoom.toFixed(2));
    panX = Math.round(rect.width / 2 - ((minX + maxX) / 2) * zoom);
    panY = Math.round(rect.height / 2 - ((minY + maxY) / 2) * zoom);
  }

  function focusOnNode(nodeId: string) {
    const node = data.nodes.find((n) => n.id === nodeId);
    if (!node || !viewportEl) return;
    const rect = viewportEl.getBoundingClientRect();
    const targetZoom = Math.max(0.85, Math.min(1.25, zoom));
    zoom = targetZoom;
    panX = Math.round(rect.width / 2 - (node.x + node.width / 2) * targetZoom);
    panY = Math.round(rect.height / 2 - (node.y + node.height / 2) * targetZoom);
    selectedEdgeId = null;
    selectedNodeIds = new Set([node.id]);
  }

  function deleteNode(id: string) {
    data = {
      nodes: data.nodes.filter((n) => n.id !== id),
      edges: data.edges.filter((e) => e.fromNode !== id && e.toNode !== id),
    };
    if (selectedNodeIds.has(id)) {
      const next = new Set(selectedNodeIds);
      next.delete(id);
      selectedNodeIds = next;
    }
    if (editingTextNodeId === id) editingTextNodeId = null;
    scheduleSave();
  }

  function findFreeCardPosition(
    baseX: number,
    baseY: number,
    width: number,
    height: number,
    isGroup: boolean,
  ): { x: number; y: number } {
    const pad = 24;
    const obstacles = data.nodes.filter((n) => (isGroup ? n.type === 'group' : n.type !== 'group'));
    const overlaps = (candX: number, candY: number) =>
      obstacles.some(
        (n) =>
          candX < n.x + n.width + pad &&
          candX + width + pad > n.x &&
          candY < n.y + n.height + pad &&
          candY + height + pad > n.y,
      );

    const initialX = snapCoord(baseX, snapToGrid);
    const initialY = snapCoord(baseY, snapToGrid);
    if (!overlaps(initialX, initialY)) {
      return { x: initialX, y: initialY };
    }

    const deltas: [number, number][] = [
      [30, 30],
      [60, 60],
      [width + 24, 0],
      [0, height + 24],
      [-(width + 24), 0],
      [0, -(height + 24)],
      [width + 24, height + 24],
      [-(width + 24), height + 24],
      [90, 90],
      [120, 120],
      [150, 150],
    ];
    for (const [dx, dy] of deltas) {
      const candX = snapCoord(baseX + dx, snapToGrid);
      const candY = snapCoord(baseY + dy, snapToGrid);
      if (!overlaps(candX, candY)) {
        return { x: candX, y: candY };
      }
    }
    const count = data.nodes.length;
    return {
      x: snapCoord(baseX + (count % 5) * 30, snapToGrid),
      y: snapCoord(baseY + (count % 5) * 30, snapToGrid),
    };
  }

  function addNewCard(kind: 'text' | 'note' | 'media' | 'link' | 'group') {
    const rect = viewportEl?.getBoundingClientRect();
    const cx = rect ? (rect.width / 2 - panX) / zoom : 200;
    const cy = rect ? (rect.height / 2 - panY) / zoom : 140;
    const dims =
      kind === 'group'
        ? { w: 440, h: 300 }
        : kind === 'link'
          ? { w: 280, h: 130 }
          : kind === 'text'
            ? { w: 280, h: 160 }
            : { w: 280, h: 170 };
    const { x, y } = findFreeCardPosition(
      cx - dims.w / 2,
      cy - dims.h / 2,
      dims.w,
      dims.h,
      kind === 'group',
    );

    let newNode: CanvasJsonNode;
    if (kind === 'text') {
      newNode = {
        id: createCanvasId('txt'),
        type: 'text',
        x,
        y,
        width: dims.w,
        height: dims.h,
        color: '4',
        text: '# Nueva idea\n\nEscribe aquí en **Markdown**.',
      };
    } else if (kind === 'note') {
      const defaultNote = notes[0]?.relative_path ?? 'Bienvenida.md';
      newNode = {
        id: createCanvasId('note'),
        type: 'file',
        x,
        y,
        width: dims.w,
        height: dims.h,
        color: '6',
        file: defaultNote,
        mediaKind: 'note',
      };
    } else if (kind === 'media') {
      newNode = {
        id: createCanvasId('media'),
        type: 'file',
        x,
        y,
        width: dims.w,
        height: dims.h,
        color: '3',
        file: 'assets/recurso.png',
        mediaKind: 'media',
      };
    } else if (kind === 'link') {
      newNode = {
        id: createCanvasId('web'),
        type: 'link',
        x,
        y,
        width: dims.w,
        height: dims.h,
        color: '2',
        url: 'https://jsoncanvas.org/',
      };
    } else {
      newNode = {
        id: createCanvasId('grp'),
        type: 'group',
        x,
        y,
        width: dims.w,
        height: dims.h,
        color: '5',
        label: 'Nuevo grupo',
        backgroundStyle: 'cover',
      };
    }

    data = { ...data, nodes: [...data.nodes, newNode] };
    selectedEdgeId = null;
    selectedNodeIds = new Set([newNode.id]);
    if (kind === 'text') {
      editingTextNodeId = newNode.id;
    }
    focusOnNode(newNode.id);
    scheduleSave();
  }

  function updateNode(nodeId: string, updater: (prev: CanvasJsonNode) => CanvasJsonNode) {
    data = {
      ...data,
      nodes: data.nodes.map((n) => (n.id === nodeId ? updater(n) : n)),
    };
    scheduleSave();
  }

  function updateEdge(edgeId: string, updater: (prev: CanvasJsonEdge) => CanvasJsonEdge) {
    data = {
      ...data,
      edges: data.edges.map((e) => (e.id === edgeId ? updater(e) : e)),
    };
    scheduleSave();
  }

  function deleteSelection() {
    if (selectedEdgeId) {
      data = { ...data, edges: data.edges.filter((e) => e.id !== selectedEdgeId) };
      selectedEdgeId = null;
      scheduleSave();
      return;
    }
    if (selectedNodeIds.size === 0) return;
    const doomed = selectedNodeIds;
    data = {
      nodes: data.nodes.filter((n) => !doomed.has(n.id)),
      edges: data.edges.filter((e) => !doomed.has(e.fromNode) && !doomed.has(e.toNode)),
    };
    selectedNodeIds = new Set();
    editingTextNodeId = null;
    scheduleSave();
  }

  function handleAlign(action: CanvasAlignAction) {
    if (selectedNodeIds.size < 2) return;
    data = {
      ...data,
      nodes: alignNodesBySelection(data.nodes, selectedNodeIds, action, snapToGrid),
    };
    scheduleSave();
  }

  function handleArrangeGrid() {
    data = {
      ...data,
      nodes: arrangeNodesInGrid(data.nodes, selectedNodeIds, snapToGrid),
    };
    scheduleSave();
  }

  function createEdgeBetween(
    fromNodeId: string,
    toNodeId: string,
    fromSide?: CanvasSide,
    toSide?: CanvasSide,
  ) {
    if (fromNodeId === toNodeId) return;
    const newEdge: CanvasJsonEdge = {
      id: createCanvasId('edge'),
      fromNode: fromNodeId,
      fromSide,
      fromEnd: 'none',
      toNode: toNodeId,
      toSide,
      toEnd: 'arrow',
      label: '',
    };
    data = { ...data, edges: [...data.edges, newEdge] };
    selectedNodeIds = new Set();
    selectedEdgeId = newEdge.id;
    scheduleSave();
  }

  async function handleConvertCardToMd(node: CanvasJsonNode) {
    if (node.type !== 'text') return;
    const firstLine = node.text
      .split(/\r?\n/)
      .find((line) => line.trim().length > 0)
      ?.replace(/^#+\s*/, '')
      .trim();
    const suggested = firstLine ? firstLine.slice(0, 48) : `Nota desde Canvas ${node.id.slice(-4)}`;
    const createdPath = await onConvertTextToNote(suggested, node.text);
    if (!createdPath) return;
    updateNode(node.id, (prev) =>
      prev.type === 'text'
        ? {
            id: prev.id,
            type: 'file',
            x: prev.x,
            y: prev.y,
            width: prev.width,
            height: prev.height,
            color: prev.color,
            file: createdPath,
            mediaKind: 'note',
          }
        : prev,
    );
    editingTextNodeId = null;
  }

  async function handleExportPng() {
    exporting = true;
    exportStatus = null;
    try {
      const blob = await renderCanvasToPngBlob(data, {
        theme,
        privacyMode,
        noteSnippets,
      });
      if (!blob) {
        exportStatus = 'No se pudo generar el PNG.';
        return;
      }
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      const suffix = privacyMode ? '-privado' : '';
      anchor.href = url;
      anchor.download = `${canvas.title}${suffix}.png`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(url);
      exportStatus = privacyMode
        ? 'PNG exportado en Modo Privacidad'
        : 'PNG exportado correctamente';
      setTimeout(() => {
        exportStatus = null;
      }, 3200);
    } finally {
      exporting = false;
    }
  }

  function handleViewportPointerDown(event: PointerEvent) {
    if (event.button !== 0 && event.button !== 1) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('.canvas-card') || target?.closest('.edge-label-btn')) return;

    if (pendingConnectFrom) {
      pendingConnectFrom = null;
    }
    if (!event.shiftKey) {
      selectedNodeIds = new Set();
      selectedEdgeId = null;
      editingTextNodeId = null;
    }
    interaction = {
      kind: 'pan',
      startX: event.clientX,
      startY: event.clientY,
      origPanX: panX,
      origPanY: panY,
    };
  }

  function handleCardPointerDown(event: PointerEvent, node: CanvasJsonNode) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('button, input, textarea, select, a, .port-dot, .resize-handle')) {
      return;
    }
    event.stopPropagation();

    if (pendingConnectFrom) {
      if (pendingConnectFrom.nodeId !== node.id) {
        createEdgeBetween(pendingConnectFrom.nodeId, node.id, pendingConnectFrom.side);
      }
      pendingConnectFrom = null;
      return;
    }

    selectedEdgeId = null;
    let nextSelected: Set<string>;
    if (event.shiftKey) {
      nextSelected = new Set(selectedNodeIds);
      if (nextSelected.has(node.id)) nextSelected.delete(node.id);
      else nextSelected.add(node.id);
    } else if (selectedNodeIds.has(node.id)) {
      nextSelected = new Set(selectedNodeIds);
    } else {
      nextSelected = new Set([node.id]);
    }
    selectedNodeIds = nextSelected;

    // Include cards whose center falls inside a dragged group (unless Alt is held)
    const dragIds = new Set(nextSelected);
    if (!event.altKey) {
      for (const grpId of nextSelected) {
        const grp = data.nodes.find((n) => n.id === grpId && n.type === 'group');
        if (!grp) continue;
        for (const candidate of data.nodes) {
          if (candidate.id === grp.id) continue;
          const cx = candidate.x + candidate.width / 2;
          const cy = candidate.y + candidate.height / 2;
          if (
            cx >= grp.x &&
            cx <= grp.x + grp.width &&
            cy >= grp.y &&
            cy <= grp.y + grp.height
          ) {
            dragIds.add(candidate.id);
          }
        }
      }
    }

    const initialPositions = new Map<string, { x: number; y: number }>();
    for (const id of dragIds) {
      const n = data.nodes.find((item) => item.id === id);
      if (n) initialPositions.set(id, { x: n.x, y: n.y });
    }

    interaction = {
      kind: 'drag-nodes',
      startX: event.clientX,
      startY: event.clientY,
      initialPositions,
    };
  }

  function handleResizePointerDown(event: PointerEvent, node: CanvasJsonNode) {
    event.stopPropagation();
    event.preventDefault();
    selectedEdgeId = null;
    selectedNodeIds = new Set([node.id]);
    interaction = {
      kind: 'resize-node',
      nodeId: node.id,
      startX: event.clientX,
      startY: event.clientY,
      origW: node.width,
      origH: node.height,
    };
  }

  function handlePortPointerDown(event: PointerEvent, node: CanvasJsonNode, side: CanvasSide) {
    event.stopPropagation();
    event.preventDefault();
    const world = screenToWorld(event.clientX, event.clientY);
    const sx =
      side === 'left' ? node.x : side === 'right' ? node.x + node.width : node.x + node.width / 2;
    const sy =
      side === 'top' ? node.y : side === 'bottom' ? node.y + node.height : node.y + node.height / 2;
    interaction = {
      kind: 'connect-drag',
      fromNodeId: node.id,
      fromSide: side,
      sx,
      sy,
      cx: world.x,
      cy: world.y,
    };
  }

  function handleWindowPointerMove(event: PointerEvent) {
    if (interaction.kind === 'none') return;
    if (interaction.kind === 'pan') {
      panX = interaction.origPanX + (event.clientX - interaction.startX);
      panY = interaction.origPanY + (event.clientY - interaction.startY);
      return;
    }
    if (interaction.kind === 'drag-nodes') {
      const dx = (event.clientX - interaction.startX) / zoom;
      const dy = (event.clientY - interaction.startY) / zoom;
      const posMap = interaction.initialPositions;
      data = {
        ...data,
        nodes: data.nodes.map((node) => {
          const orig = posMap.get(node.id);
          if (!orig) return node;
          return {
            ...node,
            x: snapCoord(orig.x + dx, snapToGrid),
            y: snapCoord(orig.y + dy, snapToGrid),
          };
        }),
      };
      dirty = true;
      return;
    }
    if (interaction.kind === 'resize-node') {
      const dx = (event.clientX - interaction.startX) / zoom;
      const dy = (event.clientY - interaction.startY) / zoom;
      const targetId = interaction.nodeId;
      const nextW = Math.max(160, snapCoord(interaction.origW + dx, snapToGrid));
      const nextH = Math.max(100, snapCoord(interaction.origH + dy, snapToGrid));
      data = {
        ...data,
        nodes: data.nodes.map((node) =>
          node.id === targetId ? { ...node, width: nextW, height: nextH } : node,
        ),
      };
      dirty = true;
      return;
    }
    if (interaction.kind === 'connect-drag') {
      const world = screenToWorld(event.clientX, event.clientY);
      interaction = { ...interaction, cx: world.x, cy: world.y };
    }
  }

  function handleWindowPointerUp(event: PointerEvent) {
    if (interaction.kind === 'none') return;
    if (interaction.kind === 'drag-nodes' || interaction.kind === 'resize-node') {
      scheduleSave();
    } else if (interaction.kind === 'connect-drag') {
      const world = screenToWorld(event.clientX, event.clientY);
      const fromId = interaction.fromNodeId;
      const fromSide = interaction.fromSide;
      const hitNode = [...data.nodes]
        .reverse()
        .find(
          (n) =>
            n.id !== fromId &&
            world.x >= n.x &&
            world.x <= n.x + n.width &&
            world.y >= n.y &&
            world.y <= n.y + n.height,
        );
      if (hitNode) {
        const dxL = Math.abs(world.x - hitNode.x);
        const dxR = Math.abs(world.x - (hitNode.x + hitNode.width));
        const dyT = Math.abs(world.y - hitNode.y);
        const dyB = Math.abs(world.y - (hitNode.y + hitNode.height));
        const minD = Math.min(dxL, dxR, dyT, dyB);
        const toSide: CanvasSide =
          minD === dxL ? 'left' : minD === dxR ? 'right' : minD === dyT ? 'top' : 'bottom';
        createEdgeBetween(fromId, hitNode.id, fromSide, toSide);
      }
    }
    interaction = { kind: 'none' };
  }

  function handleViewportWheel(event: WheelEvent) {
    event.preventDefault();
    if (!viewportEl) return;
    const rect = viewportEl.getBoundingClientRect();
    const pointerX = event.clientX - rect.left;
    const pointerY = event.clientY - rect.top;
    const factor = event.deltaY < 0 ? 1.1 : 0.91;
    const nextZoom = Number(Math.max(0.25, Math.min(2.4, zoom * factor)).toFixed(2));
    if (nextZoom === zoom) return;
    const worldX = (pointerX - panX) / zoom;
    const worldY = (pointerY - panY) / zoom;
    zoom = nextZoom;
    panX = Math.round(pointerX - worldX * nextZoom);
    panY = Math.round(pointerY - worldY * nextZoom);
  }

  function handleKeydown(event: KeyboardEvent) {
    const tag = (event.target as HTMLElement | null)?.tagName;
    const isInput = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      void flushSave();
      return;
    }
    if (isInput) return;
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault();
      deleteSelection();
    } else if (event.key === 'Escape') {
      pendingConnectFrom = null;
      selectedEdgeId = null;
      selectedNodeIds = new Set();
      editingTextNodeId = null;
    }
  }

  function handleEdgeDirectionChange(edge: CanvasJsonEdge, mode: 'uni' | 'bi' | 'none') {
    updateEdge(edge.id, (prev) => ({
      ...prev,
      fromEnd: mode === 'bi' ? 'arrow' : 'none',
      toEnd: mode === 'none' ? 'none' : 'arrow',
    }));
  }

  function edgeDirectionMode(edge: CanvasJsonEdge): 'uni' | 'bi' | 'none' {
    if (edge.fromEnd === 'arrow' && edge.toEnd === 'arrow') return 'bi';
    if (edge.toEnd === 'none' && edge.fromEnd !== 'arrow') return 'none';
    return 'uni';
  }

  function handleLocalMediaFilePick(event: Event, nodeId: string) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') {
        const resultStr = reader.result;
        updateNode(nodeId, (prev) =>
          prev.type === 'file'
            ? { ...prev, file: resultStr, mediaKind: 'media' }
            : prev,
        );
        void flushSave();
      }
    };
    reader.readAsDataURL(file);
  }
</script>

<svelte:window
  onpointermove={handleWindowPointerMove}
  onpointerup={handleWindowPointerUp}
  onkeydown={handleKeydown}
/>

<section class="canvas-shell" aria-label="Editor visual JSON Canvas">
  <header class="canvas-toolbar">
    <div class="toolbar-row">
      <div class="title-group">
        <span class="canvas-badge">JSON CANVAS</span>
        <strong>{canvas.title}.canvas</strong>
        {#if saving || dirty}
          <span class="status-pill dirty">{saving ? 'Guardando...' : 'Cambios sin guardar'}</span>
        {:else}
          <span class="status-pill saved">Guardado</span>
        {/if}
        {#if exportStatus}
          <span class="status-pill saved">{exportStatus}</span>
        {/if}
      </div>

      <div class="toolbar-actions">
        <button
          type="button"
          class="tb-btn"
          class:active={snapToGrid}
          onclick={() => (snapToGrid = !snapToGrid)}
          title="Ajustar tarjetas a la cuadrícula de 20px"
        >
          Snap 20px
        </button>
        <button
          type="button"
          class="tb-btn"
          class:privacy-on={privacyMode}
          onclick={() => (privacyMode = !privacyMode)}
          title="Difuminar texto sensible en pantalla y al exportar PNG"
        >
          {privacyMode ? 'Privacidad: ON' : 'Modo Privacidad'}
        </button>
        <button
          type="button"
          class="tb-btn"
          onclick={() => void handleExportPng()}
          disabled={exporting}
          title="Exportar diagrama completo a imagen PNG"
        >
          {exporting ? 'Exportando...' : 'Exportar PNG'}
        </button>
        <button
          type="button"
          class="tb-btn primary"
          onclick={() => void flushSave()}
          title="Guardar lienzo (.canvas) [Ctrl+S]"
        >
          Guardar
        </button>
        <button type="button" class="tb-btn" onclick={onClose}>
          Editor .md
        </button>
      </div>
    </div>

    <div class="toolbar-row secondary">
      <div class="btn-group" role="group" aria-label="Añadir tarjetas">
        <span class="group-label">Tarjetas:</span>
        <button type="button" class="tb-btn" onclick={() => addNewCard('text')}>+ Texto</button>
        <button type="button" class="tb-btn" onclick={() => addNewCard('note')}>+ Nota</button>
        <button type="button" class="tb-btn" onclick={() => addNewCard('media')}>+ Multimedia</button>
        <button type="button" class="tb-btn" onclick={() => addNewCard('link')}>+ Web</button>
        <button type="button" class="tb-btn" onclick={() => addNewCard('group')}>+ Grupo</button>
      </div>

      <div class="btn-group" role="group" aria-label="Alineación y cuadrícula">
        <span class="group-label">Alinear:</span>
        <button
          type="button"
          class="tb-btn"
          disabled={selectedNodeIds.size < 2}
          onclick={() => handleAlign('left')}
          title="Alinear a la izquierda"
        >
          ◧ Izq
        </button>
        <button
          type="button"
          class="tb-btn"
          disabled={selectedNodeIds.size < 2}
          onclick={() => handleAlign('center-h')}
          title="Centrar horizontalmente"
        >
          ◫ Centro
        </button>
        <button
          type="button"
          class="tb-btn"
          disabled={selectedNodeIds.size < 2}
          onclick={() => handleAlign('top')}
          title="Alinear arriba"
        >
          ⬒ Arriba
        </button>
        <button
          type="button"
          class="tb-btn"
          disabled={selectedNodeIds.size < 3}
          onclick={() => handleAlign('distribute-h')}
          title="Distribuir horizontalmente (3+ tarjetas)"
        >
          ⊞ Distribuir
        </button>
        <button
          type="button"
          class="tb-btn"
          disabled={data.nodes.filter((n) => n.type !== 'group').length < 2}
          onclick={handleArrangeGrid}
          title="Organizar tarjetas en cuadrícula uniforme"
        >
          ▦ Cuadrícula
        </button>
      </div>

      <div class="btn-group">
        <button
          type="button"
          class="tb-btn"
          onclick={() => (zoom = Number(Math.max(0.25, zoom - 0.15).toFixed(2)))}
          title="Alejar"
        >
          −
        </button>
        <span class="zoom-readout">{Math.round(zoom * 100)}%</span>
        <button
          type="button"
          class="tb-btn"
          onclick={() => (zoom = Number(Math.min(2.4, zoom + 0.15).toFixed(2)))}
          title="Acercar"
        >
          +
        </button>
        <button type="button" class="tb-btn" onclick={fitToView} title="Encuadrar lienzo">
          Encuadrar
        </button>
      </div>
    </div>
  </header>

  {#if pendingConnectFrom}
    <div class="connect-banner">
      <span>Haz clic en otra tarjeta destino para crear la conexión direccional.</span>
      <button type="button" class="tb-btn" onclick={() => (pendingConnectFrom = null)}>
        Cancelar
      </button>
    </div>
  {/if}

  <div class="canvas-stage">
    <div
      bind:this={viewportEl}
      class="canvas-viewport"
      class:panning={interaction.kind === 'pan'}
      style:background-position={`${panX}px ${panY}px`}
      style:background-size={`${20 * zoom}px ${20 * zoom}px`}
      onpointerdown={handleViewportPointerDown}
      onwheel={handleViewportWheel}
      role="application"
      aria-label="Superficie interactiva del lienzo"
    >
      <div
        class="canvas-world"
        style:transform={`translate(${panX}px, ${panY}px) scale(${zoom})`}
      >
        <svg class="edges-svg" aria-label="Conectores del lienzo">
          {#each computedEdges as item (item.edge.id)}
            {@const isSel = selectedEdgeId === item.edge.id}
            <g class="edge-group">
              <path
                d={item.geom.pathD}
                class="edge-hit-path"
                role="button"
                tabindex="0"
                aria-label={item.edge.label || 'Conector'}
                onclick={(e) => {
                  e.stopPropagation();
                  selectedNodeIds = new Set();
                  selectedEdgeId = item.edge.id;
                }}
                onkeydown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    selectedNodeIds = new Set();
                    selectedEdgeId = item.edge.id;
                  }
                }}
              />
              <path
                d={item.geom.pathD}
                class="edge-visible-path"
                class:selected={isSel}
                style:stroke={isSel ? 'var(--accent)' : item.stroke}
              />
              {#if item.edge.toEnd !== 'none'}
                {@const angle = Math.atan2(
                  item.geom.ey - item.geom.c2y,
                  item.geom.ex - item.geom.c2x,
                )}
                <polygon
                  points={`0,0 -11,-5.5 -11,5.5`}
                  transform={`translate(${item.geom.ex}, ${item.geom.ey}) rotate(${(angle * 180) / Math.PI})`}
                  fill={isSel ? 'var(--accent)' : item.stroke}
                />
              {/if}
              {#if item.edge.fromEnd === 'arrow'}
                {@const angleStart = Math.atan2(
                  item.geom.sy - item.geom.c1y,
                  item.geom.sx - item.geom.c1x,
                )}
                <polygon
                  points={`0,0 -11,-5.5 -11,5.5`}
                  transform={`translate(${item.geom.sx}, ${item.geom.sy}) rotate(${(angleStart * 180) / Math.PI})`}
                  fill={isSel ? 'var(--accent)' : item.stroke}
                />
              {/if}
            </g>
          {/each}

          {#if interaction.kind === 'connect-drag'}
            <line
              x1={interaction.sx}
              y1={interaction.sy}
              x2={interaction.cx}
              y2={interaction.cy}
              class="edge-draft-line"
            />
          {/if}
        </svg>

        {#each computedEdges as item (item.edge.id)}
          {#if item.edge.label?.trim() || selectedEdgeId === item.edge.id}
            <button
              type="button"
              class="edge-label-btn"
              class:selected={selectedEdgeId === item.edge.id}
              class:privacy-blur={privacyMode}
              style:left={`${item.geom.mx}px`}
              style:top={`${item.geom.my}px`}
              style:border-color={item.stroke}
              onclick={(e) => {
                e.stopPropagation();
                selectedNodeIds = new Set();
                selectedEdgeId = item.edge.id;
              }}
              title="Seleccionar conexión (Follow connection disponible en inspector)"
            >
              {item.edge.label?.trim() || 'Conexión'}
            </button>
          {/if}
        {/each}

        {#each sortedNodesForRender as node (node.id)}
          {@const selected = selectedNodeIds.has(node.id)}
          {@const accent = resolveCanvasColor(node.color, theme === 'dark' ? '#64748b' : '#475569')}
          <article
            class="canvas-card"
            class:is-group={node.type === 'group'}
            class:selected
            style:left={`${node.x}px`}
            style:top={`${node.y}px`}
            style:width={`${node.width}px`}
            style:height={`${node.height}px`}
            style:border-color={selected ? 'var(--accent)' : accent}
            style:--card-accent={accent}
            style:background-image={node.type === 'group' && node.background
              ? `url("${node.background}")`
              : undefined}
            style:background-size={node.type === 'group' && node.backgroundStyle === 'ratio'
              ? 'contain'
              : node.type === 'group' && node.backgroundStyle === 'repeat'
                ? '160px'
                : 'cover'}
            style:background-repeat={node.type === 'group' && node.backgroundStyle === 'repeat'
              ? 'repeat'
              : 'no-repeat'}
            onpointerdown={(e) => handleCardPointerDown(e, node)}
            ondblclick={() => {
              if (node.type === 'text') editingTextNodeId = node.id;
            }}
          >
            <header class="card-head">
              <span class="card-type-pill" class:privacy-blur={privacyMode && node.type === 'group'}>
                {#if node.type === 'text'}
                  TEXTO MD
                {:else if node.type === 'file'}
                  {isNoteFileNode(node)
                    ? 'NOTA BÓVEDA'
                    : `MEDIA · ${detectMediaKindLabel(node.file).toUpperCase()}`}
                {:else if node.type === 'link'}
                  WEB
                {:else}
                  <input
                    type="text"
                    class="group-title-input"
                    value={node.label || 'Grupo'}
                    placeholder="Grupo..."
                    oninput={(e) => {
                      const val = (e.target as HTMLInputElement).value;
                      updateNode(node.id, (p) => (p.type === 'group' ? { ...p, label: val } : p));
                    }}
                    onpointerdown={(e) => e.stopPropagation()}
                  />
                {/if}
              </span>

              <div class="card-head-tools">
                {#if node.type === 'file' && isNoteFileNode(node) && node.file}
                  <button
                    type="button"
                    class="mini-card-btn"
                    onclick={() => onOpenNote(node.file, node.subpath)}
                    title="Abrir nota en editor"
                  >
                    Abrir
                  </button>
                {:else if node.type === 'link' && isSafeExternalUrl(node.url)}
                  <a
                    class="mini-card-btn"
                    href={node.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    title="Abrir página web externa"
                  >
                    ↗
                  </a>
                {:else if node.type === 'text'}
                  <button
                    type="button"
                    class="mini-card-btn"
                    onclick={() => void handleConvertCardToMd(node)}
                    title="Convertir tarjeta de texto en archivo .md"
                  >
                    → .md
                  </button>
                {/if}
                <button
                  type="button"
                  class="mini-card-btn danger"
                  onclick={(e) => {
                    e.stopPropagation();
                    deleteNode(node.id);
                  }}
                  title="Eliminar esta tarjeta"
                >
                  ✕
                </button>
              </div>
            </header>

            {#if node.type !== 'group'}
              <div class="card-body" class:privacy-blur={privacyMode}>
                {#if node.type === 'text'}
                  {#if editingTextNodeId === node.id}
                    <textarea
                      class="inline-card-textarea"
                      value={node.text}
                      oninput={(e) => {
                        const val = (e.target as HTMLTextAreaElement).value;
                        updateNode(node.id, (prev) =>
                          prev.type === 'text' ? { ...prev, text: val } : prev,
                        );
                      }}
                      onblur={() => {
                        editingTextNodeId = null;
                      }}
                      onpointerdown={(e) => e.stopPropagation()}
                    ></textarea>
                  {:else}
                    <div
                      role="button"
                      tabindex="0"
                      class="card-pre-btn"
                      onclick={() => {
                        editingTextNodeId = node.id;
                      }}
                      onkeydown={(e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          editingTextNodeId = node.id;
                        }
                      }}
                      title="Hacé clic para editar el texto en Markdown"
                    >
                      <pre class="card-pre">{node.text || '(Hacé clic para escribir...)'}</pre>
                    </div>
                  {/if}
                {:else if node.type === 'file'}
                  {#if isNoteFileNode(node)}
                    <div class="note-select-row" role="presentation" onpointerdown={(e) => e.stopPropagation()}>
                      <select
                        class="card-note-select"
                        value={node.file}
                        onchange={(e) => {
                          const nextFile = (e.target as HTMLSelectElement).value;
                          updateNode(node.id, (p) =>
                            p.type === 'file' ? { ...p, file: nextFile, subpath: undefined } : p,
                          );
                        }}
                      >
                        {#each notes as item (item.relative_path)}
                          <option value={item.relative_path}>{item.title || item.relative_path}</option>
                        {/each}
                      </select>
                    </div>
                    {#if node.subpath}
                      <div class="subpath-chip">{node.subpath}</div>
                    {/if}
                    <pre class="card-pre muted">{noteDisplayBodyLines(node, noteSnippets).slice(1).join('\n')}</pre>
                  {:else if canRenderImageDirectly(node.file)}
                    <div class="media-preview-container">
                      <img
                        class="media-thumb"
                        src={node.file}
                        alt="Recurso multimedia en lienzo"
                      />
                      <label class="media-change-overlay" onpointerdown={(e) => e.stopPropagation()}>
                        📷 Cambiar
                        <input
                          type="file"
                          accept="image/*,audio/*"
                          style="display: none;"
                          onchange={(e) => handleLocalMediaFilePick(e, node.id)}
                        />
                      </label>
                    </div>
                  {:else}
                    <div class="media-badge-card">
                      <strong>{detectMediaKindLabel(node.file)}</strong>
                      <label class="media-pick-btn" onpointerdown={(e) => e.stopPropagation()}>
                        📷 Elegir Imagen o Archivo
                        <input
                          type="file"
                          accept="image/*,audio/*"
                          style="display: none;"
                          onchange={(e) => handleLocalMediaFilePick(e, node.id)}
                        />
                      </label>
                    </div>
                  {/if}
                {:else if node.type === 'link'}
                  <div class="link-preview-box" role="presentation" onpointerdown={(e) => e.stopPropagation()}>
                    <input
                      type="url"
                      class="card-link-input"
                      value={node.url}
                      placeholder="https://..."
                      oninput={(e) => {
                        const nextUrl = (e.target as HTMLInputElement).value;
                        updateNode(node.id, (p) => (p.type === 'link' ? { ...p, url: nextUrl } : p));
                      }}
                    />
                    <small>Hacé clic en ↗ arriba para abrir el enlace</small>
                  </div>
                {/if}
              </div>
            {/if}

            <!-- Connection port handles -->
            <button
              type="button"
              class="port-dot port-top"
              aria-label="Conectar desde arriba"
              onpointerdown={(e) => handlePortPointerDown(e, node, 'top')}
            ></button>
            <button
              type="button"
              class="port-dot port-right"
              aria-label="Conectar desde la derecha"
              onpointerdown={(e) => handlePortPointerDown(e, node, 'right')}
            ></button>
            <button
              type="button"
              class="port-dot port-bottom"
              aria-label="Conectar desde abajo"
              onpointerdown={(e) => handlePortPointerDown(e, node, 'bottom')}
            ></button>
            <button
              type="button"
              class="port-dot port-left"
              aria-label="Conectar desde la izquierda"
              onpointerdown={(e) => handlePortPointerDown(e, node, 'left')}
            ></button>

            <div
              class="resize-handle"
              role="presentation"
              title="Redimensionar tarjeta"
              onpointerdown={(e) => handleResizePointerDown(e, node)}
            ></div>
          </article>
        {/each}
      </div>
    </div>

    {#if primarySelectedNode || selectedEdge}
      <aside class="inspector-dock" aria-label="Inspector de selección">
        {#if primarySelectedNode}
          {@const node = primarySelectedNode}
          <div class="inspector-head">
            <strong>
              {node.type === 'text'
                ? 'Tarjeta Texto'
                : node.type === 'file'
                  ? isNoteFileNode(node)
                    ? 'Nota Bóveda'
                    : 'Multimedia'
                  : node.type === 'link'
                    ? 'Página Web'
                    : 'Grupo'}
            </strong>
            <div class="inspector-head-btns">
              <button
                type="button"
                class="tb-btn"
                onclick={() => (pendingConnectFrom = { nodeId: node.id })}
                title="Conectar con otra tarjeta"
              >
                Conectar →
              </button>
              <button
                type="button"
                class="tb-btn danger"
                onclick={deleteSelection}
                title="Eliminar tarjeta (Supr)"
              >
                Eliminar
              </button>
            </div>
          </div>

          <div class="color-row">
            <span class="field-lbl">Color:</span>
            <button
              type="button"
              class="swatch"
              class:active={!node.color}
              onclick={() => updateNode(node.id, (p) => ({ ...p, color: undefined }))}
              title="Por defecto"
            >
              ∅
            </button>
            {#each CANVAS_COLOR_OPTIONS as opt (opt.code)}
              <button
                type="button"
                class="swatch"
                class:active={node.color === opt.code}
                style:background={opt.hex}
                onclick={() => updateNode(node.id, (p) => ({ ...p, color: opt.code }))}
                title={opt.label}
                aria-label={`Color ${opt.label}`}
              ></button>
            {/each}
          </div>

          {#if node.type === 'text'}
            <label class="inspector-field">
              <span class="field-lbl">Contenido Markdown (sin backlinks hasta convertir):</span>
              <textarea
                rows="6"
                value={node.text}
                oninput={(e) => {
                  const val = (e.target as HTMLTextAreaElement).value;
                  updateNode(node.id, (p) => (p.type === 'text' ? { ...p, text: val } : p));
                }}
              ></textarea>
            </label>
            <button
              type="button"
              class="tb-btn primary full"
              onclick={() => void handleConvertCardToMd(node)}
            >
              Convertir tarjeta a archivo .md
            </button>
          {:else if node.type === 'file' && isNoteFileNode(node)}
            {@const detail = noteDetailsCache[node.file]}
            <label class="inspector-field">
              <span class="field-lbl">Nota de la bóveda:</span>
              <select
                value={node.file}
                onchange={(e) => {
                  const nextFile = (e.target as HTMLSelectElement).value;
                  updateNode(node.id, (p) =>
                    p.type === 'file' ? { ...p, file: nextFile, subpath: undefined } : p,
                  );
                }}
              >
                {#if node.file && !notes.some((n) => n.relative_path === node.file)}
                  <option value={node.file}>{node.file}</option>
                {/if}
                {#each notes as item (item.relative_path)}
                  <option value={item.relative_path}>{item.relative_path}</option>
                {/each}
              </select>
            </label>

            <label class="inspector-field">
              <span class="field-lbl">Acotar sección (heading / block):</span>
              <select
                value={node.subpath ?? ''}
                onchange={(e) => {
                  const nextSub = (e.target as HTMLSelectElement).value || undefined;
                  updateNode(node.id, (p) =>
                    p.type === 'file' ? { ...p, subpath: nextSub } : p,
                  );
                }}
              >
                <option value="">(Nota completa)</option>
                {#if detail?.content}
                  {#each extractHeadingsFromContent(detail.content) as hd (hd)}
                    <option value={`#${hd}`}># {hd}</option>
                  {/each}
                  {#each extractBlocksFromContent(detail.content) as blk (blk.id)}
                    <option value={`#^${blk.id}`}>#^{blk.id} · {blk.text.slice(0, 28)}</option>
                  {/each}
                {/if}
              </select>
            </label>

            <input
              type="text"
              class="subpath-manual"
              value={node.subpath ?? ''}
              placeholder="#Encabezado o #^id-bloque"
              oninput={(e) => {
                const v = (e.target as HTMLInputElement).value.trim() || undefined;
                updateNode(node.id, (p) => (p.type === 'file' ? { ...p, subpath: v } : p));
              }}
            />

            <button
              type="button"
              class="tb-btn primary full"
              onclick={() => onOpenNote(node.file, node.subpath)}
            >
              Abrir nota en Editor
            </button>
          {:else if node.type === 'file'}
            <label class="inspector-field">
              <span class="field-lbl">Ruta o URL multimedia (PNG/JPG/PDF/Audio):</span>
              <input
                type="text"
                value={node.file}
                placeholder="assets/diagrama.png o https://..."
                oninput={(e) => {
                  const nextVal = (e.target as HTMLInputElement).value;
                  updateNode(node.id, (p) =>
                    p.type === 'file' ? { ...p, file: nextVal, mediaKind: 'media' } : p,
                  );
                }}
              />
            </label>
            <label class="inspector-field">
              <span class="field-lbl">Incrustar imagen local (Data URL):</span>
              <input
                type="file"
                accept="image/*,audio/*"
                onchange={(e) => handleLocalMediaFilePick(e, node.id)}
              />
            </label>
          {:else if node.type === 'link'}
            <label class="inspector-field">
              <span class="field-lbl">URL de página web:</span>
              <input
                type="url"
                value={node.url}
                placeholder="https://..."
                oninput={(e) => {
                  const nextUrl = (e.target as HTMLInputElement).value;
                  updateNode(node.id, (p) => (p.type === 'link' ? { ...p, url: nextUrl } : p));
                }}
              />
            </label>
            {#if isSafeExternalUrl(node.url)}
              <a
                class="tb-btn primary full link-btn"
                href={node.url}
                target="_blank"
                rel="noopener noreferrer"
              >
                Abrir página web ↗
              </a>
            {/if}
          {:else if node.type === 'group'}
            <label class="inspector-field">
              <span class="field-lbl">Etiqueta del grupo:</span>
              <input
                type="text"
                value={node.label ?? ''}
                oninput={(e) => {
                  const lbl = (e.target as HTMLInputElement).value;
                  updateNode(node.id, (p) => (p.type === 'group' ? { ...p, label: lbl } : p));
                }}
              />
            </label>
            <label class="inspector-field">
              <span class="field-lbl">Fondo opcional (URL/Data imagen):</span>
              <input
                type="text"
                value={node.background ?? ''}
                placeholder="https://... o vacío"
                oninput={(e) => {
                  const bg = (e.target as HTMLInputElement).value.trim() || undefined;
                  updateNode(node.id, (p) =>
                    p.type === 'group' ? { ...p, background: bg } : p,
                  );
                }}
              />
            </label>
            <label class="inspector-field">
              <span class="field-lbl">Estilo de fondo:</span>
              <select
                value={node.backgroundStyle ?? 'cover'}
                onchange={(e) => {
                  const st = (e.target as HTMLSelectElement).value as 'cover' | 'ratio' | 'repeat';
                  updateNode(node.id, (p) =>
                    p.type === 'group' ? { ...p, backgroundStyle: st } : p,
                  );
                }}
              >
                <option value="cover">Cover (cubrir)</option>
                <option value="ratio">Ratio (ajustar)</option>
                <option value="repeat">Repeat (mosaico)</option>
              </select>
            </label>
          {/if}
        {:else if selectedEdge}
          {@const edge = selectedEdge}
          <div class="inspector-head">
            <strong>Conexión direccional</strong>
            <button type="button" class="tb-btn danger" onclick={deleteSelection}>
              Eliminar
            </button>
          </div>

          <label class="inspector-field">
            <span class="field-lbl">Etiqueta de conexión:</span>
            <input
              type="text"
              value={edge.label ?? ''}
              placeholder="Ej: depende de / referencia"
              oninput={(e) => {
                const nextLbl = (e.target as HTMLInputElement).value;
                updateEdge(edge.id, (p) => ({ ...p, label: nextLbl }));
              }}
            />
          </label>

          <label class="inspector-field">
            <span class="field-lbl">Direccionalidad:</span>
            <select
              value={edgeDirectionMode(edge)}
              onchange={(e) => {
                const mode = (e.target as HTMLSelectElement).value as 'uni' | 'bi' | 'none';
                handleEdgeDirectionChange(edge, mode);
              }}
            >
              <option value="uni">Unidireccional (→)</option>
              <option value="bi">Bidireccional (↔)</option>
              <option value="none">No direccional (—)</option>
            </select>
          </label>

          <div class="color-row">
            <span class="field-lbl">Color:</span>
            <button
              type="button"
              class="swatch"
              class:active={!edge.color}
              onclick={() => updateEdge(edge.id, (p) => ({ ...p, color: undefined }))}
              title="Por defecto"
            >
              ∅
            </button>
            {#each CANVAS_COLOR_OPTIONS as opt (opt.code)}
              <button
                type="button"
                class="swatch"
                class:active={edge.color === opt.code}
                style:background={opt.hex}
                onclick={() => updateEdge(edge.id, (p) => ({ ...p, color: opt.code }))}
                title={opt.label}
                aria-label={`Color ${opt.label}`}
              ></button>
            {/each}
          </div>

          <div class="follow-row">
            <button
              type="button"
              class="tb-btn primary"
              onclick={() => focusOnNode(edge.toNode)}
              title="Centrar vista en el nodo destino (Follow connection)"
            >
              Follow connection →
            </button>
            <button
              type="button"
              class="tb-btn"
              onclick={() => focusOnNode(edge.fromNode)}
              title="Centrar vista en el nodo origen"
            >
              ← Ir a origen
            </button>
          </div>
        {/if}
      </aside>
    {/if}
  </div>
</section>

<style>
  .canvas-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg-app);
  }

  .canvas-toolbar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-surface);
  }

  .toolbar-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }

  .toolbar-row.secondary {
    padding-top: 4px;
    border-top: 1px solid var(--border-subtle);
  }

  .title-group,
  .toolbar-actions,
  .btn-group {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .canvas-badge {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    padding: 2px 6px;
    border-radius: 5px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .group-label {
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 600;
    margin-right: 2px;
  }

  .status-pill {
    font-size: 11.5px;
    padding: 2px 7px;
    border-radius: 999px;
    border: 1px solid var(--border-subtle);
  }

  .status-pill.saved {
    color: var(--text-muted);
  }

  .status-pill.dirty {
    color: #f59e0b;
    border-color: rgba(245, 158, 11, 0.45);
  }

  .tb-btn {
    border: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    color: var(--text-primary);
    border-radius: 6px;
    padding: 4px 9px;
    font-size: 12px;
    cursor: pointer;
  }

  .tb-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .tb-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .tb-btn.primary {
    background: var(--accent);
    color: #ffffff;
    border-color: var(--accent);
  }

  .tb-btn.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .tb-btn.privacy-on {
    background: rgba(168, 85, 247, 0.2);
    border-color: #a855f7;
    color: #d8b4fe;
    font-weight: 600;
  }

  .tb-btn.danger {
    border-color: rgba(239, 68, 68, 0.5);
    color: #ef4444;
  }

  .tb-btn.full {
    width: 100%;
  }

  .zoom-readout {
    font-size: 12px;
    min-width: 42px;
    text-align: center;
    color: var(--text-secondary);
  }

  .connect-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    background: var(--accent-soft);
    border-bottom: 1px solid var(--accent);
    font-size: 12.5px;
  }

  .canvas-stage {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .canvas-viewport {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    cursor: grab;
    background-image: radial-gradient(circle, var(--border-subtle) 1.2px, transparent 1.2px);
    user-select: none;
  }

  .canvas-viewport.panning {
    cursor: grabbing;
  }

  .canvas-world {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
    width: 1px;
    height: 1px;
  }

  .edges-svg {
    position: absolute;
    left: 0;
    top: 0;
    width: 1px;
    height: 1px;
    overflow: visible;
    pointer-events: none;
    z-index: 2;
  }

  .edge-group {
    pointer-events: auto;
  }

  .edge-hit-path {
    fill: none;
    stroke: transparent;
    stroke-width: 14;
    cursor: pointer;
  }

  .edge-visible-path {
    fill: none;
    stroke-width: 2.2;
    pointer-events: none;
  }

  .edge-visible-path.selected {
    stroke-width: 3.2;
  }

  .edge-draft-line {
    stroke: var(--accent);
    stroke-width: 2;
    stroke-dasharray: 5 4;
  }

  .edge-label-btn {
    position: absolute;
    transform: translate(-50%, -50%);
    z-index: 3;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--text-primary);
    border-radius: 999px;
    padding: 2px 9px;
    font-size: 11.5px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }

  .edge-label-btn.selected {
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .canvas-card {
    position: absolute;
    z-index: 4;
    display: flex;
    flex-direction: column;
    border-radius: 10px;
    border: 1.8px solid var(--card-accent, var(--border-subtle));
    background: var(--bg-surface);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.16);
    overflow: visible;
  }

  .canvas-card.is-group {
    z-index: 1;
    border-style: dashed;
    background: color-mix(in srgb, var(--card-accent, var(--accent)) 8%, transparent);
    box-shadow: none;
  }

  .canvas-card.selected {
    box-shadow: 0 0 0 2px var(--accent-soft), 0 10px 24px rgba(0, 0, 0, 0.24);
  }

  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 6px 9px;
    border-top-left-radius: 8px;
    border-top-right-radius: 8px;
    background: color-mix(in srgb, var(--card-accent, var(--accent)) 14%, transparent);
    border-bottom: 1px solid var(--border-subtle);
    cursor: move;
  }

  .card-type-pill {
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--card-accent, var(--text-secondary));
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-head-tools {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .mini-card-btn {
    border: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    color: var(--text-primary);
    border-radius: 4px;
    padding: 1px 6px;
    font-size: 11px;
    text-decoration: none;
    cursor: pointer;
  }

  .mini-card-btn.danger {
    color: #ef4444;
    font-weight: 700;
  }

  .mini-card-btn.danger:hover {
    background: rgba(239, 68, 68, 0.15);
    border-color: #ef4444;
  }

  .group-title-input {
    background: transparent;
    border: none;
    color: var(--card-accent, var(--text-primary));
    font-weight: 700;
    font-size: 11px;
    outline: none;
    width: 140px;
    padding: 0;
  }

  .note-select-row {
    margin-bottom: 6px;
  }

  .card-note-select {
    width: 100%;
    padding: 3px 6px;
    font-size: 11.5px;
    border-radius: 5px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-elevated);
    color: var(--text-primary);
    outline: none;
  }

  .media-preview-container {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 90px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .media-change-overlay {
    position: absolute;
    bottom: 4px;
    right: 4px;
    background: rgba(0, 0, 0, 0.72);
    color: #fff;
    padding: 2px 8px;
    border-radius: 4px;
    font-size: 10.5px;
    cursor: pointer;
  }

  .media-pick-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    background: var(--accent);
    color: #fff;
    padding: 5px 10px;
    border-radius: 6px;
    font-size: 11.5px;
    font-weight: 600;
    cursor: pointer;
    margin-top: 4px;
    text-align: center;
  }

  .media-pick-btn:hover {
    opacity: 0.9;
  }

  .card-link-input {
    width: 100%;
    padding: 3px 6px;
    font-size: 11.5px;
    border-radius: 5px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    color: var(--accent);
    outline: none;
  }

  .card-pre-btn {
    cursor: pointer;
    border-radius: 4px;
    padding: 2px;
  }

  .card-pre-btn:hover {
    background: rgba(255, 255, 255, 0.04);
  }

  .card-body {
    flex: 1;
    min-height: 0;
    padding: 8px 10px;
    overflow: auto;
    user-select: text;
  }

  .privacy-blur {
    filter: blur(5px);
    user-select: none !important;
  }

  .card-pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: inherit;
    font-size: 12.5px;
    line-height: 1.42;
    color: var(--text-primary);
  }

  .card-pre.muted {
    color: var(--text-secondary);
    font-size: 12px;
  }

  .inline-card-textarea {
    width: 100%;
    height: 100%;
    resize: none;
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 6px;
    background: var(--bg-app);
    color: var(--text-primary);
    font: inherit;
    font-size: 12.5px;
  }

  .note-ref-path {
    font-weight: 600;
    font-size: 12.5px;
    margin-bottom: 4px;
    word-break: break-word;
  }

  .subpath-chip {
    display: inline-block;
    font-size: 11px;
    padding: 1px 6px;
    margin-bottom: 6px;
    border-radius: 5px;
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .media-thumb {
    width: 100%;
    height: 100%;
    object-fit: contain;
    border-radius: 6px;
  }

  .media-badge-card,
  .link-preview-box {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px;
    border-radius: 7px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-subtle);
    font-size: 12px;
    word-break: break-all;
  }

  .link-url {
    color: var(--accent);
    font-weight: 600;
  }

  .port-dot {
    position: absolute;
    width: 11px;
    height: 11px;
    border-radius: 999px;
    border: 2px solid var(--accent);
    background: var(--bg-surface);
    opacity: 0;
    cursor: crosshair;
    transition: opacity 0.12s ease;
    padding: 0;
  }

  .canvas-card:hover .port-dot,
  .canvas-card.selected .port-dot {
    opacity: 1;
  }

  .port-top {
    top: -6px;
    left: 50%;
    transform: translateX(-50%);
  }

  .port-right {
    right: -6px;
    top: 50%;
    transform: translateY(-50%);
  }

  .port-bottom {
    bottom: -6px;
    left: 50%;
    transform: translateX(-50%);
  }

  .port-left {
    left: -6px;
    top: 50%;
    transform: translateY(-50%);
  }

  .resize-handle {
    position: absolute;
    right: 2px;
    bottom: 2px;
    width: 12px;
    height: 12px;
    cursor: nwse-resize;
    border-right: 2px solid var(--text-muted);
    border-bottom: 2px solid var(--text-muted);
    opacity: 0.55;
  }

  .inspector-dock {
    position: absolute;
    right: 12px;
    bottom: 12px;
    z-index: 20;
    width: 300px;
    max-height: calc(100% - 24px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: 10px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-surface);
    box-shadow: 0 12px 28px rgba(0, 0, 0, 0.28);
  }

  .inspector-head,
  .inspector-head-btns,
  .color-row,
  .follow-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    flex-wrap: wrap;
  }

  .inspector-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-lbl {
    font-size: 11.5px;
    color: var(--text-secondary);
    font-weight: 600;
  }

  .inspector-dock input,
  .inspector-dock textarea,
  .inspector-dock select {
    width: 100%;
    padding: 6px 8px;
    border-radius: 6px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-app);
    color: var(--text-primary);
    font: inherit;
    font-size: 12.5px;
  }

  .subpath-manual {
    margin-top: -4px;
  }

  .swatch {
    width: 22px;
    height: 22px;
    border-radius: 999px;
    border: 2px solid transparent;
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: 11px;
    cursor: pointer;
    padding: 0;
  }

  .swatch.active {
    border-color: var(--text-primary);
    transform: scale(1.08);
  }

  .link-btn {
    text-align: center;
    text-decoration: none;
  }
</style>
