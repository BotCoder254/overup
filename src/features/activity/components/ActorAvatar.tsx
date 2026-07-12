import { Bot } from 'lucide-react';

interface ActorAvatarProps {
  login: string | null;
  avatarUrl: string | null;
}

/**
 * Circular actor avatar for feed entries — the one place the design system
 * permits `rounded-full` (avatars/spinners only). Falls back to a monogram
 * for users without an avatar and a bot glyph for system actions
 * (webhook sync, scheduler, provisioner).
 */
export function ActorAvatar({ login, avatarUrl }: ActorAvatarProps) {
  if (avatarUrl) {
    return (
      <img
        src={avatarUrl}
        alt=""
        referrerPolicy="no-referrer"
        className="h-8 w-8 shrink-0 rounded-full object-cover"
      />
    );
  }
  if (login) {
    return (
      <span
        aria-hidden="true"
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary/10 text-sm font-semibold text-primary"
      >
        {login.charAt(0).toUpperCase()}
      </span>
    );
  }
  return (
    <span
      aria-hidden="true"
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-surface text-steel"
      title="System"
    >
      <Bot size={16} aria-hidden="true" />
    </span>
  );
}
