/**
 * Understand-Anything (UA) semantic graph model — ported into GitNexus Web.
 *
 * GitNexus' native graph model is structural: `{ label, properties }`.
 * UA's model is semantic: `{ type, summary, tags, complexity }` plus
 * higher-level groupings (Layer) and guided walkthroughs (TourStep).
 *
 * These types are the target shape produced by `ua-adapter.ts`, consumed by
 * the ported UA visualization components (LayerView, TourPanel, FilterPanel,
 * NodeInfo, ExportMenu, PathFinder).
 *
 * Mirrors `understand-anything-plugin/packages/core/src/types.ts`.
 */

// 21 node types: 5 code + 8 non-code + 3 domain + 5 knowledge
export type UANodeType =
  | 'file'
  | 'function'
  | 'class'
  | 'module'
  | 'concept'
  | 'config'
  | 'document'
  | 'service'
  | 'table'
  | 'endpoint'
  | 'pipeline'
  | 'schema'
  | 'resource'
  | 'domain'
  | 'flow'
  | 'step'
  | 'article'
  | 'entity'
  | 'topic'
  | 'claim'
  | 'source';

// 35 edge types in 8 categories
export type UAEdgeType =
  | 'imports'
  | 'exports'
  | 'contains'
  | 'inherits'
  | 'implements'
  | 'calls'
  | 'subscribes'
  | 'publishes'
  | 'middleware'
  | 'reads_from'
  | 'writes_to'
  | 'transforms'
  | 'validates'
  | 'depends_on'
  | 'tested_by'
  | 'configures'
  | 'related'
  | 'similar_to'
  | 'deploys'
  | 'serves'
  | 'provisions'
  | 'triggers'
  | 'migrates'
  | 'documents'
  | 'routes'
  | 'defines_schema'
  | 'contains_flow'
  | 'flow_step'
  | 'cross_domain'
  | 'cites'
  | 'contradicts'
  | 'builds_on'
  | 'exemplifies'
  | 'categorized_under'
  | 'authored_by';

export type UAComplexity = 'simple' | 'moderate' | 'complex';

export type UAEdgeCategory =
  | 'structural'
  | 'behavioral'
  | 'data-flow'
  | 'dependencies'
  | 'semantic'
  | 'infrastructure'
  | 'domain'
  | 'knowledge';

/** Entry trigger type for a business flow. */
export type UAEntryType = 'http' | 'cli' | 'event' | 'cron' | 'manual';

/**
 * Business-domain metadata attached to domain/flow nodes in a domain graph.
 * Mirrors UA's `DomainMeta` (understand-anything-plugin/packages/core).
 */
export interface UADomainMeta {
  /** Key domain objects, e.g. Order, Product (domain nodes). */
  entities?: string[];
  /** Important constraints/invariants (domain nodes). */
  businessRules?: string[];
  /** How this domain interacts with other domains (domain nodes). */
  crossDomainInteractions?: string[];
  /** Trigger that starts a flow, e.g. `POST /api/orders` (flow nodes). */
  entryPoint?: string;
  /** Trigger type for the entry point (flow nodes). */
  entryType?: UAEntryType;
}

export interface UAGraphNode {
  id: string;
  type: UANodeType;
  name: string;
  filePath?: string;
  lineRange?: [number, number];
  summary: string;
  tags: string[];
  complexity: UAComplexity;
  languageNotes?: string;
  /** Present on domain/flow nodes in a domain graph. */
  domainMeta?: UADomainMeta;
}

export interface UAGraphEdge {
  source: string;
  target: string;
  type: UAEdgeType;
  direction: 'forward' | 'backward' | 'bidirectional';
  description?: string;
  weight: number; // 0-1
}

/** Logical grouping of nodes — derived from GN `Community` nodes. */
export interface UALayer {
  id: string;
  name: string;
  description: string;
  nodeIds: string[];
}

/** Guided walkthrough step — derived from GN `Process` nodes. */
export interface UATourStep {
  order: number;
  title: string;
  description: string;
  nodeIds: string[];
  languageLesson?: string;
}

export interface UAProjectMeta {
  name: string;
  languages: string[];
  frameworks: string[];
  description: string;
  analyzedAt: string;
  gitCommitHash: string;
}

export interface UAKnowledgeGraph {
  version: string;
  kind: 'codebase' | 'knowledge';
  project: UAProjectMeta;
  nodes: UAGraphNode[];
  edges: UAGraphEdge[];
  layers: UALayer[];
  tour: UATourStep[];
}

/**
 * A domain graph is structurally a `UAKnowledgeGraph` whose nodes are limited
 * to domain/flow/step and whose edges are limited to the `domain` category
 * (contains_flow / flow_step / cross_domain). `layers` and `tour` are empty.
 */
export type UADomainGraph = UAKnowledgeGraph;

/** Node types valid inside a domain graph. */
export const UA_DOMAIN_NODE_TYPES: UANodeType[] = ['domain', 'flow', 'step'];

/** Edge types valid inside a domain graph. */
export const UA_DOMAIN_EDGE_TYPES: UAEdgeType[] = ['contains_flow', 'flow_step', 'cross_domain'];

/** Valid entry types for a flow node. */
export const UA_ENTRY_TYPES: UAEntryType[] = ['http', 'cli', 'event', 'cron', 'manual'];

/** Map an edge type to its filter category. */
export const UA_EDGE_CATEGORY_MAP: Record<UAEdgeCategory, UAEdgeType[]> = {
  structural: ['imports', 'exports', 'contains', 'inherits', 'implements'],
  behavioral: ['calls', 'subscribes', 'publishes', 'middleware'],
  'data-flow': ['reads_from', 'writes_to', 'transforms', 'validates'],
  dependencies: ['depends_on', 'tested_by', 'configures'],
  semantic: ['related', 'similar_to'],
  infrastructure: [
    'deploys',
    'serves',
    'provisions',
    'triggers',
    'migrates',
    'documents',
    'routes',
    'defines_schema',
  ],
  domain: ['contains_flow', 'flow_step', 'cross_domain'],
  knowledge: [
    'cites',
    'contradicts',
    'builds_on',
    'exemplifies',
    'categorized_under',
    'authored_by',
  ],
};

export const UA_ALL_NODE_TYPES: UANodeType[] = [
  'file',
  'function',
  'class',
  'module',
  'concept',
  'config',
  'document',
  'service',
  'table',
  'endpoint',
  'pipeline',
  'schema',
  'resource',
  'domain',
  'flow',
  'step',
  'article',
  'entity',
  'topic',
  'claim',
  'source',
];

export const UA_ALL_COMPLEXITIES: UAComplexity[] = ['simple', 'moderate', 'complex'];

export const UA_ALL_EDGE_CATEGORIES: UAEdgeCategory[] = [
  'structural',
  'behavioral',
  'data-flow',
  'dependencies',
  'semantic',
  'infrastructure',
  'domain',
  'knowledge',
];
