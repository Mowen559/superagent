import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { getSyntaxLanguageFromFilename } from 'gitnexus-shared';
import { useAppState } from '../hooks/useAppState';
import {
  fetchFileHistory,
  fetchFileVersion,
  fetchFileDiff,
  type FileCommit,
} from '../services/backend-client';
import FileDiffViewer from './FileDiffViewer';

/**
 * File history & comparison modal.
 *
 * Left: a commit timeline for the file (from /api/file/history).
 * Right: a Diff/Version toggle (mirrors UA's DiffToggle interaction):
 *  - Diff    → unified diff of the selected commit vs the previous one in the
 *              file's history (or vs nothing for the initial commit).
 *  - Version → the full file content at the selected commit, syntax-highlighted.
 */

interface FileHistoryModalProps {
  filePath: string;
  onClose: () => void;
}

const codeTheme = {
  ...vscDarkPlus,
  'pre[class*="language-"]': {
    ...vscDarkPlus['pre[class*="language-"]'],
    background: 'transparent',
    margin: 0,
    padding: '12px 0',
    fontSize: '12px',
    lineHeight: '1.6',
  },
  'code[class*="language-"]': {
    ...vscDarkPlus['code[class*="language-"]'],
    background: 'transparent',
    fontFamily: '"JetBrains Mono", "Fira Code", monospace',
  },
};

type ViewMode = 'diff' | 'version';

function shortHash(hash: string): string {
  return hash.slice(0, 7);
}

