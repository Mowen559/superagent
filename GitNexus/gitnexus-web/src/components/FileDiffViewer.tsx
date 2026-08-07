import { useMemo } from 'react';

/**
 * Renders a unified git diff with add/remove/hunk colouring.
 *
 * Lightweight inline-diff viewer (no external diff library): the backend
 * returns `git diff` text which we split into lines and classify. Kept
 * dependency-free to match GitNexus' existing rendering approach.
 */

interface FileDiffViewerProps {
  diff: string;
  /** Shown when the diff is empty (e.g. no textual change between refs). */
  emptyHint?: string;
}

type DiffLineKind = 'add' | 'del' | 'hunk' | 'meta' | 'context';

interface DiffLine {
  kind: DiffLineKind;
  text: string;
}

function classify(line: string): DiffLineKind {
  if (line.startsWith('@@')) return 'hunk';
  if (
    line.startsWith('diff ') ||
    line.startsWith('index ') ||
    line.startsWith('--- ') ||
    line.startsWith('+++ ') ||
    line.startsWith('new file') ||
    line.startsWith('deleted file') ||
    line.startsWith('similarity ') ||
    line.startsWith('rename ')
  ) {
    return 'meta';
  }
  if (line.startsWith('+')) return 'add';
  if (line.startsWith('-')) return 'del';
  return 'context';
}

const KIND_CLASS: Record<DiffLineKind, string> = {
  add: 'bg-node-function/10 text-node-function',
  del: 'bg-[#c97070]/10 text-[#c97070]',
  hunk: 'bg-accent/10 text-accent',
  meta: 'text-text-muted',
  context: 'text-text-secondary',
};

export default function FileDiffViewer({ diff, emptyHint }: FileDiffViewerProps) {
  const lines = useMemo<DiffLine[]>(() => {
    if (!diff) return [];
    return diff.split('\n').map((text) => ({ kind: classify(text), text }));
  }, [diff]);

  if (lines.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-sm text-text-muted">
        {emptyHint ?? 'No changes'}
      </div>
    );
  }

  return (
    <pre className="m-0 overflow-auto p-0 font-mono text-[12px] leading-relaxed">
      <code className="block">
        {lines.map((line, i) => (
          <span key={i} className={`block px-3 whitespace-pre ${KIND_CLASS[line.kind]}`}>
            {line.text === '' ? ' ' : line.text}
          </span>
        ))}
      </code>
    </pre>
  );
}
