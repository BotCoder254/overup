import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';
import { cn } from '../../lib/cn';

interface EmptyStateProps {
  /** Large lucide icon drawn in `currentColor` — never given a background. */
  icon: LucideIcon;
  title: string;
  description: string;
  action?: ReactNode;
  className?: string;
}

/** Shared empty state: centered large icon, title, description, optional action. */
export function EmptyState({ icon: Icon, title, description, action, className }: EmptyStateProps) {
  return (
    <div
      className={cn(
        'flex min-h-[60vh] animate-fade-in flex-col items-center justify-center gap-4 rounded border border-steel/20 bg-canvas p-8 text-center sm:p-12',
        className,
      )}
    >
      <Icon size={64} strokeWidth={1.25} className="text-steel" aria-hidden="true" />
      <div className="space-y-2">
        <h2 className="text-lg font-semibold sm:text-xl">{title}</h2>
        <p className="mx-auto max-w-md leading-relaxed text-steel">{description}</p>
      </div>
      {action}
    </div>
  );
}
