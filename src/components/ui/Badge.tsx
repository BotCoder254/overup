import { cva, type VariantProps } from 'class-variance-authority';
import type { ReactNode } from 'react';
import { cn } from '../../lib/cn';

const badgeVariants = cva(
  'inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs font-medium',
  {
    variants: {
      variant: {
        neutral: 'bg-surface text-steel',
        primary: 'bg-primary/10 text-primary',
        info: 'bg-link/10 text-link',
        danger: 'bg-danger/10 text-danger',
        success: 'bg-primary/10 text-primary',
        outline: 'border border-steel/20 bg-canvas text-steel',
      },
    },
    defaultVariants: {
      variant: 'neutral',
    },
  },
);

interface BadgeProps extends VariantProps<typeof badgeVariants> {
  children: ReactNode;
  className?: string;
}

/** Small status/label chip. Solid palette tints only — never a gradient. */
export function Badge({ variant, className, children }: BadgeProps) {
  return <span className={cn(badgeVariants({ variant }), className)}>{children}</span>;
}
