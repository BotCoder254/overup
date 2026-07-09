import { cn } from '../../lib/cn';

interface SpinnerProps {
  className?: string;
}

/**
 * Modern SVG arc spinner: a faint circular track with a rotating quarter
 * arc, both drawn in `currentColor` so it adapts to any surface (steel on
 * light panels, white inside primary buttons). Size via className.
 */
export function Spinner({ className }: SpinnerProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      role="status"
      aria-label="Loading"
      className={cn('h-5 w-5 animate-spin', className)}
    >
      <circle
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeWidth="3"
        className="opacity-25"
      />
      <path
        d="M12 2a10 10 0 0 1 10 10"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** Centered full-viewport loading state used while auth boots. */
export function FullScreenLoader() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-canvas">
      <Spinner className="h-7 w-7 text-steel" />
    </div>
  );
}
