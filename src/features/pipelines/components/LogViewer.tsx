import { CaseSensitive, Clock, Copy, Download, Regex, Search } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { toast } from 'sonner';
import '@xterm/xterm/css/xterm.css';
import { cn } from '../../../lib/cn';
import type { LogChunk, PipelineJob } from '../../../types/pipeline';
import { useLogStore } from '../stores/logStore';
import { rawLogUrl } from '../api/pipelinesApi';

interface LogViewerProps {
  workspaceId: string;
  pipelineId: string;
  job: PipelineJob;
  /** Ask the stream layer to backfill + follow this job's logs. */
  onWatch: (jobId: string) => void;
}

const EMPTY_CHUNKS: LogChunk[] = [];

/** ANSI dressing per stream; job output keeps its own colors (xterm renders
 * ANSI natively), stderr is tinted red, system lines are dimmed. When
 * timestamps are on, each chunk's first line gets a dim HH:mm:ss prefix
 * (timestamps are per chunk — the server's receive time). */
function decorate(chunk: LogChunk, showTimestamps: boolean): string {
  let content = chunk.content;
  switch (chunk.stream) {
    case 'stderr':
      content = `\x1b[31m${content}\x1b[0m`;
      break;
    case 'system':
      content = `\x1b[2m${content}\x1b[0m`;
      break;
    default:
      break;
  }
  if (showTimestamps && chunk.createdAt) {
    const time = new Date(chunk.createdAt);
    if (!Number.isNaN(time.getTime())) {
      const hh = String(time.getHours()).padStart(2, '0');
      const mm = String(time.getMinutes()).padStart(2, '0');
      const ss = String(time.getSeconds()).padStart(2, '0');
      content = `\x1b[2m${hh}:${mm}:${ss}\x1b[0m ${content}`;
    }
  }
  return content;
}

/** Match highlighting in the shell's primary tint. */
const SEARCH_DECORATIONS = {
  matchBackground: '#5645d433',
  activeMatchBackground: '#5645d466',
  matchOverviewRuler: '#5645d4',
  activeMatchColorOverviewRuler: '#5645d4',
};

/**
 * Streaming observability console: xterm with the shell's light theme,
 * incremental appends (no re-render of past lines), substring/regex/
 * case-sensitive search, per-chunk timestamps, copy-to-clipboard, and raw
 * download. Only rendered lines live in the terminal's scrollback — the
 * chunk buffer itself stays in the zustand store. Line numbers are
 * deliberately absent: xterm has no gutter primitive, and prefixing every
 * wrapped line would break copy and search fidelity.
 */
