import { type TextareaHTMLAttributes, forwardRef } from 'react';
import { cn } from '../../lib/cn';

export const Textarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement>
>(({ className, rows = 3, ...props }, ref) => (
  <textarea
    ref={ref}
    rows={rows}
    className={cn(
      'w-full resize-y rounded border border-steel/20 bg-canvas px-3.5 py-2.5 text-sm text-charcoal transition-colors',
      'placeholder:text-steel focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
      'disabled:cursor-not-allowed disabled:opacity-60',
      'aria-[invalid=true]:border-danger aria-[invalid=true]:focus-visible:ring-danger',
      className,
    )}
    {...props}
  />
));

Textarea.displayName = 'Textarea';
