import { Bot } from 'lucide-react';

import { cn } from '../../lib/cn';

type AvatarSize = 'xs' | 'sm' | 'md';

const SIZE_CLASSES: Record<AvatarSize, string> = {
  xs: 'h-5 w-5 text-[10px]',
  sm: 'h-6 w-6 text-xs',
  md: 'h-8 w-8 text-sm',
};

const ICON_SIZES: Record<AvatarSize, number> = { xs: 12, sm: 14, md: 16 };

interface AvatarProps {
  login: string | null;
  avatarUrl: string | null;
  size?: AvatarSize;
  className?: string;
  /** Optional hover text (e.g. the login when adjacent text differs). */
  title?: string;
}

/**
 * Circular identity avatar — the one place the design system permits
 * `rounded-full` (avatars/spinners only). Falls back to a monogram for
 * identities without an image and a bot glyph for system actions (webhook
 * sync, scheduler, provisioner). URLs are https-checked server-side; the
 * client re-checks as defense in depth and never interpolates them anywhere
 * but an <img src> with referrer suppressed.
 */
export function Avatar({ login, avatarUrl, size = 'md', className, title }: AvatarProps) {
  if (avatarUrl && avatarUrl.startsWith('https://')) {
    return (
      <img
        src={avatarUrl}
        alt=""
        title={title}
        referrerPolicy="no-referrer"
        loading="lazy"
        className={cn('shrink-0 rounded-full object-cover select-none', SIZE_CLASSES[size], className)}
      />
    );
  }
  if (login) {
    return (
      <span
        aria-hidden="true"
        title={title}
        className={cn(
          'flex shrink-0 items-center justify-center rounded-full bg-primary/10 font-semibold text-primary',
          SIZE_CLASSES[size],
          className,
        )}
      >
        {login.charAt(0).toUpperCase()}
      </span>
    );
  }
  return (
    <span
      aria-hidden="true"
      className={cn(
        'flex shrink-0 items-center justify-center rounded-full bg-surface text-steel',
        SIZE_CLASSES[size],
        className,
      )}
      title={title ?? 'System'}
    >
      <Bot size={ICON_SIZES[size]} aria-hidden="true" />
    </span>
  );
}
