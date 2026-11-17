<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { getGraphData } from '../api';
  import type { GraphData, GraphEdge, GraphNode } from '../types';

  interface ColorGroupRule {
    id: string;
    query: string;
    color: string;
  }

  interface SimNode extends GraphNode {
    x: number;
    y: number;
    vx: number;
    vy: number;
    radius: number;
    color: string | null;
  }

  interface Props {
    activeNotePath: string | null;
    theme: 'dark' | 'light';
    refreshToken?: number;
    mode?: 'global' | 'local';
    onModeChange?: (mode: 'global' | 'local') => void;
    onOpenNote: (relativePath: string) => void;
    onOpenWikilink: (target: string) => void;
    onClose?: () => void;
  }

  let {
    activeNotePath,
    theme,
    refreshToken = 0,
    mode = 'global',
    onModeChange,
    onOpenNote,
    onOpenWikilink,
    onClose,
  }: Props = $props();

  const COLOR_GROUPS_STORAGE_KEY = 'romsidian:graph:color-groups';
  const PRESET_COLORS = ['#7c6cff', '#2ea043', '#d29922', '#f85149', '#39c5cf', '#db61a2'];

  let rawGraph = $state<GraphData>({ nodes: [], edges: [] });
  let loading = $state(false);
  let errorMsg = $state<string | null>(null);

  let filtersOpen = $state(typeof window !== 'undefined' ? window.innerWidth > 900 : true);
  let searchQuery = $state('');
  let tagFilter = $state('');
  let folderFilter = $state('');
  let showOrphans = $state(true);
  let showUnresolved = $state(true);
  let localDepth = $state(1);

  let linkDistance = $state(100);
  let repulsionForce = $state(240);

  let colorGroups = $state<ColorGroupRule[]>(loadStoredColorGroups());
  let newGroupQuery = $state('');
  let newGroupColor = $state(PRESET_COLORS[0]);

  let hoveredNode = $state<GraphNode | null>(null);
  let zoomPercent = $state(100);
  let settling = $state(false);

  let canvasEl: HTMLCanvasElement | null = $state(null);
  let stageEl: HTMLDivElement | null = $state(null);

  // Estado mutable de simulación fuera de reactividad por cuadro (60fps sin re-renders Svelte)
  let simNodes: SimNode[] = [];
  let simNodeById = new Map<string, SimNode>();
  let simEdges: Array<{ source: SimNode; target: SimNode; resolved: boolean }> = [];
  let neighborMap = new Map<string, Set<string>>();

  let camX = 0;
  let camY = 0;
  let camZoom = 1;
  let viewportW = 800;
  let viewportH = 600;
  let alpha = 0;
  let rafId = 0;

  let dragMode: 'none' | 'node' | 'pan' = 'none';
  let draggedNode: SimNode | null = null;
  let pointerStartX = 0;
  let pointerStartY = 0;
  let panStartX = 0;
  let panStartY = 0;
  let pointerMoved = false;
  let currentHoverId: string | null = null;

  function loadStoredColorGroups(): ColorGroupRule[] {
    try {
      const raw = localStorage.getItem(COLOR_GROUPS_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as ColorGroupRule[];
        if (Array.isArray(parsed)) return parsed;
      }
    } catch {
      // ignorar errores de almacenamiento local
    }
    return [
      { id: 'default-arch', query: '#arquitectura', color: '#7c6cff' },
      { id: 'default-daily', query: 'folder:daily', color: '#2ea043' },
    ];
  }

  function persistColorGroups(next: ColorGroupRule[]) {
    colorGroups = next;
    try {
      localStorage.setItem(COLOR_GROUPS_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignorar errores de cuota
    }
  }

  let availableTags = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const node of rawGraph.nodes) {
      for (const tag of node.tags) {
        counts.set(tag, (counts.get(tag) ?? 0) + 1);
      }
    }
    return Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([tag, count]) => ({ tag, count }));
  });

  let availableFolders = $derived.by(() => {
    const folders = new Set<string>();
    for (const node of rawGraph.nodes) {
      if (node.exists) {
        folders.add(node.folder || '(raíz)');
      }
    }
    return Array.from(folders).sort((a, b) => a.localeCompare(b));
  });

  function matchesGroupQuery(node: GraphNode, rawQuery: string): boolean {
    const q = rawQuery.trim().toLowerCase();
    if (!q) return false;
    if (q.startsWith('#')) {
      const targetTag = q.slice(1);
      return node.tags.some((t) => t === targetTag || t.startsWith(`${targetTag}/`));
    }
    if (q.startsWith('tag:')) {
      const targetTag = q.slice(4).trim().replace(/^#/, '');
      return node.tags.some((t) => t === targetTag || t.startsWith(`${targetTag}/`));
    }
    if (q.startsWith('folder:') || q.startsWith('path:')) {
      const prefix = q.slice(q.indexOf(':') + 1).trim().replace(/^\/+|\/+$/g, '');
      if (!prefix || prefix === '(raíz)') return node.exists && node.folder === '';
      return (
        node.folder.toLowerCase() === prefix ||
        node.folder.toLowerCase().startsWith(`${prefix}/`) ||
        node.id.toLowerCase().startsWith(`${prefix}/`)
      );
    }
    return node.label.toLowerCase().includes(q) || node.id.toLowerCase().includes(q);
  }

  function resolveNodeColor(node: GraphNode): string | null {
    for (const group of colorGroups) {
      if (matchesGroupQuery(node, group.query)) {
        return group.color;
      }
    }
    return null;
  }

  let filteredGraph = $derived.by(() => {
    const allNodes = rawGraph.nodes;
    const allEdges = rawGraph.edges;
    let candidateIds: Set<string> | null = null;

    if (mode === 'local') {
      candidateIds = new Set<string>();
      if (activeNotePath && allNodes.some((n) => n.id === activeNotePath)) {
        const adj = new Map<string, string[]>();
        for (const edge of allEdges) {
          if (!adj.has(edge.source)) adj.set(edge.source, []);
          if (!adj.has(edge.target)) adj.set(edge.target, []);
          adj.get(edge.source)!.push(edge.target);
          adj.get(edge.target)!.push(edge.source);
        }
        const queue: Array<{ id: string; depth: number }> = [{ id: activeNotePath, depth: 0 }];
        candidateIds.add(activeNotePath);
        while (queue.length > 0) {
          const cur = queue.shift()!;
          if (cur.depth >= localDepth) continue;
          for (const nextId of adj.get(cur.id) ?? []) {
            if (!candidateIds.has(nextId)) {
              candidateIds.add(nextId);
              queue.push({ id: nextId, depth: cur.depth + 1 });
            }
          }
        }
      }
    }

    const q = searchQuery.trim().toLowerCase();
    const visibleNodes: GraphNode[] = [];
    const visibleSet = new Set<string>();

    for (const node of allNodes) {
      if (candidateIds !== null && !candidateIds.has(node.id)) continue;
      if (!showUnresolved && !node.exists) continue;
      if (!showOrphans && node.degree === 0 && node.id !== activeNotePath) continue;

      if (tagFilter) {
        const matchesTag = node.tags.some(
          (t) => t === tagFilter || t.startsWith(`${tagFilter}/`),
        );
        if (!matchesTag) continue;
      }

      if (folderFilter) {
        if (!node.exists) continue;
        const folderName = node.folder || '(raíz)';
        if (folderName !== folderFilter && !folderName.startsWith(`${folderFilter}/`)) continue;
      }

      if (q) {
        const matchesText =
          node.label.toLowerCase().includes(q) ||
          node.id.toLowerCase().includes(q) ||
          node.tags.some((t) => `#${t}`.includes(q));
        if (!matchesText) continue;
      }

      visibleNodes.push(node);
      visibleSet.add(node.id);
    }

    const visibleEdges: GraphEdge[] = allEdges.filter(
      (e) => visibleSet.has(e.source) && visibleSet.has(e.target),
    );

    return { nodes: visibleNodes, edges: visibleEdges };
  });

  function computeNodeRadius(node: GraphNode): number {
    const base = node.exists ? 6 : 4.5;
    const degBoost = Math.min(9, Math.sqrt(Math.max(0, node.degree)) * 2.1);
    const activeBoost = node.id === activeNotePath ? 2.5 : 0;
    return base + degBoost + activeBoost;
  }

  function rebuildSimulation(
    nodes: GraphNode[],
    edges: GraphEdge[],
    preservePositions: boolean,
  ) {
    const prevById = preservePositions ? simNodeById : new Map<string, SimNode>();
    const nextNodes: SimNode[] = [];
    const nextMap = new Map<string, SimNode>();

    nodes.forEach((node, idx) => {
      const prev = prevById.get(node.id);
      const isCenter = mode === 'local' && node.id === activeNotePath;
      let x = 0;
      let y = 0;
      if (prev) {
        x = prev.x;
        y = prev.y;
      } else if (!isCenter) {
        const angle = idx * 2.39996323;
        const dist = 34 * Math.sqrt(idx + 1);
        x = Math.cos(angle) * dist;
        y = Math.sin(angle) * dist;
      }
      const sn: SimNode = {
        ...node,
        x,
        y,
        vx: prev ? prev.vx * 0.3 : 0,
        vy: prev ? prev.vy * 0.3 : 0,
        radius: computeNodeRadius(node),
        color: resolveNodeColor(node),
      };
      nextNodes.push(sn);
      nextMap.set(node.id, sn);
    });

    const nextEdges: Array<{ source: SimNode; target: SimNode; resolved: boolean }> = [];
    const nextNeighbors = new Map<string, Set<string>>();
    for (const n of nextNodes) nextNeighbors.set(n.id, new Set());

    for (const e of edges) {
      const s = nextMap.get(e.source);
      const t = nextMap.get(e.target);
      if (s && t) {
        nextEdges.push({ source: s, target: t, resolved: e.resolved });
        nextNeighbors.get(s.id)?.add(t.id);
        nextNeighbors.get(t.id)?.add(s.id);
      }
    }

    simNodes = nextNodes;
    simNodeById = nextMap;
    simEdges = nextEdges;
    neighborMap = nextNeighbors;

    if (currentHoverId && !nextMap.has(currentHoverId)) {
      currentHoverId = null;
      hoveredNode = null;
    }

    kickSimulation(0.85);
  }

  function kickSimulation(targetAlpha = 0.85) {
    alpha = Math.max(alpha, targetAlpha);
    settling = true;
    if (!rafId) {
      rafId = requestAnimationFrame(animationLoop);
    }
  }

  function getThemePalette() {
    const cs = getComputedStyle(document.documentElement);
    const val = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
    const isLight = theme === 'light';
    return {
      bg: val('--bg-editor', isLight ? '#ffffff' : '#1e1e2e'),
      grid: isLight ? 'rgba(15, 23, 42, 0.06)' : 'rgba(255, 255, 255, 0.05)',
      nodeDefault: isLight ? '#64748b' : '#94a3b8',
      nodeGhost: isLight ? '#cbd5e1' : '#45475a',
      accent: val('--accent', '#7c6cff'),
      text: val('--text-primary', isLight ? '#1e293b' : '#e2e8f0'),
      textMuted: val('--text-secondary', isLight ? '#64748b' : '#94a3b8'),
      edge: isLight ? 'rgba(100, 116, 139, 0.28)' : 'rgba(148, 163, 184, 0.24)',
      edgeUnresolved: isLight ? 'rgba(148, 163, 184, 0.22)' : 'rgba(100, 116, 139, 0.22)',
      edgeHighlight: val('--accent', '#7c6cff'),
      labelBg: isLight ? 'rgba(255, 255, 255, 0.82)' : 'rgba(24, 24, 37, 0.78)',
    };
  }

  function stepPhysics() {
    const n = simNodes.length;
    if (n === 0) return;

    const repulsion = repulsionForce * 12;
    const targetLen = linkDistance;
    const kSpring = 0.045;
    const gravity = mode === 'local' ? 0.022 : 0.015;
    const maxCutoffSq = n > 140 ? 420 * 420 : Infinity;

    // Gravedad hacia el origen (más fuerte para huérfanas)
    for (let i = 0; i < n; i++) {
      const a = simNodes[i];
      if (dragMode === 'node' && draggedNode === a) continue;
      const gFactor = a.degree === 0 ? gravity * 2.2 : gravity;
      a.vx += -a.x * gFactor * alpha;
      a.vy += -a.y * gFactor * alpha;
    }

    // Repulsión entre pares + colisión mínima
    for (let i = 0; i < n; i++) {
      const a = simNodes[i];
      for (let j = i + 1; j < n; j++) {
        const b = simNodes[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let distSq = dx * dx + dy * dy;
        if (distSq > maxCutoffSq) continue;
        if (distSq < 1) {
          dx = (Math.random() - 0.5) * 2 || 0.5;
          dy = (Math.random() - 0.5) * 2 || 0.5;
          distSq = dx * dx + dy * dy;
        }
        const dist = Math.sqrt(distSq);
        const ux = dx / dist;
        const uy = dy / dist;

        const repForce = (repulsion / Math.max(distSq, 64)) * alpha;
        const minGap = a.radius + b.radius + 10;
        const push = dist < minGap ? (minGap - dist) * 0.22 : 0;

        const fx = ux * (repForce + push);
        const fy = uy * (repForce + push);

        if (!(dragMode === 'node' && draggedNode === a)) {
          a.vx -= fx;
          a.vy -= fy;
        }
        if (!(dragMode === 'node' && draggedNode === b)) {
          b.vx += fx;
          b.vy += fy;
        }
      }
    }

    // Resortes en aristas
    for (const edge of simEdges) {
      const a = edge.source;
      const b = edge.target;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const diff = (dist - targetLen) * kSpring * alpha;
      const fx = (dx / dist) * diff;
      const fy = (dy / dist) * diff;

      if (!(dragMode === 'node' && draggedNode === a)) {
        a.vx += fx;
        a.vy += fy;
      }
      if (!(dragMode === 'node' && draggedNode === b)) {
        b.vx -= fx;
        b.vy -= fy;
      }
    }

    // Integración y amortiguamiento
    for (let i = 0; i < n; i++) {
      const node = simNodes[i];
      if (dragMode === 'node' && draggedNode === node) {
        node.vx = 0;
        node.vy = 0;
        continue;
      }
      node.vx *= 0.82;
      node.vy *= 0.82;
      node.x += Math.max(-24, Math.min(24, node.vx));
      node.y += Math.max(-24, Math.min(24, node.vy));
    }

    if (dragMode === 'node') {
      alpha = Math.max(alpha, 0.25);
    } else {
      alpha *= 0.985;
    }
  }

  function worldToScreen(wx: number, wy: number) {
    return {
      x: viewportW / 2 + (wx - camX) * camZoom,
      y: viewportH / 2 + (wy - camY) * camZoom,
    };
  }

  function screenToWorld(sx: number, sy: number) {
    return {
      x: (sx - viewportW / 2) / camZoom + camX,
      y: (sy - viewportH / 2) / camZoom + camY,
    };
  }

  function drawCanvas() {
    if (!canvasEl) return;
    const ctx = canvasEl.getContext('2d');
    if (!ctx) return;

    const palette = getThemePalette();
    ctx.clearRect(0, 0, viewportW, viewportH);

    // Fondo + grilla sutil en espacio del mundo
    ctx.fillStyle = palette.bg;
    ctx.fillRect(0, 0, viewportW, viewportH);

    const gridStep = 48 * camZoom;
    if (gridStep >= 14) {
      const origin = worldToScreen(0, 0);
      const offsetX = ((origin.x % gridStep) + gridStep) % gridStep;
      const offsetY = ((origin.y % gridStep) + gridStep) % gridStep;
      ctx.fillStyle = palette.grid;
      for (let gx = offsetX; gx < viewportW; gx += gridStep) {
        for (let gy = offsetY; gy < viewportH; gy += gridStep) {
          ctx.fillRect(gx - 0.75, gy - 0.75, 1.5, 1.5);
        }
      }
    }

    const focusId = currentHoverId;
    const focusNeighbors = focusId ? neighborMap.get(focusId) : null;

    ctx.save();
    ctx.translate(viewportW / 2, viewportH / 2);
    ctx.scale(camZoom, camZoom);
    ctx.translate(-camX, -camY);

    // Dibujar aristas
    for (const edge of simEdges) {
      const isConnected =
        focusId !== null && (edge.source.id === focusId || edge.target.id === focusId);

      ctx.save();
      if (focusId !== null) {
        ctx.globalAlpha = isConnected ? 0.92 : 0.1;
      }
      ctx.beginPath();
      ctx.moveTo(edge.source.x, edge.source.y);
      ctx.lineTo(edge.target.x, edge.target.y);
      if (!edge.resolved) {
        ctx.setLineDash([4 / camZoom, 4 / camZoom]);
        ctx.strokeStyle = isConnected ? palette.edgeHighlight : palette.edgeUnresolved;
      } else {
        ctx.strokeStyle = isConnected ? palette.edgeHighlight : palette.edge;
      }
      ctx.lineWidth = (isConnected ? 2 : 1.1) / Math.max(camZoom, 0.55);
      ctx.stroke();
      ctx.restore();
    }

    // Dibujar nodos
    for (const node of simNodes) {
      const isHovered = node.id === focusId;
      const isNeighbor = focusNeighbors ? focusNeighbors.has(node.id) : false;
      const isActiveNote = node.id === activeNotePath;
      const dimmed = focusId !== null && !isHovered && !isNeighbor;

      ctx.save();
      if (dimmed) ctx.globalAlpha = 0.22;

      const r = node.radius;
      const fillColor = node.color ?? (node.exists ? palette.nodeDefault : palette.nodeGhost);

      // Halo de foco / nota activa
      if (isActiveNote || isHovered) {
        ctx.beginPath();
        ctx.arc(node.x, node.y, r + 4.5 / Math.max(camZoom, 0.6), 0, Math.PI * 2);
        ctx.strokeStyle = palette.accent;
        ctx.lineWidth = 2.2 / Math.max(camZoom, 0.6);
        ctx.stroke();
      }

      ctx.beginPath();
      ctx.arc(node.x, node.y, r, 0, Math.PI * 2);
      if (node.exists) {
        ctx.fillStyle = fillColor;
        ctx.fill();
      } else {
        ctx.fillStyle = palette.bg;
        ctx.fill();
        ctx.setLineDash([2.5 / camZoom, 2 / camZoom]);
        ctx.strokeStyle = fillColor;
        ctx.lineWidth = 1.6 / Math.max(camZoom, 0.6);
        ctx.stroke();
      }

      ctx.restore();
    }

    // Dibujar etiquetas (progresivo según zoom / relevancia para evitar ruido)
    const showAllLabels = camZoom >= 0.9 || simNodes.length <= 45;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    const fontSize = Math.max(9.5, Math.min(13, 11.5 / Math.pow(camZoom, 0.35)));
    ctx.font = `500 ${fontSize}px Inter, system-ui, sans-serif`;

    for (const node of simNodes) {
      const isHovered = node.id === focusId;
      const isNeighbor = focusNeighbors ? focusNeighbors.has(node.id) : false;
      const isActiveNote = node.id === activeNotePath;
      const important = node.degree >= 3 || isHovered || isNeighbor || isActiveNote;
      if (!showAllLabels && !important) continue;

      const dimmed = focusId !== null && !isHovered && !isNeighbor;
      ctx.save();
      ctx.globalAlpha = dimmed ? 0.18 : node.exists ? 0.95 : 0.72;

      const labelY = node.y + node.radius + 4 / Math.max(camZoom, 0.65);
      const text = node.label.length > 34 ? `${node.label.slice(0, 32)}…` : node.label;

      if (isHovered || isActiveNote) {
        const metrics = ctx.measureText(text);
        const padX = 5 / Math.max(camZoom, 0.7);
        const boxH = fontSize + 4;
        ctx.fillStyle = palette.labelBg;
        ctx.fillRect(
          node.x - metrics.width / 2 - padX,
          labelY - 2,
          metrics.width + padX * 2,
          boxH,
        );
      }

      ctx.fillStyle = isHovered || isActiveNote ? palette.text : palette.textMuted;
      ctx.fillText(text, node.x, labelY);
      ctx.restore();
    }

    ctx.restore();
  }

  function animationLoop() {
    rafId = 0;
    if (alpha > 0.012 || dragMode === 'node') {
      stepPhysics();
    } else {
      alpha = 0;
      settling = false;
    }
    drawCanvas();
    if (alpha > 0.012 || dragMode === 'node') {
      rafId = requestAnimationFrame(animationLoop);
    }
  }

  function scheduleRedraw() {
    if (!rafId) {
      rafId = requestAnimationFrame(animationLoop);
    }
  }

  function fitView() {
    if (simNodes.length === 0) {
      camX = 0;
      camY = 0;
      camZoom = 1;
      zoomPercent = 100;
      drawCanvas();
      return;
    }
    let minX = Infinity;
    let maxX = -Infinity;
    let minY = Infinity;
    let maxY = -Infinity;
    for (const node of simNodes) {
      minX = Math.min(minX, node.x - node.radius);
      maxX = Math.max(maxX, node.x + node.radius);
      minY = Math.min(minY, node.y - node.radius);
      maxY = Math.max(maxY, node.y + node.radius);
    }
    const spanW = Math.max(120, maxX - minX + 120);
    const spanH = Math.max(120, maxY - minY + 120);
    camX = (minX + maxX) / 2;
    camY = (minY + maxY) / 2;
    camZoom = Math.max(0.22, Math.min(2.2, Math.min(viewportW / spanW, viewportH / spanH)));
    zoomPercent = Math.round(camZoom * 100);
    drawCanvas();
  }

  function adjustZoom(factor: number, centerX = viewportW / 2, centerY = viewportH / 2) {
    const before = screenToWorld(centerX, centerY);
    const nextZoom = Math.max(0.2, Math.min(4.0, camZoom * factor));
    camZoom = nextZoom;
    zoomPercent = Math.round(camZoom * 100);
    const after = screenToWorld(centerX, centerY);
    camX += before.x - after.x;
    camY += before.y - after.y;
    drawCanvas();
  }

  function findNodeAtScreen(sx: number, sy: number): SimNode | null {
    const world = screenToWorld(sx, sy);
    const hitSlack = 5 / Math.max(camZoom, 0.5);
    for (let i = simNodes.length - 1; i >= 0; i--) {
      const node = simNodes[i];
      const dx = world.x - node.x;
      const dy = world.y - node.y;
      const r = node.radius + hitSlack;
      if (dx * dx + dy * dy <= r * r) {
        return node;
      }
    }
    return null;
  }

  function handlePointerDown(event: PointerEvent) {
    if (!canvasEl || event.button !== 0) return;
    const rect = canvasEl.getBoundingClientRect();
    const sx = event.clientX - rect.left;
    const sy = event.clientY - rect.top;

    pointerStartX = sx;
    pointerStartY = sy;
    pointerMoved = false;

    const hit = findNodeAtScreen(sx, sy);
    if (hit) {
      dragMode = 'node';
      draggedNode = hit;
      kickSimulation(0.35);
    } else {
      dragMode = 'pan';
      draggedNode = null;
      panStartX = camX;
      panStartY = camY;
    }
    canvasEl.setPointerCapture(event.pointerId);
  }

  function handlePointerMove(event: PointerEvent) {
    if (!canvasEl) return;
    const rect = canvasEl.getBoundingClientRect();
    const sx = event.clientX - rect.left;
    const sy = event.clientY - rect.top;

    if (dragMode === 'node' && draggedNode) {
      if (Math.hypot(sx - pointerStartX, sy - pointerStartY) > 4) {
        pointerMoved = true;
      }
      const world = screenToWorld(sx, sy);
      draggedNode.x = world.x;
      draggedNode.y = world.y;
      draggedNode.vx = 0;
      draggedNode.vy = 0;
      scheduleRedraw();
      return;
    }

    if (dragMode === 'pan') {
      if (Math.hypot(sx - pointerStartX, sy - pointerStartY) > 3) {
        pointerMoved = true;
      }
      camX = panStartX - (sx - pointerStartX) / camZoom;
      camY = panStartY - (sy - pointerStartY) / camZoom;
      drawCanvas();
      return;
    }

    const hit = findNodeAtScreen(sx, sy);
    const nextHoverId = hit?.id ?? null;
    if (nextHoverId !== currentHoverId) {
      currentHoverId = nextHoverId;
      hoveredNode = hit ? { ...hit } : null;
      drawCanvas();
    }
  }

  function handlePointerUp(event: PointerEvent) {
    if (!canvasEl) return;
    if (canvasEl.hasPointerCapture(event.pointerId)) {
      canvasEl.releasePointerCapture(event.pointerId);
    }
    if (dragMode === 'node' && draggedNode && !pointerMoved) {
      const target = draggedNode;
      if (target.exists) {
        onOpenNote(target.id);
      } else {
        onOpenWikilink(target.label);
      }
    }
    dragMode = 'none';
    draggedNode = null;
  }

  function handlePointerLeave() {
    if (dragMode === 'none' && currentHoverId !== null) {
      currentHoverId = null;
      hoveredNode = null;
      drawCanvas();
    }
  }

  function handleWheel(event: WheelEvent) {
    event.preventDefault();
    if (!canvasEl) return;
    const rect = canvasEl.getBoundingClientRect();
    const sx = event.clientX - rect.left;
    const sy = event.clientY - rect.top;
    const factor = event.deltaY < 0 ? 1.12 : 0.89;
    adjustZoom(factor, sx, sy);
  }

  function switchMode(next: 'global' | 'local') {
    if (onModeChange) {
      onModeChange(next);
    } else {
      mode = next;
    }
    camX = 0;
    camY = 0;
    camZoom = 1;
    zoomPercent = 100;
  }

  function addColorGroup() {
    const clean = newGroupQuery.trim();
    if (!clean) return;
    const next = [
      ...colorGroups,
      { id: `${Date.now()}-${Math.random().toString(36).slice(2, 6)}`, query: clean, color: newGroupColor },
    ];
    persistColorGroups(next);
    newGroupQuery = '';
    newGroupColor = PRESET_COLORS[next.length % PRESET_COLORS.length];
  }

  function removeColorGroup(id: string) {
    persistColorGroups(colorGroups.filter((g) => g.id !== id));
  }

  function resetAllFilters() {
    searchQuery = '';
    tagFilter = '';
    folderFilter = '';
    showOrphans = true;
    showUnresolved = true;
  }

  async function loadGraph() {
    loading = true;
    errorMsg = null;
    try {
      rawGraph = await getGraphData();
    } catch (err) {
      errorMsg = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }

  function syncCanvasSize() {
    if (!canvasEl || !stageEl) return;
    const rect = stageEl.getBoundingClientRect();
    const width = Math.max(240, Math.floor(rect.width));
    const height = Math.max(240, Math.floor(rect.height));
    const dpr = window.devicePixelRatio || 1;
    viewportW = width;
    viewportH = height;
    canvasEl.width = Math.floor(width * dpr);
    canvasEl.height = Math.floor(height * dpr);
    canvasEl.style.width = `${width}px`;
    canvasEl.style.height = `${height}px`;
    const ctx = canvasEl.getContext('2d');
    if (ctx) {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    }
    drawCanvas();
  }

  onMount(() => {
    syncCanvasSize();
    const ro = new ResizeObserver(() => syncCanvasSize());
    if (stageEl) ro.observe(stageEl);
    return () => {
      ro.disconnect();
      if (rafId) cancelAnimationFrame(rafId);
    };
  });

  // Recargar datos al abrir o cuando cambia la bóveda / refreshToken
  $effect(() => {
    void refreshToken;
    void loadGraph();
  });

  // Reconstruir simulación cuando cambian los nodos/aristas filtrados o grupos de color
  $effect(() => {
    const graph = filteredGraph;
    void colorGroups;
    void mode;
    void activeNotePath;
    untrack(() => {
      rebuildSimulation(graph.nodes, graph.edges, true);
    });
  });

  // Despertar simulación al ajustar parámetros físicos
  $effect(() => {
    void linkDistance;
    void repulsionForce;
    untrack(() => {
      if (simNodes.length > 0) kickSimulation(0.45);
    });
  });

  // Redibujar al cambiar tema claro/oscuro
  $effect(() => {
    void theme;
    untrack(() => drawCanvas());
  });
</script>

<section class="graph-shell" aria-label="Vista de Grafo">
  <header class="graph-header">
    <div class="graph-mode-tabs" role="tablist" aria-label="Modo del grafo">
      <button
        type="button"
        role="tab"
        aria-selected={mode === 'global'}
        class="mode-btn"
        class:active={mode === 'global'}
        onclick={() => switchMode('global')}
      >
        Grafo Global
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={mode === 'local'}
        class="mode-btn"
        class:active={mode === 'local'}
        onclick={() => switchMode('local')}
        disabled={!activeNotePath}
        title={activeNotePath ? `Vecindario de ${activeNotePath}` : 'Abre una nota primero'}
      >
        Grafo Local
      </button>
    </div>

    <div class="graph-stats">
      <span>{filteredGraph.nodes.length} nodos</span>
      <span>·</span>
      <span>{filteredGraph.edges.length} enlaces</span>
      {#if settling}
        <span class="settling-dot" title="Simulación activa">●</span>
      {/if}
    </div>

    <div class="graph-actions">
      <button
        type="button"
        class="tool-btn"
        class:active={filtersOpen}
        onclick={() => (filtersOpen = !filtersOpen)}
        title="Mostrar u ocultar filtros y grupos de color"
      >
        Filtros
      </button>
      <button
        type="button"
        class="tool-btn"
        onclick={() => rebuildSimulation(filteredGraph.nodes, filteredGraph.edges, false)}
        title="Recalcular simulación desde el centro"
      >
        Reordenar
      </button>
      <button type="button" class="tool-btn" onclick={fitView} title="Encuadrar todos los nodos">
        Encuadrar
      </button>
      <div class="zoom-group">
        <button type="button" class="zoom-btn" onclick={() => adjustZoom(0.84)} title="Alejar">
          −
        </button>
        <span class="zoom-label">{zoomPercent}%</span>
        <button type="button" class="zoom-btn" onclick={() => adjustZoom(1.19)} title="Acercar">
          +
        </button>
      </div>
      {#if onClose}
        <button type="button" class="close-btn" onclick={onClose} title="Volver al editor Markdown">
          Volver al editor
        </button>
      {/if}
    </div>
  </header>

  <div class="graph-body">
    <div class="graph-stage" bind:this={stageEl}>
      {#if loading}
        <div class="graph-banner">Cargando grafo de la bóveda…</div>
      {:else if errorMsg}
        <div class="graph-banner error">{errorMsg}</div>
      {:else if mode === 'local' && !activeNotePath}
        <div class="graph-empty">
          <p>No hay ninguna nota activa para mostrar su Grafo Local.</p>
          <button type="button" class="tool-btn" onclick={() => switchMode('global')}>
            Ver Grafo Global
          </button>
        </div>
      {:else if filteredGraph.nodes.length === 0}
        <div class="graph-empty">
          <p>Ningún nodo coincide con los filtros actuales.</p>
          <button type="button" class="tool-btn" onclick={resetAllFilters}>
            Limpiar filtros
          </button>
        </div>
      {/if}

      <canvas
        bind:this={canvasEl}
        class="graph-canvas"
        class:can-click={hoveredNode !== null}
        onpointerdown={handlePointerDown}
        onpointermove={handlePointerMove}
        onpointerup={handlePointerUp}
        onpointerleave={handlePointerLeave}
        onwheel={handleWheel}
      ></canvas>

      {#if hoveredNode}
        <aside class="hover-card">
          <div class="hover-title">
            {hoveredNode.label}
            {#if !hoveredNode.exists}
              <span class="ghost-badge">Sin crear</span>
            {/if}
          </div>
          <div class="hover-meta">
            <span>{hoveredNode.degree} {hoveredNode.degree === 1 ? 'conexión' : 'conexiones'}</span>
            {#if hoveredNode.exists}
              <span>· {hoveredNode.id}</span>
            {/if}
          </div>
          {#if hoveredNode.tags.length > 0}
            <div class="hover-tags">
              {#each hoveredNode.tags as tag}
                <span class="hover-tag">#{tag}</span>
              {/each}
            </div>
          {/if}
          <div class="hover-hint">
            {hoveredNode.exists ? 'Clic para abrir nota · Arrastra para mover' : 'Clic para crear nota'}
          </div>
        </aside>
      {/if}
    </div>

    {#if filtersOpen}
      <aside class="graph-controls" aria-label="Filtros del grafo">
        {#if mode === 'local'}
          <div class="control-section">
            <div class="section-title">Vecindario local</div>
            <label class="range-row">
              <span>Profundidad: {localDepth} {localDepth === 1 ? 'salto' : 'saltos'}</span>
              <input type="range" min="1" max="3" step="1" bind:value={localDepth} />
            </label>
          </div>
        {/if}

        <div class="control-section">
          <div class="section-title">Filtros</div>
          <input
            type="search"
            class="control-input"
            bind:value={searchQuery}
            placeholder="Buscar nodo o #etiqueta…"
          />

          <label class="select-field">
            <span>Etiqueta</span>
            <select bind:value={tagFilter}>
              <option value="">Todas las etiquetas</option>
              {#each availableTags as item}
                <option value={item.tag}>#{item.tag} ({item.count})</option>
              {/each}
            </select>
          </label>

          <label class="select-field">
            <span>Carpeta</span>
            <select bind:value={folderFilter}>
              <option value="">Todas las carpetas</option>
              {#each availableFolders as folder}
                <option value={folder}>{folder}</option>
              {/each}
            </select>
          </label>

          <label class="check-row">
            <input type="checkbox" bind:checked={showOrphans} />
            <span>Mostrar huérfanas</span>
          </label>
          <label class="check-row">
            <input type="checkbox" bind:checked={showUnresolved} />
            <span>Mostrar enlaces sin crear</span>
          </label>
        </div>

        <div class="control-section">
          <div class="section-title">Grupos de color</div>
          <div class="group-add-row">
            <input
              type="text"
              class="control-input"
              bind:value={newGroupQuery}
              placeholder="#etiqueta o folder:ruta"
              onkeydown={(e) => {
                if (e.key === 'Enter') addColorGroup();
              }}
            />
            <input
              type="color"
              class="color-picker"
              bind:value={newGroupColor}
              title="Color del grupo"
            />
            <button type="button" class="mini-btn" onclick={addColorGroup}>+</button>
          </div>

          {#if colorGroups.length === 0}
            <p class="empty-hint">Sin reglas. Usa `#etiqueta` o `folder:carpeta`.</p>
          {:else}
            <ul class="group-list">
              {#each colorGroups as group (group.id)}
                <li class="group-item">
                  <span class="color-dot" style={`background:${group.color}`}></span>
                  <span class="group-query" title={group.query}>{group.query}</span>
                  <button
                    type="button"
                    class="remove-group"
                    onclick={() => removeColorGroup(group.id)}
                    title="Quitar grupo"
                  >
                    ✕
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>

        <div class="control-section">
          <div class="section-title">Física 2D</div>
          <label class="range-row">
            <span>Repulsión ({repulsionForce})</span>
            <input type="range" min="80" max="500" step="10" bind:value={repulsionForce} />
          </label>
          <label class="range-row">
            <span>Longitud de enlace ({linkDistance}px)</span>
            <input type="range" min="50" max="220" step="5" bind:value={linkDistance} />
          </label>
        </div>
      </aside>
    {/if}
  </div>
</section>

<style>
  .graph-shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg-editor);
    color: var(--text-primary);
  }

  .graph-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem;
    padding: 0.5rem 0.85rem;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-sidebar);
  }

  .graph-mode-tabs {
    display: flex;
    gap: 0.25rem;
    background: var(--bg-editor);
    border: 1px solid var(--border-subtle);
    border-radius: 7px;
    padding: 2px;
  }

  .mode-btn {
    border: 0;
    background: transparent;
    color: var(--text-secondary);
    padding: 0.28rem 0.65rem;
    border-radius: 5px;
    font-size: 0.78rem;
    font-weight: 500;
    cursor: pointer;
  }

  .mode-btn.active {
    background: var(--accent);
    color: #fff;
  }

  .mode-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .graph-stats {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.76rem;
    color: var(--text-secondary);
  }

  .settling-dot {
    color: var(--accent);
    font-size: 0.65rem;
  }

  .graph-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
  }

  .tool-btn,
  .close-btn,
  .mini-btn {
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    color: var(--text-primary);
    padding: 0.26rem 0.6rem;
    border-radius: 6px;
    font-size: 0.76rem;
    cursor: pointer;
  }

  .tool-btn.active {
    border-color: var(--accent);
    color: var(--accent);
  }

  .tool-btn:hover,
  .close-btn:hover,
  .mini-btn:hover,
  .zoom-btn:hover {
    background: var(--bg-hover);
  }

  .zoom-group {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--border-subtle);
    border-radius: 6px;
    background: var(--bg-editor);
    overflow: hidden;
  }

  .zoom-btn {
    border: 0;
    background: transparent;
    color: var(--text-primary);
    width: 24px;
    height: 24px;
    cursor: pointer;
    font-size: 0.85rem;
  }

  .zoom-label {
    font-size: 0.72rem;
    color: var(--text-secondary);
    padding: 0 0.35rem;
    min-width: 40px;
    text-align: center;
  }

  .graph-body {
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: hidden;
  }

  .graph-stage {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  .graph-canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: grab;
    touch-action: none;
  }

  .graph-canvas:active {
    cursor: grabbing;
  }

  .graph-canvas.can-click {
    cursor: pointer;
  }

  .graph-banner,
  .graph-empty {
    position: absolute;
    z-index: 4;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    background: var(--bg-sidebar);
    border: 1px solid var(--border-subtle);
    border-radius: 10px;
    padding: 0.9rem 1.2rem;
    font-size: 0.84rem;
    color: var(--text-secondary);
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    align-items: center;
    box-shadow: 0 10px 26px rgba(0, 0, 0, 0.22);
  }

  .graph-banner.error {
    color: #f85149;
  }

  .hover-card {
    position: absolute;
    left: 0.85rem;
    bottom: 0.85rem;
    z-index: 5;
    max-width: 320px;
    background: var(--bg-sidebar);
    border: 1px solid var(--border-subtle);
    border-radius: 9px;
    padding: 0.6rem 0.75rem;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.26);
    pointer-events: none;
    display: flex;
    flex-direction: column;
    gap: 0.28rem;
  }

  .hover-title {
    font-size: 0.84rem;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .ghost-badge {
    font-size: 0.66rem;
    padding: 0.08rem 0.35rem;
    border-radius: 999px;
    border: 1px dashed var(--border-subtle);
    color: var(--text-secondary);
  }

  .hover-meta {
    font-size: 0.72rem;
    color: var(--text-secondary);
    word-break: break-word;
  }

  .hover-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
  }

  .hover-tag {
    font-size: 0.68rem;
    padding: 0.05rem 0.35rem;
    border-radius: 4px;
    background: var(--bg-hover);
    color: var(--accent);
  }

  .hover-hint {
    font-size: 0.68rem;
    color: var(--text-secondary);
    opacity: 0.85;
  }

  .graph-controls {
    width: 248px;
    flex-shrink: 0;
    border-left: 1px solid var(--border-subtle);
    background: var(--bg-sidebar);
    padding: 0.75rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
  }

  .control-section {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .section-title {
    font-size: 0.72rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
  }

  .control-input,
  .select-field select {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid var(--border-subtle);
    background: var(--bg-editor);
    color: var(--text-primary);
    border-radius: 6px;
    padding: 0.32rem 0.48rem;
    font-size: 0.76rem;
  }

  .select-field,
  .range-row {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.74rem;
    color: var(--text-secondary);
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.76rem;
    color: var(--text-primary);
    cursor: pointer;
  }

  .group-add-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .color-picker {
    width: 28px;
    height: 26px;
    padding: 1px;
    border: 1px solid var(--border-subtle);
    border-radius: 5px;
    background: var(--bg-editor);
    cursor: pointer;
    flex-shrink: 0;
  }

  .empty-hint {
    margin: 0;
    font-size: 0.72rem;
    color: var(--text-secondary);
  }

  .group-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .group-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.24rem 0.42rem;
    background: var(--bg-editor);
    border: 1px solid var(--border-subtle);
    border-radius: 6px;
    font-size: 0.74rem;
  }

  .color-dot {
    width: 10px;
    height: 10px;
    border-radius: 999px;
    flex-shrink: 0;
  }

  .group-query {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .remove-group {
    border: 0;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 0.72rem;
    padding: 0 0.15rem;
  }

  @media (max-width: 900px) {
    .graph-body {
      position: relative;
    }

    .graph-controls {
      position: absolute;
      inset: 0 0 0 auto;
      width: min(260px, 85vw);
      z-index: 15;
      box-shadow: -8px 0 24px rgba(0, 0, 0, 0.35);
    }
  }
</style>