function formatDate(iso: string): string {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

export default function FileHistoryModal({ filePath, onClose }: FileHistoryModalProps) {
  const { projectName } = useAppState();
  const { t } = useTranslation(['common']);
  const modalRef = useRef<HTMLDivElement>(null);

  const repo = projectName || undefined;
  const fileName = useMemo(() => filePath.split('/').pop() ?? filePath, [filePath]);
  const language = useMemo(() => getSyntaxLanguageFromFilename(filePath), [filePath]);

  const [commits, setCommits] = useState<FileCommit[]>([]);
  const [loadingHistory, setLoadingHistory] = useState(true);
  const [historyError, setHistoryError] = useState<string | null>(null);

  const [selectedIndex, setSelectedIndex] = useState(0);
  const [viewMode, setViewMode] = useState<ViewMode>('diff');

  const [content, setContent] = useState('');
  const [loadingContent, setLoadingContent] = useState(false);
  const [contentError, setContentError] = useState<string | null>(null);

  // Close on Escape / outside click.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    const onClick = (e: MouseEvent) => {
      if (modalRef.current && !modalRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', onClick);
    return () => {
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onClick);
    };
  }, [onClose]);

  // Load commit history.
  useEffect(() => {
    let cancelled = false;
    setLoadingHistory(true);
    setHistoryError(null);
    fetchFileHistory(filePath, { repo, limit: 100 })
      .then((list) => {
        if (cancelled) return;
        setCommits(list);
        setSelectedIndex(0);
        setLoadingHistory(false);
      })
      .catch((err) => {
        if (cancelled) return;
        setHistoryError(err?.message ?? 'Failed to load history');
        setLoadingHistory(false);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, repo]);

  const selected = commits[selectedIndex];
  // The next-older commit (commits are most-recent first), if any.
  const previous = commits[selectedIndex + 1];

  // Load diff or version content for the selected commit.
  useEffect(() => {
    if (!selected) {
      setContent('');
      return;
    }
    let cancelled = false;
    setLoadingContent(true);
    setContentError(null);
    setContent('');

    const load =
      viewMode === 'version'
        ? fetchFileVersion(filePath, selected.hash, { repo })
        : previous
          ? fetchFileDiff(filePath, previous.hash, selected.hash, { repo })
          : // Initial commit (no parent in the followed history): show full content.
            fetchFileVersion(filePath, selected.hash, { repo });

    load
      .then((text) => {
        if (cancelled) return;
        setContent(text);
        setLoadingContent(false);
      })
      .catch((err) => {
        if (cancelled) return;
        setContentError(err?.message ?? 'Failed to load content');
        setLoadingContent(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selected, previous, viewMode, filePath, repo]);

  const isInitialCommit = !!selected && !previous;
  const showVersionContent = viewMode === 'version' || isInitialCommit;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-void/80 p-6 backdrop-blur-sm">
      <div
        ref={modalRef}
        className="flex h-[80vh] w-[min(1100px,92vw)] flex-col overflow-hidden rounded-xl border border-border-default bg-elevated shadow-2xl"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border-default px-4 py-3">
          <div className="min-w-0">
            <h2 className="truncate text-sm font-semibold text-text-primary">
              {t('fileHistory.title', 'File history')} · {fileName}
            </h2>
            <p className="truncate font-mono text-[11px] text-text-muted" title={filePath}>
              {filePath}
            </p>
          </div>
          <button
            onClick={onClose}
            className="ml-4 shrink-0 rounded px-2 py-1 text-xs text-text-muted transition-colors hover:bg-hover hover:text-text-primary"
          >
            {t('common:close', 'Close')}
          </button>
        </div>

        <div className="flex min-h-0 flex-1">
          {/* Commit timeline */}
          <div className="w-[280px] shrink-0 overflow-y-auto border-r border-border-default">
            {loadingHistory ? (
              <div className="p-4 text-sm text-text-muted">{t('common:loading', 'Loading…')}</div>
            ) : historyError ? (
              <div className="p-4 text-sm text-[#c97070]">{historyError}</div>
            ) : commits.length === 0 ? (
              <div className="p-4 text-sm text-text-muted">
                {t('fileHistory.noHistory', 'No git history for this file')}
              </div>
            ) : (
              <ul>
                {commits.map((c, i) => (
                  <li key={c.hash}>
                    <button
                      onClick={() => setSelectedIndex(i)}
                      className={`w-full border-b border-border-default/50 px-3 py-2.5 text-left transition-colors ${
                        i === selectedIndex ? 'bg-accent/10' : 'hover:bg-hover/50'
                      }`}
                    >
                      <div
                        className="truncate text-xs font-medium text-text-primary"
                        title={c.message}
                      >
                        {c.message || shortHash(c.hash)}
                      </div>
                      <div className="mt-1 flex items-center gap-2 text-[10px] text-text-muted">
                        <span className="font-mono text-accent">{shortHash(c.hash)}</span>
                        <span className="truncate">{c.author}</span>
                        <span className="ml-auto shrink-0">{formatDate(c.date)}</span>
                      </div>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>

          {/* Detail pane */}
          <div className="flex min-w-0 flex-1 flex-col">
            {/* Toggle bar */}
            <div className="flex items-center gap-2 border-b border-border-default px-3 py-2">
              <div className="flex overflow-hidden rounded-lg border border-border-default">
                <button
                  onClick={() => setViewMode('diff')}
                  disabled={isInitialCommit}
                  className={`px-3 py-1 text-xs transition-colors disabled:opacity-40 ${
                    viewMode === 'diff' && !isInitialCommit
                      ? 'bg-accent/20 text-accent'
                      : 'text-text-secondary hover:text-text-primary'
                  }`}
                >
                  {t('fileHistory.diff', 'Diff')}
                </button>
                <button
                  onClick={() => setViewMode('version')}
                  className={`px-3 py-1 text-xs transition-colors ${
                    showVersionContent
                      ? 'bg-accent/20 text-accent'
                      : 'text-text-secondary hover:text-text-primary'
                  }`}
                >
                  {t('fileHistory.version', 'Version')}
                </button>
              </div>
              {selected && (
                <span className="truncate text-[11px] text-text-muted">
                  {showVersionContent
                    ? t('fileHistory.viewingVersion', 'Viewing {{hash}}', {
                        hash: shortHash(selected.hash),
                      })
                    : t('fileHistory.comparing', '{{from}} → {{to}}', {
                        from: previous ? shortHash(previous.hash) : '∅',
                        to: shortHash(selected.hash),
                      })}
                </span>
              )}
            </div>

            {/* Content */}
            <div className="min-h-0 flex-1 overflow-auto bg-[#0a0a10]">
              {!selected ? (
                <div className="flex h-full items-center justify-center text-sm text-text-muted">
                  {t('fileHistory.selectCommit', 'Select a commit')}
                </div>
              ) : loadingContent ? (
                <div className="flex h-full items-center justify-center text-sm text-text-muted">
                  {t('common:loading', 'Loading…')}
                </div>
              ) : contentError ? (
                <div className="p-4 text-sm text-[#c97070]">{contentError}</div>
              ) : showVersionContent ? (
                <SyntaxHighlighter
                  language={language}
                  style={codeTheme as never}
                  showLineNumbers
                  lineNumberStyle={{
                    minWidth: '3em',
                    paddingRight: '1em',
                    color: '#5a5a70',
                    textAlign: 'right',
                    userSelect: 'none',
                  }}
                >
                  {content || ''}
                </SyntaxHighlighter>
              ) : (
                <FileDiffViewer
                  diff={content}
                  emptyHint={t('fileHistory.noTextChange', 'No textual changes in this file')}
                />
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
