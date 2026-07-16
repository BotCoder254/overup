/** Compact duration between two instants, e.g. "1h 4m", "3m 12s", "8s". */
export function formatDuration(startIso: string | null, endIso: string | null): string {
  if (!startIso) return '—';
  const start = new Date(startIso).getTime();
  const end = endIso ? new Date(endIso).getTime() : Date.now();
  const totalSeconds = Math.max(Math.round((end - start) / 1000), 0);

  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

/**
 * "refs/heads/main" -> "main", "refs/tags/v1" -> "v1",
 * "refs/pull/7/head" -> "PR #7"; other refs pass through untouched.
 */
export function branchOfRef(ref: string): string {
  if (ref.startsWith('refs/heads/')) return ref.slice('refs/heads/'.length);
  if (ref.startsWith('refs/tags/')) return ref.slice('refs/tags/'.length);
  const pull = ref.match(/^refs\/pull\/(\d+)\/head$/);
  if (pull) return `PR #${pull[1]}`;
  return ref;
}

export function formatBytes(bytes: number | null): string {
  if (bytes === null || bytes < 0) return '—';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
