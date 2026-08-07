import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import type { GraphNode } from 'gitnexus-shared';
import { useAppState } from '../hooks/useAppState';
import { useUAFeatures } from '../hooks/useUAFeatures';
import type { UAComplexity, UADomainGraph, UAGraphNode, UANodeType } from '../core/graph/ua-model';
import FileHistoryModal from './FileHistoryModal';

/**
 * Semantic node detail panel — ported from Understand-Anything's NodeInfo.
 *
 * Adapted to GitNexus:
 *  - Reads the selected node from `useAppState` and resolves its semantic
 *    counterpart from the UA-adapted graph (`useUAFeatures`).
 *  - Type badge colours map UA node types onto GitNexus' existing `node-*`
 *    CSS tokens (file/folder/class/function/interface/method), falling back to
 *    the accent colour for non-code semantic types.
 *  - Drops UA-only knowledge/domain metadata (GitNexus' graph has none).
 *  - Navigation selects the GitNexus node so the sigma camera stays in sync.
 */

// Map UA node types to full Tailwind badge classes. Class strings must be
// literal (not interpolated) so Tailwind's JIT scanner picks them up. Non-code
// conceptual types fall back to the accent colour.
const TYPE_BADGE: Record<UANodeType, string> = {
  file: 'text-node-file border border-node-file/30 bg-node-file/10',
  module: 'text-node-folder border border-node-folder/30 bg-node-folder/10',
  resource: 'text-node-folder border border-node-folder/30 bg-node-folder/10',
  class: 'text-node-class border border-node-class/30 bg-node-class/10',
  table: 'text-node-class border border-node-class/30 bg-node-class/10',
  function: 'text-node-function border border-node-function/30 bg-node-function/10',
  step: 'text-node-function border border-node-function/30 bg-node-function/10',
  endpoint: 'text-node-interface border border-node-interface/30 bg-node-interface/10',
  schema: 'text-node-interface border border-node-interface/30 bg-node-interface/10',
  service: 'text-node-method border border-node-method/30 bg-node-method/10',
  pipeline: 'text-node-method border border-node-method/30 bg-node-method/10',
  flow: 'text-node-method border border-node-method/30 bg-node-method/10',
  config: 'text-node-import border border-node-import/30 bg-node-import/10',
  document: 'text-node-import border border-node-import/30 bg-node-import/10',
  concept: 'text-accent border border-accent/30 bg-accent/10',
  domain: 'text-accent border border-accent/30 bg-accent/10',
  article: 'text-accent border border-accent/30 bg-accent/10',
  entity: 'text-accent border border-accent/30 bg-accent/10',
  topic: 'text-accent border border-accent/30 bg-accent/10',
  claim: 'text-accent border border-accent/30 bg-accent/10',
  source: 'text-accent border border-accent/30 bg-accent/10',
};

function typeBadgeClass(type: UANodeType): string {
  return TYPE_BADGE[type] ?? TYPE_BADGE.file;
}

const COMPLEXITY_CLASS: Record<UAComplexity, string> = {
  simple: 'text-node-function border border-node-function/30 bg-node-function/10',
  moderate: 'text-accent border border-accent/30 bg-accent/10',
  complex: 'text-[#c97070] border border-[#c97070]/30 bg-[#c97070]/10',
};

function humanizeEdge(type: string): string {
  return type.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}

/**
 * Domain node detail panel — shown when the canvas is in domain-graph mode.
 * Renders the business-domain metadata (entities / business rules / cross-domain
 * interactions / entry point) plus the node's connections inside the domain graph.
 */
