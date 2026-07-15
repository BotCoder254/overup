import { create } from 'zustand';
import type { LogChunk, LogStreamName, PipelineJobPlanStep } from '../../../types/pipeline';

/**
 * Client-side buffer for streamed job logs. Chunks arrive from two sources
 * (REST backfill and the live WebSocket) with runner-monotonic sequence
 * numbers; the store dedupes on seq and keeps each job's chunks ordered so
 * the viewer can render deterministically.
 */
interface JobLogBuffer {
  seqs: Set<number>;
  chunks: LogChunk[];
}

interface LogStoreState {
  jobs: Record<string, JobLogBuffer>;
  append: (jobId: string, chunks: LogChunk[]) => void;
  lastSeq: (jobId: string) => number;
  clear: () => void;
}

export const useLogStore = create<LogStoreState>((set, get) => ({
  jobs: {},

  append: (jobId, incoming) => {
    if (incoming.length === 0) return;
    set((state) => {
      const existing = state.jobs[jobId] ?? { seqs: new Set<number>(), chunks: [] };
      const fresh = incoming.filter((chunk) => !existing.seqs.has(chunk.seq));
      if (fresh.length === 0) return state;

      const seqs = new Set(existing.seqs);
      for (const chunk of fresh) seqs.add(chunk.seq);

      let chunks = [...existing.chunks, ...fresh];
      // Backfill and live streams can interleave; restore order when needed.
      const lastExisting = existing.chunks[existing.chunks.length - 1];
      if (lastExisting && fresh[0].seq < lastExisting.seq) {
        chunks = chunks.sort((a, b) => a.seq - b.seq);
      }

      return { jobs: { ...state.jobs, [jobId]: { seqs, chunks } } };
    });
  },

  lastSeq: (jobId) => {
    const buffer = get().jobs[jobId];
    if (!buffer || buffer.chunks.length === 0) return -1;
    return buffer.chunks[buffer.chunks.length - 1].seq;
  },

  clear: () => set({ jobs: {} }),
}));

// ---------------------------------------------------------------------------
// Pure log-analysis helpers. Everything below is presentation-only and runs
// client-side over the already-masked chunks in this store — severity is
// never persisted or trusted anywhere.
// ---------------------------------------------------------------------------

export type LogSeverity = 'error' | 'warning';

/**
 * The overflow marker sits at i64::MAX (beyond JS's safe range after JSON
 * parsing). It is pinned: always rendered, never inside a section.
 */
export function isOverflowMarker(chunk: LogChunk): boolean {
  return !Number.isSafeInteger(chunk.seq);
}

/** Stable key for the collapsible section a chunk belongs to. */
export function chunkSectionKey(chunk: LogChunk): string {
  if (isOverflowMarker(chunk)) return 'pinned';
  if (chunk.phase === 'steps' && chunk.stepIndex != null) return `step:${chunk.stepIndex}`;
  if (chunk.phase) return `phase:${chunk.phase}`;
  // Old-runner / pre-migration chunks degrade into one flat section.
  return 'output';
}

const PHASE_LABELS: Record<string, string> = {
  checkout: 'Checkout',
  image_pull: 'Image pull',
  container: 'Container',
  steps: 'Steps',
  artifacts: 'Artifacts',
  cleanup: 'Cleanup',
  output: 'Output',
};

export interface LogSection {
  key: string;
  label: string;
  stepIndex: number | null;
  chunkCount: number;
  lineCount: number;
  errorCount: number;
  warningCount: number;
}

/**
 * Group chunks into ordered collapsible sections (first-appearance order).
 * Step labels come from the job's signed plan, never from log text. The
 * pinned overflow marker is excluded — it renders outside every section.
 */
export function buildSections(
  chunks: LogChunk[],
  planSteps: PipelineJobPlanStep[],
): LogSection[] {
  const sections = new Map<string, LogSection>();
  for (const chunk of chunks) {
    const key = chunkSectionKey(chunk);
    if (key === 'pinned') continue;
    let section = sections.get(key);
    if (!section) {
      let label = PHASE_LABELS[key.replace('phase:', '')] ?? 'Output';
      let stepIndex: number | null = null;
      if (key.startsWith('step:')) {
        stepIndex = Number(key.slice('step:'.length));
        label = planSteps[stepIndex]?.name || `Step ${stepIndex + 1}`;
      }
      section = {
        key,
        label,
        stepIndex,
        chunkCount: 0,
        lineCount: 0,
        errorCount: 0,
        warningCount: 0,
      };
      sections.set(key, section);
    }
    section.chunkCount += 1;
    section.lineCount += countLines(chunk.content);
    const severity = classifyChunkSeverity(chunk);
    if (severity === 'error') section.errorCount += 1;
    else if (severity === 'warning') section.warningCount += 1;
  }
  return Array.from(sections.values());
}

function countLines(content: string): number {
  if (content.length === 0) return 0;
  let lines = 1;
  for (let i = 0; i < content.length; i += 1) {
    if (content.charCodeAt(i) === 10 && i < content.length - 1) lines += 1;
  }
  return lines;
}

const ERROR_RE =
  /\b(error|errors|err!|fatal|panic(?:ked)?|exception|traceback|failed|failure|cannot|denied|refused)\b|\bexit code [1-9]/i;
const WARNING_RE = /\b(warn|warning|warnings|deprecated|deprecation)\b/i;

/**
 * Conservative, presentation-only severity for one log line. The structured
 * "step failed" system marker always classifies as an error; otherwise a
 * cautious keyword match (stderr lines get the same rules — plenty of tools
 * write ordinary progress to stderr, so the stream alone never escalates).
 */
export function classifySeverity(
  line: string,
  stream: LogStreamName,
): LogSeverity | null {
  if (stream === 'system' && line.startsWith('step failed')) return 'error';
  if (ERROR_RE.test(line)) return 'error';
  if (WARNING_RE.test(line)) return 'warning';
  return null;
}

/** Worst severity across a chunk's lines (chunks are the jump targets). */
export function classifyChunkSeverity(chunk: LogChunk): LogSeverity | null {
  let worst: LogSeverity | null = null;
  for (const line of chunk.content.split('\n')) {
    const severity = classifySeverity(line, chunk.stream);
    if (severity === 'error') return 'error';
    if (severity === 'warning') worst = 'warning';
  }
  return worst;
}

export interface LogStats {
  chunks: number;
  lines: number;
  bytes: number;
  errors: number;
  warnings: number;
  byStream: Record<LogStreamName, number>;
}

/** Aggregate counts for the toolbar stats popover. */
export function computeStats(chunks: LogChunk[]): LogStats {
  const stats: LogStats = {
    chunks: chunks.length,
    lines: 0,
    bytes: 0,
    errors: 0,
    warnings: 0,
    byStream: { stdout: 0, stderr: 0, system: 0 },
  };
  for (const chunk of chunks) {
    stats.lines += countLines(chunk.content);
    stats.bytes += chunk.content.length;
    stats.byStream[chunk.stream] += 1;
    const severity = classifyChunkSeverity(chunk);
    if (severity === 'error') stats.errors += 1;
    else if (severity === 'warning') stats.warnings += 1;
  }
  return stats;
}
