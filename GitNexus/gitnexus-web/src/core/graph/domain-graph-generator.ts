/**
 * Domain graph generator.
 *
 * Reuses GitNexus-web's built-in LangChain LLM stack (the same providers the
 * chat agent uses) to derive a *business domain graph* from the already-built
 * structural graph. No Claude/UA pipeline is involved — generation runs fully
 * in the browser using whatever provider the user configured in Settings.
 *
 * Pipeline:
 *   UA semantic graph (modules/flows/endpoints + summaries)
 *     → compact textual context (token-bounded)
 *     → LLM (createChatModel(getActiveProviderConfig()))
 *     → strict JSON
 *     → validateDomainGraph (zod + referential integrity)
 *     → UADomainGraph
 */
import { SystemMessage, HumanMessage } from '@langchain/core/messages';
import { createChatModel } from '../llm/agent';
import { getActiveProviderConfig } from '../llm/settings-service';
import { validateDomainGraph } from './domain-graph-schema';
import type { UADomainGraph, UAGraphNode, UAKnowledgeGraph } from './ua-model';

/** Thrown when no LLM provider is configured. */
export class NoProviderError extends Error {
  constructor() {
    super('No LLM provider configured');
    this.name = 'NoProviderError';
  }
}

/** Thrown when the model output cannot be parsed/validated into a domain graph. */
export class DomainGraphParseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'DomainGraphParseError';
  }
}

const SYSTEM_PROMPT = `You are a business domain analysis expert. You identify the business domains, flows, and process steps within a codebase and produce a structured domain graph.

## Three-Level Hierarchy
1. Business Domain — high-level business areas (e.g. "Order Management", "User Authentication").
2. Business Flow — specific processes within a domain (e.g. "Create Order", "Process Refund").
3. Business Step — individual actions within a flow (e.g. "Validate input", "Check inventory").

## Output — return ONLY a single JSON object, no prose, no markdown fences:
{
  "version": "1.0.0",
  "project": { "name": "<name>", "languages": [], "frameworks": [], "description": "<business purpose>", "analyzedAt": "<ISO>", "gitCommitHash": "" },
  "nodes": [
    { "id": "domain:<kebab>", "type": "domain", "name": "<Name>", "summary": "<2-3 sentences>", "tags": ["..."], "complexity": "simple|moderate|complex", "domainMeta": { "entities": ["..."], "businessRules": ["..."], "crossDomainInteractions": ["..."] } },
    { "id": "flow:<kebab>", "type": "flow", "name": "<Name>", "summary": "<what it accomplishes>", "tags": ["..."], "complexity": "simple|moderate|complex", "domainMeta": { "entryPoint": "<e.g. POST /api/orders>", "entryType": "http|cli|event|cron|manual" } },
    { "id": "step:<flow>:<step>", "type": "step", "name": "<Name>", "summary": "<what it does>", "tags": ["..."], "complexity": "simple|moderate|complex", "filePath": "<relative path or omit>" }
  ],
  "edges": [
    { "source": "domain:<n>", "target": "flow:<n>", "type": "contains_flow", "direction": "forward", "weight": 1.0 },
    { "source": "flow:<n>", "target": "step:<f>:<s>", "type": "flow_step", "direction": "forward", "weight": 0.1 },
    { "source": "domain:<a>", "target": "domain:<b>", "type": "cross_domain", "direction": "forward", "description": "<interaction>", "weight": 0.6 }
  ],
  "layers": [],
  "tour": []
}

## Rules
- Node IDs use kebab-case after the prefix (domain:order-management).
- Every flow connects to a domain via contains_flow; every step connects to a flow via flow_step.
- flow_step weights are monotonically increasing within a flow, all in [0,1].
- All weights in [0,1]. Every node has a non-empty summary and >=1 tag. complexity is simple|moderate|complex.
- No duplicate IDs, no self-edges. Only document domains/flows that actually exist in the provided context.
- Scale: aim for 2-6 domains, 2-5 flows per domain, 3-8 steps per flow. Fewer is fine for small projects.
- layers and tour MUST be empty arrays.`;

interface BuildContextOptions {
  /** Max nodes to include per category to bound token usage. */
  maxPerCategory?: number;
}

/**
 * Build a compact textual context from the UA semantic graph, prioritising the
 * high-level nodes that carry business meaning (modules → domains, flows,
 * endpoints) and their summaries/tags.
 */