export function LogViewer({ workspaceId, pipelineId, job, onWatch }: LogViewerProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const renderedRef = useRef<{ jobId: string; count: number; lastSeq: number; stamped: boolean }>({
    jobId: '',
    count: 0,
    lastSeq: -1,
    stamped: false,
  });
  const [query, setQuery] = useState('');
  const [useRegex, setUseRegex] = useState(false);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [showTimestamps, setShowTimestamps] = useState(false);

  const chunks = useLogStore((state) => state.jobs[job.id]?.chunks) ?? EMPTY_CHUNKS;

  // One terminal for the component's lifetime.
  useEffect(() => {
    const element = containerRef.current;
    if (!element) return undefined;

    const terminal = new Terminal({
      convertEol: true,
      disableStdin: true,
      cursorBlink: false,
      fontSize: 12,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      scrollback: 10_000,
      theme: {
        background: '#ffffff',
        foreground: '#37352f',
        cursor: '#ffffff',
        selectionBackground: '#5645d433',
        red: '#c62828',
        blue: '#0075de',
        magenta: '#5645d4',
      },
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    terminal.loadAddon(fit);
    terminal.loadAddon(search);
    terminal.loadAddon(new WebLinksAddon());
    terminal.open(element);
    fit.fit();

    terminalRef.current = terminal;
    searchRef.current = search;
    fitRef.current = fit;

    const observer = new ResizeObserver(() => fitRef.current?.fit());
    observer.observe(element);

    return () => {
      observer.disconnect();
      terminal.dispose();
      terminalRef.current = null;
      searchRef.current = null;
      fitRef.current = null;
    };
  }, []);

  // Follow the selected job.
  useEffect(() => {
    onWatch(job.id);
  }, [job.id, onWatch]);

  // Incremental append; full rewrite when the job changes, backfill
  // inserted earlier chunks behind what we already rendered, or the
  // timestamp toggle flipped (prefixes change every rendered line).
  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const rendered = renderedRef.current;

    const needsReset =
      rendered.jobId !== job.id ||
      rendered.stamped !== showTimestamps ||
      chunks.length < rendered.count ||
      (rendered.count > 0 && chunks[rendered.count - 1]?.seq !== rendered.lastSeq);

    if (needsReset) {
      terminal.reset();
      rendered.jobId = job.id;
      rendered.stamped = showTimestamps;
      rendered.count = 0;
      rendered.lastSeq = -1;
    }

    for (let index = rendered.count; index < chunks.length; index += 1) {
      const chunk = chunks[index];
      terminal.write(decorate(chunk, showTimestamps));
      if (!chunk.content.endsWith('\n')) terminal.write('\r\n');
      rendered.lastSeq = chunk.seq;
    }
    rendered.count = chunks.length;
  }, [chunks, job.id, showTimestamps]);

  const downloadUrl = useMemo(
    () => rawLogUrl(workspaceId, pipelineId, job.id),
    [workspaceId, pipelineId, job.id],
  );

  const runSearch = (backwards: boolean) => {
    if (!query) return;
    const options = { regex: useRegex, caseSensitive, decorations: SEARCH_DECORATIONS };
    try {
      if (backwards) searchRef.current?.findPrevious(query, options);
      else searchRef.current?.findNext(query, options);
    } catch {
      // An unfinished regex (e.g. a lone "(") throws; ignore while typing.
    }
  };

  const copyAll = async () => {
    const buffered = useLogStore.getState().jobs[job.id]?.chunks ?? [];
    const text = buffered
      .map((chunk) => (chunk.content.endsWith('\n') ? chunk.content : `${chunk.content}\n`))
      .join('');
    try {
      await navigator.clipboard.writeText(text);
      toast.success('Log copied to clipboard.');
    } catch {
      toast.error('Could not access the clipboard.');
    }
  };

  const toggleClasses = (active: boolean) =>
    cn(
      'inline-flex h-7 w-7 shrink-0 items-center justify-center rounded border text-xs transition-colors',
      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
      active
        ? 'border-primary/40 bg-primary/10 text-primary'
        : 'border-steel/20 text-steel hover:bg-surface',
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-1.5 border-b border-steel/10 px-2 py-1.5">
        <div className="relative min-w-0 flex-1">
          <Search
            size={13}
            className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-steel"
            aria-hidden="true"
          />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') runSearch(event.shiftKey);
            }}
            placeholder="Search logs (Enter next, Shift+Enter previous)"
            className="h-7 w-full rounded border border-steel/20 bg-canvas pl-7 pr-2 text-xs text-charcoal placeholder:text-steel/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            aria-label="Search logs"
          />
        </div>
        <button
          type="button"
          className={toggleClasses(useRegex)}
          onClick={() => setUseRegex((v) => !v)}
          title="Regular-expression search"
          aria-pressed={useRegex}
          aria-label="Toggle regular-expression search"
        >
          <Regex size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={toggleClasses(caseSensitive)}
          onClick={() => setCaseSensitive((v) => !v)}
          title="Case-sensitive search"
          aria-pressed={caseSensitive}
          aria-label="Toggle case-sensitive search"
        >
          <CaseSensitive size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={toggleClasses(showTimestamps)}
          onClick={() => setShowTimestamps((v) => !v)}
          title="Show per-chunk timestamps"
          aria-pressed={showTimestamps}
          aria-label="Toggle timestamps"
        >
          <Clock size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={toggleClasses(false)}
          onClick={() => void copyAll()}
          title="Copy the full log to the clipboard"
          aria-label="Copy log"
        >
          <Copy size={13} aria-hidden="true" />
        </button>
        <a
          href={downloadUrl}
          download
          className="inline-flex h-7 shrink-0 items-center gap-1 rounded px-2 text-xs font-medium text-charcoal transition-colors hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <Download size={12} aria-hidden="true" />
          Raw
        </a>
      </div>
      <div ref={containerRef} className="min-h-0 flex-1 bg-canvas p-1" />
      {chunks.length === 0 && (
        <p className="border-t border-steel/10 px-3 py-2 text-xs text-steel">
          {job.status === 'queued'
            ? 'Waiting for a runner — logs stream here the moment execution starts.'
            : 'No log output yet.'}
        </p>
      )}
    </div>
  );
}
