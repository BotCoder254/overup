import { create } from 'zustand';
import type { LogChunk } from '../../../types/pipeline';

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
