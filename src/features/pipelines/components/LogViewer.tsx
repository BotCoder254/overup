import {
  AlertTriangle,
  ArrowDownToLine,
  CaseSensitive,
  ChevronDown,
  ChevronUp,
  Clock,
  Copy,
  Download,
  Layers,
  Pause,
  Play,
  Regex,
  Search,
  Sigma,
  WrapText,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { toast } from 'sonner';
import '@xterm/xterm/css/xterm.css';
import { Popover } from '../../../components/ui/Popover';
import { cn } from '../../../lib/cn';
import type { LogChunk, PipelineEvent, PipelineJob } from '../../../types/pipeline';
import {
  buildSections,
  chunkSectionKey,
  classifyChunkSeverity,
  computeStats,
  isOverflowMarker,
  useLogStore,
} from '../stores/logStore';
import { rawLogUrl } from '../api/pipelinesApi';

interface LogViewerProps {
  workspaceId: string;
  pipelineId: string;
  job: PipelineJob;
  /** Ask the stream layer to backfill + follow this job's logs. */
  onWatch: (jobId: string) => void;
  /** Job events (job.step_finished carries the structured failed step). */
  events?: PipelineEvent[];
  /** Lifted section visibility shared with the step list; when absent the
   * viewer manages its own. */
  hiddenSections?: Set<string>;
  onToggleSection?: (key: string) => void;
  /** External "scroll the terminal to this section" request (step list). */
  jumpToSection?: { key: string; nonce: number } | null;
}

const EMPTY_CHUNKS: LogChunk[] = [];
const EMPTY_EVENTS: PipelineEvent[] = [];

/** Columns used when line wrapping is off (horizontal scroll instead). */
const NOWRAP_COLS = 512;

type SeverityFilter = 'all' | 'warnings' | 'errors';

const SEVERITY_LABELS: Record<SeverityFilter, string> = {
  all: 'All output',
  warnings: 'Warnings + errors',
  errors: 'Errors only',
};

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

function formatBytesShort(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/**
 * Streaming observability console: xterm with the shell's light theme,
 * incremental appends (no re-render of past lines), substring/regex/
 * case-sensitive search, collapsible per-step/phase sections, a
 * presentation-only severity filter with error navigation, per-chunk
 * timestamps, pause/resume, autoscroll follow, wrap toggle, log stats,
 * copy-to-clipboard, and raw download. Only rendered lines live in the
 * terminal's scrollback — the chunk buffer itself stays in the zustand
 * store, so collapse/filter changes are a deterministic re-render from the
 * buffer. Line numbers are deliberately absent: xterm has no gutter
 * primitive, and prefixing every wrapped line would break copy and search
 * fidelity.
 */
export function LogViewer({
  workspaceId,
  pipelineId,
  job,
  onWatch,
  events,
  hiddenSections,
  onToggleSection,
  jumpToSection,
}: LogViewerProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const wrapRef = useRef(true);
  const programmaticScrollRef = useRef(false);
  /** Store chunk index -> terminal buffer line where the chunk starts. */
  const chunkLineIndexRef = useRef<Map<number, number>>(new Map());
  const renderedRef = useRef<{
    filterKey: string;
    count: number;
    lastSeq: number;
    /** Section whose collapse placeholder was last written. */
    placeholderKey: string | null;
  }>({ filterKey: '', count: 0, lastSeq: -1, placeholderKey: null });
  /** Index of the last error the prev/next buttons visited. */
  const jumpCursorRef = useRef<number>(-1);
  const pendingSectionScrollRef = useRef<string | null>(null);

  const [query, setQuery] = useState('');
  const [useRegex, setUseRegex] = useState(false);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [showTimestamps, setShowTimestamps] = useState(false);
  const [severityFilter, setSeverityFilter] = useState<SeverityFilter>('all');
  const [autoScroll, setAutoScroll] = useState(true);
  /** While paused, chunks past this count are buffered but not rendered. */
  const [pausedAtCount, setPausedAtCount] = useState<number | null>(null);
  const [wrap, setWrap] = useState(true);
  // Uncontrolled fallback when the section state is not lifted by the page.
  const [ownHidden, setOwnHidden] = useState<Set<string>>(new Set());

  const hidden = hiddenSections ?? ownHidden;
  const toggleSection = useCallback(
    (key: string) => {
      if (onToggleSection) onToggleSection(key);
      else {
        setOwnHidden((current) => {
          const next = new Set(current);
          if (next.has(key)) next.delete(key);
          else next.add(key);
          return next;
        });
      }
    },
    [onToggleSection],
  );

  const chunks = useLogStore((state) => state.jobs[job.id]?.chunks) ?? EMPTY_CHUNKS;
  const jobEvents = events ?? EMPTY_EVENTS;

  const sections = useMemo(() => buildSections(chunks, job.plan.steps), [chunks, job.plan.steps]);
  const stats = useMemo(() => computeStats(chunks), [chunks]);
  const hiddenKey = useMemo(() => Array.from(hidden).sort().join(','), [hidden]);

  /** Should this chunk be rendered under the current filters? */
  const isChunkVisible = useCallback(
    (chunk: LogChunk) => {
      if (isOverflowMarker(chunk)) return true;
      if (hidden.has(chunkSectionKey(chunk))) return false;
      if (severityFilter === 'all') return true;
      // System lines (step markers, lifecycle) stay for context.
      if (chunk.stream === 'system') return true;
      const severity = classifyChunkSeverity(chunk);
      if (severity === 'error') return true;
      return severityFilter === 'warnings' && severity === 'warning';
    },
    [hidden, severityFilter],
  );

  /** Visible chunk indexes classified as errors — the jump targets. */
  const errorTargets = useMemo(() => {
    const targets: number[] = [];
    chunks.forEach((chunk, index) => {
      if (isChunkVisible(chunk) && classifyChunkSeverity(chunk) === 'error') {
        targets.push(index);
      }
    });
    return targets;
  }, [chunks, isChunkVisible]);

  /** First chunk of the structurally failed step (preferred jump target). */
  const failureTarget = useMemo(() => {
    const failed = jobEvents.find(
      (event) =>
        event.eventType === 'job.step_finished' &&
        event.payload?.status === 'failed' &&
        (event.payload?.attempt == null || event.payload.attempt === job.attempt),
    );
    if (failed && typeof failed.payload.stepIndex === 'number') {
      const key = `step:${failed.payload.stepIndex}`;
      const index = chunks.findIndex((chunk) => chunkSectionKey(chunk) === key);
      if (index >= 0) return index;
    }
    return errorTargets.length > 0 ? errorTargets[0] : null;
  }, [jobEvents, job.attempt, chunks, errorTargets]);

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

    // Scrolling away from the bottom disengages follow mode; scrolling back
    // to the bottom re-engages it (programmatic scrolls are exempt).
    terminal.onScroll((viewportY) => {
      if (programmaticScrollRef.current) return;
      const atBottom = viewportY >= terminal.buffer.active.baseY;
      setAutoScroll(atBottom);
    });

    const observer = new ResizeObserver(() => {
      if (wrapRef.current) {
        fitRef.current?.fit();
      } else {
        const rows = fitRef.current?.proposeDimensions()?.rows ?? terminal.rows;
        terminal.resize(NOWRAP_COLS, rows);
      }
    });
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

  // Wrap toggle: fit-to-container (wrap) vs fixed wide columns with
  // horizontal scroll (no wrap). Reflow moves buffer lines, so the render
  // effect below rebuilds (wrap is part of its filter key).
  useEffect(() => {
    wrapRef.current = wrap;
    const terminal = terminalRef.current;
    const fit = fitRef.current;
    if (!terminal || !fit) return;
    if (wrap) {
      fit.fit();
    } else {
      const rows = fit.proposeDimensions()?.rows ?? terminal.rows;
      terminal.resize(NOWRAP_COLS, rows);
    }
  }, [wrap]);

  /** Scroll the terminal so the given store chunk's first line is on top. */
  const scrollToChunk = useCallback((index: number) => {
    const terminal = terminalRef.current;
    const line = chunkLineIndexRef.current.get(index);
    if (!terminal || line === undefined) return;
    programmaticScrollRef.current = true;
    terminal.scrollToLine(line);
    programmaticScrollRef.current = false;
    setAutoScroll(false);
  }, []);

  // Incremental append; full rewrite when the job or any render-affecting
  // toggle changes, the buffer shrank, or backfill inserted earlier chunks
  // behind what we already rendered. Collapsed sections render as one dim
  // placeholder line; the chunk->line index (used by error/section jumps)
  // is captured exactly via zero-length write callbacks, which fire after
  // every previously queued write has been processed.
  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const rendered = renderedRef.current;
    const filterKey = `${job.id}|${showTimestamps}|${severityFilter}|${wrap}|${hiddenKey}`;
    const limit = pausedAtCount === null ? chunks.length : Math.min(pausedAtCount, chunks.length);

    const needsReset =
      rendered.filterKey !== filterKey ||
      chunks.length < rendered.count ||
      (rendered.count > 0 && chunks[rendered.count - 1]?.seq !== rendered.lastSeq);

    if (needsReset) {
      terminal.reset();
      rendered.filterKey = filterKey;
      rendered.count = 0;
      rendered.lastSeq = -1;
      rendered.placeholderKey = null;
      chunkLineIndexRef.current = new Map();
      jumpCursorRef.current = -1;
    }

    if (rendered.count >= limit) return;

    const sectionByKey = new Map(sections.map((section) => [section.key, section]));
    let wrote = false;
    for (let index = rendered.count; index < limit; index += 1) {
      const chunk = chunks[index];
      const key = chunkSectionKey(chunk);
      if (!isOverflowMarker(chunk) && hidden.has(key)) {
        if (rendered.placeholderKey !== key) {
          rendered.placeholderKey = key;
          const section = sectionByKey.get(key);
          terminal.write(
            `\x1b[2m▸ ${section?.label ?? 'section'} — ${
              section?.lineCount ?? 0
            } lines hidden\x1b[0m\r\n`,
          );
          wrote = true;
        }
      } else if (isChunkVisible(chunk)) {
        rendered.placeholderKey = null;
        // Zero-length write: its callback runs after all queued output is
        // processed and before this chunk's text — the exact start row.
        terminal.write('', () => {
          const buffer = terminal.buffer.active;
          chunkLineIndexRef.current.set(index, buffer.baseY + buffer.cursorY);
        });
        terminal.write(decorate(chunk, showTimestamps));
        if (!chunk.content.endsWith('\n')) terminal.write('\r\n');
        wrote = true;
      }
      rendered.lastSeq = chunk.seq;
    }
    rendered.count = limit;

    if (wrote && autoScroll) {
      terminal.write('', () => {
        programmaticScrollRef.current = true;
        terminal.scrollToBottom();
        programmaticScrollRef.current = false;
      });
    }

    // A step-list jump that arrived before this render pass had lines.
    const pendingKey = pendingSectionScrollRef.current;
    if (pendingKey) {
      const target = chunks.findIndex(
        (chunk, index) =>
          chunkSectionKey(chunk) === pendingKey && chunkLineIndexRef.current.has(index),
      );
      if (target >= 0) {
        pendingSectionScrollRef.current = null;
        terminal.write('', () => scrollToChunk(target));
      }
    }
  }, [
    chunks,
    job.id,
    showTimestamps,
    severityFilter,
    wrap,
    hiddenKey,
    hidden,
    pausedAtCount,
    autoScroll,
    sections,
    isChunkVisible,
    scrollToChunk,
  ]);

  // External jump request from the step list: unhide the section if needed,
  // then scroll to its first rendered chunk (deferred until lines exist).
  useEffect(() => {
    if (!jumpToSection) return;
    const { key } = jumpToSection;
    if (hidden.has(key)) {
      pendingSectionScrollRef.current = key;
      toggleSection(key);
      return;
    }
    const target = chunks.findIndex(
      (chunk, index) => chunkSectionKey(chunk) === key && chunkLineIndexRef.current.has(index),
    );
    if (target >= 0) scrollToChunk(target);
    else pendingSectionScrollRef.current = key;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jumpToSection?.nonce]);

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

  const jumpToFailure = () => {
    if (failureTarget === null) {
      toast.info('No failure detected in this log.');
      return;
    }
    jumpCursorRef.current = errorTargets.indexOf(failureTarget);
    scrollToChunk(failureTarget);
  };

  const stepError = (direction: 1 | -1) => {
    if (errorTargets.length === 0) {
      toast.info('No error lines detected.');
      return;
    }
    const next =
      (jumpCursorRef.current + direction + errorTargets.length) % errorTargets.length;
    jumpCursorRef.current = next;
    scrollToChunk(errorTargets[next]);
  };

  const paused = pausedAtCount !== null;
  const bufferedWhilePaused = paused ? Math.max(chunks.length - pausedAtCount, 0) : 0;

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
      <div className="flex flex-wrap items-center gap-1.5 border-b border-steel/10 px-2 py-1.5">
        <div className="relative min-w-[140px] flex-1">
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

        <span className="h-4 w-px shrink-0 bg-steel/20" aria-hidden="true" />

        <Popover
          ariaLabel="Severity filter"
          align="end"
          renderTrigger={(props) => (
            <button
              type="button"
              {...props}
              className={toggleClasses(severityFilter !== 'all')}
              title={`Severity filter: ${SEVERITY_LABELS[severityFilter]}`}
              aria-label="Severity filter"
            >
              <AlertTriangle size={13} aria-hidden="true" />
            </button>
          )}
        >
          {({ close }) => (
            <div className="min-w-[180px]">
              {(Object.keys(SEVERITY_LABELS) as SeverityFilter[]).map((level) => (
                <button
                  key={level}
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  onClick={() => {
                    setSeverityFilter(level);
                    close();
                  }}
                  className={cn(
                    'flex w-full items-center justify-between gap-2 rounded px-2.5 py-1.5 text-left text-xs transition-colors hover:bg-surface focus-visible:bg-surface focus-visible:outline-none',
                    severityFilter === level
                      ? 'font-semibold text-primary'
                      : 'text-charcoal',
                  )}
                >
                  {SEVERITY_LABELS[level]}
                  {level === 'errors' && stats.errors > 0 && (
                    <span className="font-mono text-[10px] text-danger">{stats.errors}</span>
                  )}
                  {level === 'warnings' && stats.warnings > 0 && (
                    <span className="font-mono text-[10px] text-steel">
                      {stats.warnings + stats.errors}
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}
        </Popover>
        <button
          type="button"
          className={toggleClasses(false)}
          onClick={jumpToFailure}
          title="Jump to the first failure"
          aria-label="Jump to failure"
        >
          <AlertTriangle size={13} className="text-danger" aria-hidden="true" />
        </button>
        <button
          type="button"
          className={toggleClasses(false)}
          onClick={() => stepError(-1)}
          title="Previous error"
          aria-label="Previous error"
        >
          <ChevronUp size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={toggleClasses(false)}
          onClick={() => stepError(1)}
          title="Next error"
          aria-label="Next error"
        >
          <ChevronDown size={13} aria-hidden="true" />
        </button>

        <span className="h-4 w-px shrink-0 bg-steel/20" aria-hidden="true" />

        <Popover
          ariaLabel="Log sections"
          align="end"
          renderTrigger={(props) => (
            <button
              type="button"
              {...props}
              className={toggleClasses(hidden.size > 0)}
              title="Show or hide execution sections"
              aria-label="Log sections"
            >
              <Layers size={13} aria-hidden="true" />
            </button>
          )}
        >
          {() => (
            <div className="max-h-72 min-w-[220px] overflow-y-auto">
              {sections.length === 0 ? (
                <p className="px-2.5 py-2 text-xs text-steel">No sections yet.</p>
              ) : (
                sections.map((section) => (
                  <label
                    key={section.key}
                    className="flex w-full cursor-pointer items-center gap-2 rounded px-2.5 py-1.5 text-xs text-charcoal transition-colors hover:bg-surface"
                  >
                    <input
                      type="checkbox"
                      checked={!hidden.has(section.key)}
                      onChange={() => toggleSection(section.key)}
                      className="h-3.5 w-3.5 rounded border-steel/40 text-primary focus-visible:ring-primary"
                    />
                    <span className="min-w-0 flex-1 truncate">{section.label}</span>
                    <span className="shrink-0 font-mono text-[10px] text-steel">
                      {section.lineCount}
                    </span>
                    {section.errorCount > 0 && (
                      <span className="shrink-0 font-mono text-[10px] text-danger">
                        {section.errorCount}!
                      </span>
                    )}
                  </label>
                ))
              )}
            </div>
          )}
        </Popover>
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
          className={toggleClasses(wrap)}
          onClick={() => setWrap((v) => !v)}
          title={wrap ? 'Wrap lines (on)' : 'Wrap lines (off — scroll horizontally)'}
          aria-pressed={wrap}
          aria-label="Toggle line wrapping"
        >
          <WrapText size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={toggleClasses(autoScroll)}
          onClick={() => {
            const next = !autoScroll;
            setAutoScroll(next);
            if (next) {
              const terminal = terminalRef.current;
              if (terminal) {
                programmaticScrollRef.current = true;
                terminal.scrollToBottom();
                programmaticScrollRef.current = false;
              }
            }
          }}
          title="Follow new output (autoscroll)"
          aria-pressed={autoScroll}
          aria-label="Toggle autoscroll"
        >
          <ArrowDownToLine size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={toggleClasses(paused)}
          onClick={() =>
            setPausedAtCount((current) => (current === null ? chunks.length : null))
          }
          title={paused ? 'Resume streaming' : 'Pause streaming (buffer keeps filling)'}
          aria-pressed={paused}
          aria-label={paused ? 'Resume streaming' : 'Pause streaming'}
        >
          {paused ? <Play size={13} aria-hidden="true" /> : <Pause size={13} aria-hidden="true" />}
        </button>

        <span className="h-4 w-px shrink-0 bg-steel/20" aria-hidden="true" />

        <Popover
          ariaLabel="Log statistics"
          align="end"
          role="dialog"
          renderTrigger={(props) => (
            <button
              type="button"
              {...props}
              className={toggleClasses(false)}
              title="Log statistics"
              aria-label="Log statistics"
            >
              <Sigma size={13} aria-hidden="true" />
            </button>
          )}
        >
          {() => (
            <dl className="min-w-[200px] space-y-1 px-2.5 py-2 text-xs">
              <StatRow label="Lines" value={String(stats.lines)} />
              <StatRow label="Chunks" value={String(stats.chunks)} />
              <StatRow label="Buffered" value={formatBytesShort(stats.bytes)} />
              <StatRow label="Total (server)" value={formatBytesShort(job.logBytes)} />
              <StatRow
                label="Errors"
                value={String(stats.errors)}
                valueClassName={stats.errors > 0 ? 'text-danger' : undefined}
              />
              <StatRow label="Warnings" value={String(stats.warnings)} />
              <StatRow label="stdout / stderr / system"
                value={`${stats.byStream.stdout} / ${stats.byStream.stderr} / ${stats.byStream.system}`}
              />
            </dl>
          )}
        </Popover>
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
      <div
        ref={containerRef}
        className={cn('min-h-0 flex-1 bg-canvas p-1', !wrap && 'overflow-x-auto')}
      />
      {paused && (
        <p className="border-t border-steel/10 bg-surface/50 px-3 py-1.5 text-xs text-steel">
          Streaming paused
          {bufferedWhilePaused > 0 && (
            <> — {bufferedWhilePaused} new chunk{bufferedWhilePaused === 1 ? '' : 's'} buffered</>
          )}
          . Press play to catch up.
        </p>
      )}
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

function StatRow({
  label,
  value,
  valueClassName,
}: {
  label: string;
  value: string;
  valueClassName?: string;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <dt className="text-steel">{label}</dt>
      <dd className={cn('font-mono text-charcoal', valueClassName)}>{value}</dd>
    </div>
  );
}
