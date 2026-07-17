import { ArrowUpRight, Megaphone, X } from 'lucide-react';
import { useEffect, useState } from 'react';

/**
 * A full-width system-announcement drawer pinned to the very top of the app,
 * above the shell (sidebar + floating canvas) and spanning the whole viewport
 * on the same `bg-surface` band as the sidebar. Solid brand background, alert
 * icon, concise message, an optional action, and a dismiss control. Dismissal
 * is remembered per-announcement id in localStorage so a closed banner stays
 * closed across reloads — re-announcing later only needs a fresh `id`.
 */
interface AnnouncementBannerProps {
  /** Stable id — bump it to re-show a new announcement after prior dismissal. */
  id?: string;
  message?: React.ReactNode;
  actionLabel?: string;
  actionHref?: string;
}

const STORAGE_PREFIX = 'overup.banner.dismissed.';

export function AnnouncementBanner({
  id = 'workspace-capacity-2026',
  message = (
    <>
      <span className="font-semibold">Limited capacity —</span> to keep the platform fast on
      constrained server &amp; compute, the number of workspaces is currently capped. Thanks
      for your patience while we scale.
    </>
  ),
  actionLabel = 'Learn more',
  actionHref = 'https://github.com/BotCoder254/overup',
}: AnnouncementBannerProps) {
  const storageKey = `${STORAGE_PREFIX}${id}`;
  const [dismissed, setDismissed] = useState(true);

  // Read persisted dismissal after mount (avoids a first-paint flash of a
  // banner the user already closed).
  useEffect(() => {
    try {
      setDismissed(window.localStorage.getItem(storageKey) === '1');
    } catch {
      setDismissed(false);
    }
  }, [storageKey]);

  const close = () => {
    setDismissed(true);
    try {
      window.localStorage.setItem(storageKey, '1');
    } catch {
      /* storage unavailable — dismiss for this session only */
    }
  };

  if (dismissed) return null;

  return (
    <div role="status" aria-live="polite" className="shrink-0 bg-primary text-white">
      <div className="mx-auto flex items-center gap-2.5 px-3 py-2 sm:gap-3 sm:px-5">
        <span
          aria-hidden="true"
          className="hidden shrink-0 rounded bg-white/15 p-1.5 sm:inline-flex"
        >
          <Megaphone size={16} />
        </span>

        <p className="min-w-0 flex-1 text-[13px] leading-snug line-clamp-2 sm:line-clamp-none sm:text-sm">
          {message}
        </p>

        {actionLabel && actionHref && (
          <a
            href={actionHref}
            target="_blank"
            rel="noopener noreferrer"
            className="hidden shrink-0 items-center gap-1 rounded bg-white px-3 py-1.5 text-xs font-semibold text-primary transition-colors hover:bg-white/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-primary sm:inline-flex"
          >
            {actionLabel}
            <ArrowUpRight size={14} aria-hidden="true" />
          </a>
        )}

        <button
          type="button"
          onClick={close}
          aria-label="Dismiss announcement"
          className="shrink-0 rounded p-1.5 text-white/80 transition-colors hover:bg-white/15 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-primary"
        >
          <X size={18} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}