export function buildDomainContext(
  graph: UAKnowledgeGraph,
  options: BuildContextOptions = {},
): string {
  const maxPerCategory = options.maxPerCategory ?? 60;

  const pick = (types: UAGraphNode['type'][]): UAGraphNode[] =>
    graph.nodes.filter((n) => types.includes(n.type)).slice(0, maxPerCategory);

  const fmt = (n: UAGraphNode): string => {
    const loc = n.filePath ? ` (${n.filePath})` : '';
    const tags = n.tags.length ? ` [${n.tags.slice(0, 6).join(', ')}]` : '';
    const summary = n.summary ? ` — ${n.summary}` : '';
    return `- ${n.name}${loc}${tags}${summary}`;
  };

  const modules = pick(['module']);
  const flows = pick(['flow']);
  const endpoints = pick(['endpoint']);
  const services = pick(['service']);
  const classes = pick(['class', 'table']);

  const sections: string[] = [];
  sections.push(`Project: ${graph.project.name}`);
  if (graph.project.languages.length) {
    sections.push(`Languages: ${graph.project.languages.join(', ')}`);
  }
  if (graph.project.description) {
    sections.push(`Description: ${graph.project.description}`);
  }
  if (modules.length) sections.push(`\n## Modules / Communities\n${modules.map(fmt).join('\n')}`);
  if (flows.length) sections.push(`\n## Processes / Flows\n${flows.map(fmt).join('\n')}`);
  if (endpoints.length) sections.push(`\n## Endpoints / Routes\n${endpoints.map(fmt).join('\n')}`);
  if (services.length) sections.push(`\n## Services / Tools\n${services.map(fmt).join('\n')}`);
  if (classes.length) sections.push(`\n## Key Types / Tables\n${classes.map(fmt).join('\n')}`);

  // Layer groupings give the model domain hints.
  if (graph.layers.length) {
    const layerLines = graph.layers
      .slice(0, maxPerCategory)
      .map((l) => `- ${l.name}: ${l.description || `${l.nodeIds.length} members`}`);
    sections.push(`\n## Layer groupings\n${layerLines.join('\n')}`);
  }

  return sections.join('\n');
}

/** Normalise a LangChain message content (string | parts[]) to a flat string. */
function contentToString(content: unknown): string {
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (typeof part === 'string') return part;
        if (part && typeof part === 'object' && 'text' in part) {
          return String((part as { text: unknown }).text ?? '');
        }
        return '';
      })
      .join('');
  }
  return '';
}

/** Extract the first balanced top-level JSON object from raw model text. */
function extractJsonObject(text: string): string | null {
  // Strip code fences if present.
  const fenceMatch = text.match(/```(?:json)?\s*([\s\S]*?)```/i);
  const body = fenceMatch ? fenceMatch[1] : text;

  const start = body.indexOf('{');
  if (start === -1) return null;

  let depth = 0;
  let inString = false;
  let escape = false;
  for (let i = start; i < body.length; i++) {
    const ch = body[i];
    if (escape) {
      escape = false;
      continue;
    }
    if (ch === '\\') {
      escape = true;
      continue;
    }
    if (ch === '"') {
      inString = !inString;
      continue;
    }
    if (inString) continue;
    if (ch === '{') depth++;
    else if (ch === '}') {
      depth--;
      if (depth === 0) return body.slice(start, i + 1);
    }
  }
  return null;
}

/**
 * Generate a domain graph from the UA semantic graph using the configured LLM.
 *
 * @throws {NoProviderError} when no provider is configured.
 * @throws {DomainGraphParseError} when output cannot be parsed/validated.
 */
export async function generateDomainGraph(
  graph: UAKnowledgeGraph,
  signal?: AbortSignal,
): Promise<UADomainGraph> {
  const config = getActiveProviderConfig();
  if (!config) throw new NoProviderError();

  const model = createChatModel(config);
  const context = buildDomainContext(graph);

  const userPrompt = `Analyze the following codebase context and produce the domain graph JSON described in your instructions.\n\n${context}`;

  const invoke = async (extra?: string): Promise<string> => {
    const messages = [
      new SystemMessage(SYSTEM_PROMPT),
      new HumanMessage(extra ? `${userPrompt}\n\n${extra}` : userPrompt),
    ];
    const response = await model.invoke(messages, signal ? { signal } : undefined);
    return contentToString(response.content);
  };

  let raw = await invoke();
  let json = extractJsonObject(raw);
  let result = json
    ? validateDomainGraph(safeParse(json))
    : { ok: false, errors: ['No JSON object found in model output'] };

  // One repair retry on failure.
  if (!result.ok) {
    const reason = result.errors?.join('; ') ?? 'unknown error';
    raw = await invoke(
      `Your previous output was invalid (${reason}). Return ONLY the corrected JSON object, no prose, no markdown fences.`,
    );
    json = extractJsonObject(raw);
    result = json
      ? validateDomainGraph(safeParse(json))
      : { ok: false, errors: ['No JSON object found in model output'] };
  }

  if (!result.ok || !result.data) {
    throw new DomainGraphParseError(result.errors?.join('; ') ?? 'Failed to parse domain graph');
  }

  return result.data;
}

/** JSON.parse that returns `undefined` instead of throwing. */
function safeParse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}
