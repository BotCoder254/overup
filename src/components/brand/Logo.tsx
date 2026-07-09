import { Squirrel } from 'lucide-react';
import { cn } from '../../lib/cn';

const SIZES = {
  sm: { icon: 26, wordmark: 'text-lg' },
  md: { icon: 34, wordmark: 'text-2xl' },
  lg: { icon: 48, wordmark: 'text-3xl' },
  xl: { icon: 72, wordmark: 'text-5xl' },
} as const;

export type LogoSize = keyof typeof SIZES;

interface LogoProps {
  size?: LogoSize;
  withWordmark?: boolean;
  className?: string;
}

/**
 * The overup logo: the lucide Squirrel mark drawn in `currentColor` with no
 * background, optionally paired with the wordmark. Color follows the parent
 * text color, so the same component works on light and navy panels.
 */
export function Logo({ size = 'md', withWordmark = true, className }: LogoProps) {
  const { icon, wordmark } = SIZES[size];

  return (
    <span className={cn('inline-flex items-center gap-2.5', className)}>
      <Squirrel size={icon} strokeWidth={2} aria-hidden="true" />
      {withWordmark && (
        <span className={cn('font-display font-semibold leading-none tracking-tight', wordmark)}>
          overup
        </span>
      )}
    </span>
  );
}
