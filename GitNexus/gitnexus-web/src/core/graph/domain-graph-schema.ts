/**
 * Domain graph validation (zod).
 *
 * A domain graph is a `UAKnowledgeGraph` whose nodes are limited to
 * domain/flow/step and whose edges are limited to the `domain` category
 * (contains_flow / flow_step / cross_domain). This module validates JSON
 * produced by the LLM in `domain-graph-generator.ts` (and any cached copy
 * loaded from the backend) before it reaches the renderer.
 *
 * Mirrors the core of UA's `validateGraph` (packages/core/src/schema.ts) but
 * scoped to domain graphs.
 */
import { z } from 'zod';
import type { UADomainGraph } from './ua-model';

const complexitySchema = z.enum(['simple', 'moderate', 'complex']);
const entryTypeSchema = z.enum(['http', 'cli', 'event', 'cron', 'manual']);
const domainNodeTypeSchema = z.enum(['domain', 'flow', 'step']);
const domainEdgeTypeSchema = z.enum(['contains_flow', 'flow_step', 'cross_domain']);
const directionSchema = z.enum(['forward', 'backward', 'bidirectional']);

const domainMetaSchema = z
  .object({
    entities: z.array(z.string()).optional(),
    businessRules: z.array(z.string()).optional(),
    crossDomainInteractions: z.array(z.string()).optional(),
    entryPoint: z.string().optional(),
    entryType: entryTypeSchema.optional(),
  })
  .strip();

const lineRangeSchema = z.tuple([z.number(), z.number()]);

const nodeSchema = z
  .object({
    id: z.string().min(1),
    type: domainNodeTypeSchema,
    name: z.string().min(1),
    filePath: z.string().optional(),
    lineRange: lineRangeSchema.optional(),
    summary: z.string().default(''),
    tags: z.array(z.string()).default([]),
    complexity: complexitySchema.default('moderate'),
    languageNotes: z.string().optional(),
    domainMeta: domainMetaSchema.optional(),
  })
  .strip();

const edgeSchema = z
  .object({
    source: z.string().min(1),
    target: z.string().min(1),
    type: domainEdgeTypeSchema,
    direction: directionSchema.default('forward'),
    description: z.string().optional(),
    weight: z.number().min(0).max(1).default(1),
  })
  .strip();

const projectSchema = z
  .object({
    name: z.string().default('Untitled Project'),
    languages: z.array(z.string()).default([]),
    frameworks: z.array(z.string()).default([]),
    description: z.string().default(''),
    analyzedAt: z.string().default(() => new Date().toISOString()),
    gitCommitHash: z.string().default(''),
  })
  .strip();

export const domainGraphSchema = z
  .object({
    version: z.string().default('1.0.0'),
    kind: z.enum(['codebase', 'knowledge']).default('codebase'),
    project: projectSchema.default(() => projectSchema.parse({})),
    nodes: z.array(nodeSchema).default([]),
    edges: z.array(edgeSchema).default([]),
    layers: z.array(z.unknown()).default([]),
    tour: z.array(z.unknown()).default([]),
  })
  .strip();

export interface ValidateDomainGraphResult {
  ok: boolean;
  data?: UADomainGraph;
  errors?: string[];
}

/**
 * Validate + normalise an arbitrary value into a `UADomainGraph`.
 *
 * Beyond zod's structural checks this also enforces referential integrity:
 * every edge endpoint must reference an existing node, and self-edges are
 * dropped. Returns `{ ok: false, errors }` on failure.
 */
export function validateDomainGraph(input: unknown): ValidateDomainGraphResult {
  const parsed = domainGraphSchema.safeParse(input);
  if (!parsed.success) {
    const errors = parsed.error.issues.map(
      (issue) => `${issue.path.join('.') || '(root)'}: ${issue.message}`,
    );
    return { ok: false, errors };
  }

  const graph = parsed.data;

  // Deduplicate node IDs (keep first occurrence).
  const seen = new Set<string>();
  graph.nodes = graph.nodes.filter((n) => {
    if (seen.has(n.id)) return false;
    seen.add(n.id);
    return true;
  });

  // Drop edges with missing endpoints or self-references.
  const nodeIds = new Set(graph.nodes.map((n) => n.id));
  const droppedEdges: string[] = [];
  graph.edges = graph.edges.filter((e) => {
    if (e.source === e.target) {
      droppedEdges.push(`${e.source}->${e.target} (self-edge)`);
      return false;
    }
    if (!nodeIds.has(e.source) || !nodeIds.has(e.target)) {
      droppedEdges.push(`${e.source}->${e.target} (missing endpoint)`);
      return false;
    }
    return true;
  });

  if (graph.nodes.length === 0) {
    return { ok: false, errors: ['Domain graph has no nodes'] };
  }

  // `layers` / `tour` are intentionally empty for domain graphs.
  return {
    ok: true,
    data: { ...graph, layers: [], tour: [] } as UADomainGraph,
  };
}
