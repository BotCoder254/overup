/** Tiny heuristic UA parser for the sessions table — display only. */
export function deviceLabel(userAgent: string | null): string {
  if (!userAgent) return 'Unknown device';

  const browser = userAgent.includes('Edg/')
    ? 'Edge'
    : userAgent.includes('OPR/') || userAgent.includes('Opera')
      ? 'Opera'
      : userAgent.includes('Firefox/')
        ? 'Firefox'
        : userAgent.includes('Chrome/')
          ? 'Chrome'
          : userAgent.includes('Safari/')
            ? 'Safari'
            : null;

  const os = userAgent.includes('Windows')
    ? 'Windows'
    : userAgent.includes('Android')
      ? 'Android'
      : userAgent.includes('iPhone') || userAgent.includes('iPad')
        ? 'iOS'
        : userAgent.includes('Mac OS X') || userAgent.includes('Macintosh')
          ? 'macOS'
          : userAgent.includes('Linux')
            ? 'Linux'
            : null;

  if (browser && os) return `${browser} on ${os}`;
  return browser ?? os ?? 'Unknown device';
}
