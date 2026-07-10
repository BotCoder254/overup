import { type ReactNode, useEffect, useId, useRef } from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../../lib/cn';

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  children?: ReactNode;
  /** Action row, right-aligned (e.g. Cancel + destructive confirm). */
  footer?: ReactNode;
  className?: string;
}

/**
 * Minimal modal dialog: portal, navy scrim, focus containment, Escape and
 * backdrop close. Used for confirmations (e.g. removing a repository) —
 * routine flows should stay inline on the page instead.
 */
export function Dialog({ open, onClose, title, description, children, footer, className }: DialogProps) {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return undefined;
    restoreFocusRef.current = document.activeElement as HTMLElement | null;
    panelRef.current?.focus();

    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation();
        onClose();
        return;
      }
      // Containment: keep Tab cycling inside the panel.
      if (event.key === 'Tab' && panelRef.current) {
        const focusable = panelRef.current.querySelectorAll<HTMLElement>(
          'button, [href], input, textarea, select, [tabindex]:not([tabindex="-1"])',
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      restoreFocusRef.current?.focus();
    };
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <button
        type="button"
        aria-label="Close dialog"
        onClick={onClose}
        className="absolute inset-0 bg-navy/40"
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className={cn(
          'relative w-full max-w-md animate-scale-in rounded border border-steel/20 bg-canvas p-5 shadow-lg focus:outline-none',
          className,
        )}
      >
        <h2 id={titleId} className="text-base font-semibold text-charcoal">
          {title}
        </h2>
        {description && <p className="mt-1.5 text-sm leading-relaxed text-steel">{description}</p>}
        {children}
        {footer && <div className="mt-5 flex items-center justify-end gap-2">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}
