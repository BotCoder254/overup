import type { ReactNode } from 'react';
import { cn } from '../../lib/cn';

interface CardProps {
  children: ReactNode;
  className?: string;
}

/** The canonical content container: canvas surface, steel hairline, 6px radius. */
export function Card({ children, className }: CardProps) {
  return (
    <div className={cn('rounded border border-steel/20 bg-canvas', className)}>{children}</div>
  );
}

export function CardHeader({ children, className }: CardProps) {
  return (
    <div className={cn('flex items-center gap-3 border-b border-steel/10 px-4 py-3', className)}>
      {children}
    </div>
  );
}

export function CardBody({ children, className }: CardProps) {
  return <div className={cn('p-4', className)}>{children}</div>;
}