function DomainNodeDetails({
  node,
  domainGraph,
  t,
}: {
  node: UAGraphNode;
  domainGraph: UADomainGraph;
  t: TFunction;
}) {
  const meta = node.domainMeta;
  const nodeById = useMemo(
    () => new Map(domainGraph.nodes.map((n) => [n.id, n] as const)),
    [domainGraph],
  );
  const connections = useMemo(
    () => domainGraph.edges.filter((e) => e.source === node.id || e.target === node.id),
    [domainGraph, node.id],
  );

  return (
    <div className="h-full w-full animate-fade-in overflow-auto p-5">
      <div className="mb-3 flex items-center gap-2">
        <span
          className={`rounded px-2 py-0.5 text-[10px] font-semibold tracking-wider uppercase ${typeBadgeClass(node.type)}`}
        >
          {node.type}
        </span>
      </div>

      <h2 className="mb-2 text-lg font-semibold text-text-primary">{node.name}</h2>

      {node.summary && (
        <p className="mb-4 text-sm leading-relaxed text-text-secondary">{node.summary}</p>
      )}

      {/* Entry point (flow nodes) */}
      {meta?.entryPoint && (
        <div className="mb-4 rounded-lg border border-border-default bg-elevated/60 p-3 text-xs text-text-secondary">
          <div className="mb-1 font-medium text-text-muted">
            {t('domainGraph.entryPoint', 'Entry point')}
          </div>
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-text-primary" title={meta.entryPoint}>
              {meta.entryPoint}
            </span>
            {meta.entryType && (
              <span className="rounded border border-accent/30 bg-accent/10 px-1.5 py-0.5 text-[10px] font-semibold tracking-wider text-accent uppercase">
                {meta.entryType}
              </span>
            )}
          </div>
        </div>
      )}

      {/* Entities */}
      {meta?.entities && meta.entities.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('domainGraph.entities', 'Entities')}
          </h3>
          <div className="flex flex-wrap gap-1.5">
            {meta.entities.map((e) => (
              <span
                key={e}
                className="rounded-full border border-border-default bg-elevated px-2.5 py-1 text-[11px] text-text-secondary"
              >
                {e}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Business rules */}
      {meta?.businessRules && meta.businessRules.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('domainGraph.businessRules', 'Business rules')}
          </h3>
          <ul className="space-y-1">
            {meta.businessRules.map((r, i) => (
              <li
                key={i}
                className="rounded-lg border border-border-default bg-elevated px-3 py-2 text-[11px] leading-relaxed text-text-secondary"
              >
                {r}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* Cross-domain interactions */}
      {meta?.crossDomainInteractions && meta.crossDomainInteractions.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('domainGraph.crossDomainInteractions', 'Cross-domain interactions')}
          </h3>
          <ul className="space-y-1">
            {meta.crossDomainInteractions.map((c, i) => (
              <li
                key={i}
                className="rounded-lg border border-border-default bg-elevated px-3 py-2 text-[11px] leading-relaxed text-text-secondary"
              >
                {c}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* Connections within the domain graph */}
      {connections.length > 0 && (
        <div>
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('nodeInfo.connections', 'Connections')} ({connections.length})
          </h3>
          <div className="space-y-1.5">
            {connections.map((edge, i) => {
              const isSource = edge.source === node.id;
              const otherId = isSource ? edge.target : edge.source;
              const otherNode = nodeById.get(otherId);
              const arrow = isSource ? '\u2192' : '\u2190';
              return (
                <div
                  key={i}
                  className="flex items-center gap-2 rounded-lg border border-border-default bg-elevated px-3 py-2 text-xs"
                >
                  <span className="font-mono text-accent">{arrow}</span>
                  <span className="text-text-muted">{humanizeEdge(edge.type)}</span>
                  <span className="truncate text-text-primary">{otherNode?.name ?? otherId}</span>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

export default function NodeInfo() {
  const { graph, selectedNode, setSelectedNode, openCodePanel } = useAppState();
  const { uaGraph, activeGraphKind, selectedDomainNode, domainGraph } = useUAFeatures();
  const { t } = useTranslation(['common']);
  const [languageExpanded, setLanguageExpanded] = useState(true);
  const [historyOpen, setHistoryOpen] = useState(false);

  // Resolve the GN node id → GN node map for navigation.
  const gnNodeById = useMemo(() => {
    const map = new Map<string, GraphNode>();
    if (graph) graph.nodes.forEach((n) => map.set(n.id, n));
    return map;
  }, [graph]);

  const uaNodeById = useMemo(() => {
    const map = new Map<string, UAGraphNode>();
    uaGraph?.nodes.forEach((n) => map.set(n.id, n));
    return map;
  }, [uaGraph]);

  // Domain view: render domain node details from the LLM-generated domain graph.
  if (activeGraphKind === 'domain') {
    if (!selectedDomainNode || !domainGraph) {
      return (
        <div className="flex h-full w-full items-center justify-center bg-surface">
          <p className="text-sm text-text-muted">
            {t('nodeInfo.selectNode', 'Select a node to inspect')}
          </p>
        </div>
      );
    }
    return <DomainNodeDetails node={selectedDomainNode} domainGraph={domainGraph} t={t} />;
  }

  const node = selectedNode && uaGraph ? (uaNodeById.get(selectedNode.id) ?? null) : null;

  if (!node || !uaGraph) {
    return (
      <div className="flex h-full w-full items-center justify-center bg-surface">
        <p className="text-sm text-text-muted">
          {t('nodeInfo.selectNode', 'Select a node to inspect')}
        </p>
      </div>
    );
  }

  const navigateToNode = (id: string) => {
    const gn = gnNodeById.get(id);
    if (gn) {
      setSelectedNode(gn);
      openCodePanel();
    }
  };

  const connections = uaGraph.edges.filter((e) => e.source === node.id || e.target === node.id);
  const childEdges = connections.filter((e) => e.type === 'contains' && e.source === node.id);
  const otherConnections = connections.filter(
    (e) => !(e.type === 'contains' && e.source === node.id),
  );

  const childNodes = childEdges
    .map((e) => uaNodeById.get(e.target))
    .filter((n): n is UAGraphNode => n !== undefined);

  // Flow steps (derived from GN Process nodes).
  const flowSteps =
    node.type === 'flow'
      ? uaGraph.edges
          .filter((e) => e.type === 'flow_step' && e.source === node.id)
          .sort((a, b) => a.weight - b.weight)
          .map((e) => uaNodeById.get(e.target))
          .filter((n): n is UAGraphNode => n !== undefined)
      : [];

  return (
    <div className="h-full w-full animate-fade-in overflow-auto p-5">
      {/* Badges */}
      <div className="mb-3 flex items-center gap-2">
        <span
          className={`rounded px-2 py-0.5 text-[10px] font-semibold tracking-wider uppercase ${typeBadgeClass(node.type)}`}
        >
          {node.type}
        </span>
        <span
          className={`rounded px-2 py-0.5 text-[10px] font-semibold ${COMPLEXITY_CLASS[node.complexity]}`}
        >
          {node.complexity}
        </span>
      </div>

      <h2 className="mb-2 text-lg font-semibold text-text-primary">{node.name}</h2>

      {node.summary && (
        <p className="mb-4 text-sm leading-relaxed text-text-secondary">{node.summary}</p>
      )}

      {/* File path + open code */}
      {node.filePath && (
        <div className="mb-4 rounded-lg border border-border-default bg-elevated/60 p-3 text-xs text-text-secondary">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="mb-1 font-medium text-text-muted">{t('nodeInfo.file', 'File')}</div>
              <div className="truncate font-mono" title={node.filePath}>
                {node.filePath}
                {node.lineRange && (
                  <span className="ml-2 text-text-muted">
                    L{node.lineRange[0]}-{node.lineRange[1]}
                  </span>
                )}
              </div>
            </div>
            <div className="flex shrink-0 flex-col gap-1">
              <button
                type="button"
                onClick={() => navigateToNode(node.id)}
                className="rounded border border-accent/30 px-2.5 py-1 text-[10px] font-semibold tracking-wider text-accent uppercase transition-colors hover:border-accent/60"
              >
                {t('nodeInfo.openCode', 'Open code')}
              </button>
              <button
                type="button"
                onClick={() => setHistoryOpen(true)}
                className="rounded border border-border-default px-2.5 py-1 text-[10px] font-semibold tracking-wider text-text-secondary uppercase transition-colors hover:border-text-muted hover:text-text-primary"
              >
                {t('nodeInfo.viewHistory', 'History')}
              </button>
            </div>
          </div>
        </div>
      )}

      {historyOpen && node.filePath && (
        <FileHistoryModal filePath={node.filePath} onClose={() => setHistoryOpen(false)} />
      )}

      {/* Language notes */}
      {node.languageNotes && (
        <div className="mb-4">
          <button
            onClick={() => setLanguageExpanded((v) => !v)}
            className="mb-2 flex items-center gap-1.5 text-xs font-semibold tracking-wider text-accent uppercase transition-opacity hover:opacity-80"
          >
            <svg
              className={`h-3 w-3 transition-transform ${languageExpanded ? 'rotate-90' : ''}`}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
            {t('nodeInfo.languageConcepts', 'Language concepts')}
          </button>
          {languageExpanded && (
            <div className="rounded-lg border border-accent/20 bg-accent/5 p-3">
              <p className="text-sm leading-relaxed text-text-secondary">{node.languageNotes}</p>
            </div>
          )}
        </div>
      )}

      {/* Tags */}
      {node.tags.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('nodeInfo.tags', 'Tags')}
          </h3>
          <div className="flex flex-wrap gap-1.5">
            {node.tags.map((tag) => (
              <span
                key={tag}
                className="rounded-full border border-border-default bg-elevated px-2.5 py-1 text-[11px] text-text-secondary"
              >
                {tag}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Flow steps */}
      {flowSteps.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('nodeInfo.steps', 'Steps')} ({flowSteps.length})
          </h3>
          <ol className="space-y-1">
            {flowSteps.map((s, i) => (
              <li key={s.id}>
                <button
                  type="button"
                  onClick={() => navigateToNode(s.id)}
                  className="block w-full rounded bg-elevated px-2 py-1.5 text-left text-[11px] transition-colors hover:bg-hover"
                >
                  <span className="mr-1.5 text-accent/60">{i + 1}.</span>
                  <span className="text-text-secondary">{s.name}</span>
                </button>
              </li>
            ))}
          </ol>
        </div>
      )}

      {/* Children (contains) */}
      {childNodes.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('nodeInfo.definedInThisFile', 'Defined in this file')} ({childNodes.length})
          </h3>
          <div className="space-y-1">
            {childNodes.map((child) => (
              <div
                key={child.id}
                className="cursor-pointer rounded-lg border border-border-default bg-elevated px-3 py-2 text-xs transition-colors hover:border-accent/40 hover:bg-accent/5"
                onClick={() => navigateToNode(child.id)}
              >
                <div className="flex items-center gap-2">
                  <span
                    className={`rounded px-1.5 py-0.5 text-[9px] font-semibold tracking-wider uppercase ${typeBadgeClass(child.type)}`}
                  >
                    {child.type}
                  </span>
                  <span className="truncate text-text-primary">{child.name}</span>
                  <span
                    className={`ml-auto rounded px-1 py-0.5 text-[9px] ${COMPLEXITY_CLASS[child.complexity]}`}
                  >
                    {child.complexity}
                  </span>
                </div>
                {child.summary && (
                  <p className="mt-1 line-clamp-1 pl-1 text-[11px] text-text-muted">
                    {child.summary}
                  </p>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Other connections */}
      {otherConnections.length > 0 && (
        <div>
          <h3 className="mb-2 text-[11px] font-semibold tracking-wider text-accent uppercase">
            {t('nodeInfo.connections', 'Connections')} ({otherConnections.length})
          </h3>
          <div className="space-y-1.5">
            {otherConnections.map((edge, i) => {
              const isSource = edge.source === node.id;
              const otherId = isSource ? edge.target : edge.source;
              const otherNode = uaNodeById.get(otherId);
              const arrow = isSource ? '\u2192' : '\u2190';
              return (
                <div
                  key={i}
                  className="flex cursor-pointer items-center gap-2 rounded-lg border border-border-default bg-elevated px-3 py-2 text-xs transition-colors hover:border-accent/40 hover:bg-accent/5"
                  onClick={() => navigateToNode(otherId)}
                >
                  <span className="font-mono text-accent">{arrow}</span>
                  <span className="text-text-muted">{humanizeEdge(edge.type)}</span>
                  <span className="truncate text-text-primary">{otherNode?.name ?? otherId}</span>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
