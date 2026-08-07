/**
 * GN → UA data adapter.
 *
 * Bridges GitNexus' structural graph model (`{ label, properties }`) to the
 * Understand-Anything semantic model (`{ type, summary, tags, complexity }`)
 * plus derived `Layer`s (from `Community` nodes) and `TourStep`s (from
 * `Process` nodes).
 *
 * GitNexus has no LLM semantic layer, so `summary`/`tags` are synthesised from
 * available structural properties (description, keywords, language, line span).
 * Fields degrade gracefully to empty/derived values when absent.
 */
import type {
  GraphNode,
  GraphRelationship,
  NodeLabel,
  NodeProperties,
  RelationshipType,
} from 'gitnexus-shared';
import type {
  UAComplexity,
  UAEdgeType,
  UAGraphEdge,
  UAGraphNode,
  UAKnowledgeGraph,
  UALayer,
  UANodeType,
  UAProjectMeta,
  UATourStep,
} from './ua-model';

/** GN NodeLabel → UA NodeType. Unknown labels fall back to 'concept'. */
const LABEL_TO_TYPE: Record<NodeLabel, UANodeType> = {
  Project: 'module',
  Package: 'module',
  Module: 'module',
  Folder: 'module',
  Namespace: 'module',
  Community: 'module',
  File: 'file',
  Class: 'class',
  Interface: 'class',
  Enum: 'class',
  Struct: 'class',
  Union: 'class',
  Trait: 'class',
  Impl: 'class',
  Record: 'class',
  Template: 'class',
  Typedef: 'class',
  TypeAlias: 'class',
  Function: 'function',
  Method: 'function',
  Constructor: 'function',
  Delegate: 'function',
  Macro: 'function',
  Process: 'flow',
  Route: 'endpoint',
  Tool: 'service',
  Section: 'document',
  Variable: 'concept',
  Const: 'concept',
  Static: 'concept',
  Property: 'concept',
  Type: 'concept',
  Decorator: 'concept',
  Annotation: 'concept',
  Import: 'concept',
  CodeElement: 'concept',
};

const REL_TO_EDGE: Record<
  RelationshipType,
  { type: UAEdgeType; direction: UAGraphEdge['direction'] }
> = {
  CONTAINS: { type: 'contains', direction: 'forward' },
  DEFINES: { type: 'contains', direction: 'forward' },
  HAS_METHOD: { type: 'contains', direction: 'forward' },
  HAS_PROPERTY: { type: 'contains', direction: 'forward' },
  MEMBER_OF: { type: 'contains', direction: 'backward' },
  CALLS: { type: 'calls', direction: 'forward' },
  FETCHES: { type: 'calls', direction: 'forward' },
  INHERITS: { type: 'inherits', direction: 'forward' },
  EXTENDS: { type: 'inherits', direction: 'forward' },
  METHOD_OVERRIDES: { type: 'inherits', direction: 'forward' },
  IMPLEMENTS: { type: 'implements', direction: 'forward' },
  METHOD_IMPLEMENTS: { type: 'implements', direction: 'forward' },
  IMPORTS: { type: 'imports', direction: 'forward' },
  USES: { type: 'depends_on', direction: 'forward' },
  ACCESSES: { type: 'reads_from', direction: 'forward' },
  QUERIES: { type: 'reads_from', direction: 'forward' },
  DECORATES: { type: 'related', direction: 'forward' },
  STEP_IN_PROCESS: { type: 'flow_step', direction: 'forward' },
  HANDLES_ROUTE: { type: 'routes', direction: 'forward' },
  HANDLES_TOOL: { type: 'serves', direction: 'forward' },
  ENTRY_POINT_OF: { type: 'triggers', direction: 'forward' },
  WRAPS: { type: 'middleware', direction: 'forward' },
  BINDS_EVENT_HANDLER: { type: 'related', direction: 'forward' },
  EMITS_EVENT: { type: 'triggers', direction: 'forward' },
};

function mapLabelToType(label: NodeLabel): UANodeType {
  return LABEL_TO_TYPE[label] ?? 'concept';
}

/**
 * Derive a complexity bucket from structural signals. GN has no LLM complexity
 * score, so we approximate from line span, member/param/step counts.
 */
function deriveComplexity(props: NodeProperties): UAComplexity {
  const span =
    typeof props.startLine === 'number' && typeof props.endLine === 'number'
      ? Math.max(0, props.endLine - props.startLine)
      : 0;
  const counts = [props.symbolCount ?? 0, props.parameterCount ?? 0, props.stepCount ?? 0];
  const maxCount = Math.max(...counts);

  if (span > 200 || maxCount > 20) return 'complex';
  if (span > 40 || maxCount > 6) return 'moderate';
  return 'simple';
}

/** Synthesise a human-readable summary from available structural props. */
function deriveSummary(label: NodeLabel, props: NodeProperties): string {
  if (typeof props.description === 'string' && props.description.trim()) {
    return props.description.trim();
  }
  if (typeof props.heuristicLabel === 'string' && props.heuristicLabel.trim()) {
    return props.heuristicLabel.trim();
  }
  const parts: string[] = [label];
  if (props.language) parts.push(String(props.language));
  if (props.filePath) parts.push(props.filePath);
  return parts.join(' · ');
}

/** Collect tags from keywords, language, visibility and notable flags. */
function deriveTags(props: NodeProperties): string[] {
  const tags = new Set<string>();
  if (Array.isArray(props.keywords)) {
    for (const k of props.keywords) if (typeof k === 'string') tags.add(k);
  }
  if (props.language) tags.add(String(props.language));
  if (props.visibility) tags.add(String(props.visibility));
  if (props.isExported) tags.add('exported');
  if (props.isAsync) tags.add('async');
  if (props.isStatic) tags.add('static');
  if (props.isAbstract) tags.add('abstract');
  if (props.processType) tags.add(String(props.processType));
  if (props.enrichedBy) tags.add(`enriched:${props.enrichedBy}`);
  return Array.from(tags);
}

