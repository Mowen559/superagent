import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { GraphNode } from 'gitnexus-shared';
import { useAppState } from '../hooks/useAppState';
import { useUAFeatures } from '../hooks/useUAFeatures';

interface PathFinderModalProps {
  isOpen: boolean;
  onClose: () => void;
}

/**
 * Dependency Path Finder — ported from Understand-Anything.
 *
 * Runs a client-side BFS over the UA-adapted semantic graph to find the
 * shortest path between two nodes. Clicking a node in the result selects the
 * corresponding GitNexus node via `useAppState` (which syncs the sigma camera
 * through the existing selection effect).
 */
export default function PathFinderModal({ isOpen, onClose }: PathFinderModalProps) {
  const { uaGraph } = useUAFeatures();
  const { graph, setSelectedNode, openCodePanel } = useAppState();
  const { t } = useTranslation(['common']);

  const [fromNodeId, setFromNodeId] = useState('');
  const [toNodeId, setToNodeId] = useState('');
  const [path, setPath] = useState<string[] | null>(null);
  const [searching, setSearching] = useState(false);
  const modalRef = useRef<HTMLDivElement>(null);

  // Map GN node ids → GN nodes for selection on click.
  const gnNodeById = useMemo(() => {
    const map = new Map<string, GraphNode>();
    if (graph) graph.nodes.forEach((n) => map.set(n.id, n));
    return map;
  }, [graph]);

  useEffect(() => {
    if (!isOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (modalRef.current && !modalRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [isOpen, onClose]);

  useEffect(() => {
    if (!isOpen) return;
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [isOpen, onClose]);

  if (!isOpen || !uaGraph) return null;

  const nodes = uaGraph.nodes;
  const edges = uaGraph.edges;
  const nodeMap = new Map(nodes.map((n) => [n.id, n]));

  const findPath = () => {
    if (!fromNodeId || !toNodeId || fromNodeId === toNodeId) {
      setPath(null);
      return;
    }
    setSearching(true);

    // Bidirectional adjacency list.
    const adjacency = new Map<string, string[]>();
    for (const edge of edges) {
      if (!adjacency.has(edge.source)) adjacency.set(edge.source, []);
      adjacency.get(edge.source)!.push(edge.target);
      if (!adjacency.has(edge.target)) adjacency.set(edge.target, []);
      adjacency.get(edge.target)!.push(edge.source);
    }

    const queue: Array<{ nodeId: string; path: string[] }> = [
      { nodeId: fromNodeId, path: [fromNodeId] },
    ];
    const visited = new Set<string>([fromNodeId]);

    while (queue.length > 0) {
      const { nodeId, path: currentPath } = queue.shift()!;
      if (nodeId === toNodeId) {
        setPath(currentPath);
        setSearching(false);
        return;
      }
      const neighbors = adjacency.get(nodeId) ?? [];
      for (const neighbor of neighbors) {
        if (!visited.has(neighbor)) {
          visited.add(neighbor);
          queue.push({ nodeId: neighbor, path: [...currentPath, neighbor] });
        }
      }
    }

    setPath([]);
    setSearching(false);
  };

  const handleNodeClick = (nodeId: string) => {
    const gnNode = gnNodeById.get(nodeId);
    if (gnNode) {
      setSelectedNode(gnNode);
      openCodePanel();
    }
    onClose();
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-void/80 backdrop-blur-sm">
      <div
        ref={modalRef}
        className="max-h-[80vh] w-full max-w-2xl animate-fade-in overflow-hidden rounded-xl border border-border-default bg-elevated shadow-2xl"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border-default px-5 py-4">
          <div className="flex items-center gap-3">
            <svg
              className="h-5 w-5 text-accent"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"
              />
            </svg>
            <h2 className="text-xl font-semibold text-text-primary">
              {t('pathFinder.title', 'Dependency Path Finder')}
            </h2>
          </div>
          <button
            onClick={onClose}
            className="text-text-secondary transition-colors hover:text-text-primary"
          >
            <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>

        {/* Body */}
        <div className="max-h-[calc(80vh-180px)] space-y-4 overflow-y-auto p-5">
          <p className="text-sm text-text-secondary">
            {t(
              'pathFinder.description',
              'Find the shortest path between two nodes in the dependency graph.',
            )}
          </p>

          {/* From / To */}
          {(['from', 'to'] as const).map((which) => (
            <div key={which}>
              <label className="mb-2 block text-xs font-semibold tracking-wider text-text-secondary uppercase">
                {which === 'from'
                  ? t('pathFinder.fromNode', 'From Node')
                  : t('pathFinder.toNode', 'To Node')}
              </label>
              <select
                value={which === 'from' ? fromNodeId : toNodeId}
                onChange={(e) => {
                  if (which === 'from') setFromNodeId(e.target.value);
                  else setToNodeId(e.target.value);
                  setPath(null);
                }}
                className="w-full rounded-lg border border-border-default bg-surface px-3 py-2 text-sm text-text-primary focus:border-accent/50 focus:outline-none"
              >
                <option value="">{t('pathFinder.selectNode', 'Select a node...')}</option>
                {nodes.map((node) => (
                  <option key={node.id} value={node.id}>
                    {node.name} ({node.type})
                  </option>
                ))}
              </select>
            </div>
          ))}

          {/* Find Path */}
          <button
            onClick={findPath}
            disabled={!fromNodeId || !toNodeId || fromNodeId === toNodeId || searching}
            className="w-full rounded-lg border border-accent/30 bg-accent/10 px-4 py-2.5 text-sm font-medium text-accent transition-all duration-200 hover:bg-accent/20 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {searching
              ? t('pathFinder.searching', 'Searching...')
              : t('pathFinder.findPath', 'Find Path')}
          </button>

          {/* Result */}
          {path !== null && (
            <div className="mt-4">
              {path.length === 0 ? (
                <div className="rounded-lg border border-red-700/50 bg-red-900/20 p-4 text-center">
                  <svg
                    className="mx-auto mb-2 h-8 w-8 text-red-400"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                    />
                  </svg>
                  <p className="text-sm text-red-200">
                    {t('pathFinder.noPath', 'No path found between these nodes.')}
                  </p>
                </div>
              ) : (
                <div className="rounded-lg border border-border-default bg-surface p-4">
                  <div className="mb-3 flex items-center gap-2">
                    <svg
                      className="h-4 w-4 text-green-400"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
                      />
                    </svg>
                    <h3 className="text-sm font-semibold text-text-primary">
                      {t('pathFinder.pathFound', 'Path Found')} ({path.length})
                    </h3>
                  </div>
                  <div className="space-y-2">
                    {path.map((nodeId, idx) => {
                      const node = nodeMap.get(nodeId);
                      if (!node) return null;
                      const isLast = idx === path.length - 1;
                      return (
                        <div key={nodeId}>
                          <button
                            onClick={() => handleNodeClick(nodeId)}
                            className="flex w-full items-center gap-3 rounded-lg bg-elevated p-2 text-left transition-colors hover:bg-hover"
                          >
                            <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-accent/20 text-xs font-bold text-accent">
                              {idx + 1}
                            </div>
                            <div className="min-w-0 flex-1">
                              <div className="truncate text-sm text-text-primary">{node.name}</div>
                              <div className="text-xs text-text-secondary capitalize">
                                {node.type}
                              </div>
                            </div>
                            <svg
                              className="h-4 w-4 text-text-secondary"
                              fill="none"
                              stroke="currentColor"
                              viewBox="0 0 24 24"
                            >
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={2}
                                d="M9 5l7 7-7 7"
                              />
                            </svg>
                          </button>
                          {!isLast && (
                            <div className="my-1 flex items-center justify-center">
                              <svg
                                className="h-4 w-4 text-accent"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24"
                              >
                                <path
                                  strokeLinecap="round"
                                  strokeLinejoin="round"
                                  strokeWidth={2}
                                  d="M19 14l-7 7m0 0l-7-7m7 7V3"
                                />
                              </svg>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 border-t border-border-default px-5 py-4">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm text-text-secondary transition-colors hover:text-text-primary"
          >
            {t('common:close', 'Close')}
          </button>
        </div>
      </div>
    </div>
  );
}