function toUANode(node: GraphNode): UAGraphNode {
  const { properties: props, label } = node;
  const lineRange: [number, number] | undefined =
    typeof props.startLine === 'number' && typeof props.endLine === 'number'
      ? [props.startLine, props.endLine]
      : undefined;

  return {
    id: node.id,
    type: mapLabelToType(label),
    name: props.name ?? node.id,
    filePath: props.filePath || undefined,
    lineRange,
    summary: deriveSummary(label, props),
    tags: deriveTags(props),
    complexity: deriveComplexity(props),
    languageNotes: props.language ? String(props.language) : undefined,
  };
}

function toUAEdge(rel: GraphRelationship): UAGraphEdge {
  const mapped = REL_TO_EDGE[rel.type] ?? {
    type: 'related' as UAEdgeType,
    direction: 'forward' as const,
  };
  return {
    source: rel.sourceId,
    target: rel.targetId,
    type: mapped.type,
    direction: mapped.direction,
    description: rel.reason || undefined,
    // GN uses 0-1 confidence already; clamp defensively.
    weight: Math.min(1, Math.max(0, rel.confidence ?? 0.5)),
  };
}

/**
 * Derive UA Layers from GN `Community` nodes.
 * Membership: any relationship linking a member to the community
 * (MEMBER_OF member→community, or CONTAINS community→member).
 */
function deriveLayers(nodes: GraphNode[], rels: GraphRelationship[]): UALayer[] {
  const communities = nodes.filter((n) => n.label === 'Community');
  if (communities.length === 0) return [];

  const communityIds = new Set(communities.map((c) => c.id));
  const members = new Map<string, Set<string>>();
  for (const c of communities) members.set(c.id, new Set<string>());

  for (const rel of rels) {
    if (rel.type === 'MEMBER_OF' && communityIds.has(rel.targetId)) {
      members.get(rel.targetId)!.add(rel.sourceId);
    } else if (rel.type === 'CONTAINS' && communityIds.has(rel.sourceId)) {
      members.get(rel.sourceId)!.add(rel.targetId);
    }
  }

  return communities.map((c) => {
    const props = c.properties;
    const name =
      (typeof props.heuristicLabel === 'string' && props.heuristicLabel.trim()) ||
      props.name ||
      c.id;
    return {
      id: c.id,
      name,
      description: deriveSummary('Community', props),
      // Include the community node itself so it renders inside its own layer.
      nodeIds: [c.id, ...Array.from(members.get(c.id) ?? [])],
    };
  });
}

/**
 * Derive UA Tour steps from GN `Process` nodes.
 * Each Process becomes one TourStep; its `nodeIds` are the ordered step
 * targets from STEP_IN_PROCESS relationships (ordered by `step`).
 */
function deriveTour(nodes: GraphNode[], rels: GraphRelationship[]): UATourStep[] {
  const processes = nodes.filter((n) => n.label === 'Process');
  if (processes.length === 0) return [];

  const stepsByProcess = new Map<string, GraphRelationship[]>();
  for (const rel of rels) {
    if (rel.type === 'STEP_IN_PROCESS') {
      const list = stepsByProcess.get(rel.sourceId) ?? [];
      list.push(rel);
      stepsByProcess.set(rel.sourceId, list);
    }
  }

  return processes.map((p, idx) => {
    const props = p.properties;
    const stepRels = (stepsByProcess.get(p.id) ?? [])
      .slice()
      .sort((a, b) => (a.step ?? 0) - (b.step ?? 0));
    const orderedNodeIds = stepRels.map((r) => r.targetId);
    const nodeIds = orderedNodeIds.length > 0 ? orderedNodeIds : [p.id];

    return {
      order: idx + 1,
      title:
        (typeof props.heuristicLabel === 'string' && props.heuristicLabel.trim()) ||
        props.name ||
        `Process ${idx + 1}`,
      description: deriveSummary('Process', props),
      nodeIds,
    };
  });
}

function deriveProjectMeta(nodes: GraphNode[]): UAProjectMeta {
  const projectNode = nodes.find((n) => n.label === 'Project');
  const languages = Array.from(
    new Set(
      nodes
        .map((n) => n.properties.language)
        .filter((l): l is string => typeof l === 'string' && l.length > 0),
    ),
  );

  return {
    name: projectNode?.properties.name ?? 'Untitled Project',
    languages,
    frameworks: [],
    description: (projectNode && deriveSummary('Project', projectNode.properties)) || '',
    analyzedAt: new Date().toISOString(),
    gitCommitHash: '',
  };
}

/**
 * Adapt a GitNexus graph (nodes + relationships) into a UA KnowledgeGraph.
 */
export function adaptGnGraphToUA(
  nodes: GraphNode[],
  relationships: GraphRelationship[],
): UAKnowledgeGraph {
  return {
    version: '1.0.0',
    kind: 'codebase',
    project: deriveProjectMeta(nodes),
    nodes: nodes.map(toUANode),
    edges: relationships.map(toUAEdge),
    layers: deriveLayers(nodes, relationships),
    tour: deriveTour(nodes, relationships),
  };
}

// Re-export the low-level mappers for unit testing / advanced callers.
export const __testables = {
  mapLabelToType,
  deriveComplexity,
  deriveSummary,
  deriveTags,
  toUANode,
  toUAEdge,
  deriveLayers,
  deriveTour,
};
