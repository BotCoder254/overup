import type { LucideIcon } from 'lucide-react';
import type { KeyboardEvent, ReactNode, Ref } from 'react';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { cn } from '../../lib/cn';

interface TriggerProps {
  ref: Ref<HTMLButtonElement>;
  onClick: () => void;
  'aria-expanded': boolean;
  'aria-haspopup': 'menu' | 'dialog';
  'aria-controls': string;
}

interface PopoverProps {
  /** Render the trigger; spread these props onto a <button>. */
  renderTrigger: (triggerProps: TriggerProps, isOpen: boolean) => ReactNode;
  /** Panel content; call close() after activating an item. */
  children: (api: { close: () => void }) => ReactNode;
  side?: 'bottom' | 'top';
  align?: 'start' | 'end';
  role?: 'menu' | 'dialog';
  panelClassName?: string;
  ariaLabel: string;
}

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function enabledMenuItems(panel: HTMLElement): HTMLElement[] {
  return Array.from(
    panel.querySelectorAll<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])'),
  );
}

/**
 * Dependency-free popover anchored to its trigger (absolute positioning in a
 * relative wrapper — both shell triggers live in the never-scrolling sidebar,
 * so no portal or scroll tracking is needed). Handles focus trap and restore,
 * arrow-key roving over menu items, Escape, and outside-click dismissal.
 */
export function Popover({
  renderTrigger,
  children,
  side = 'bottom',
  align = 'start',
  role = 'menu',
  panelClassName,
  ariaLabel,
}: PopoverProps) {
  const [isOpen, setIsOpen] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const panelId = useId();

  const close = useCallback((restoreFocus = false) => {
    setIsOpen(false);
    if (restoreFocus) triggerRef.current?.focus();
  }, []);

  // Focus the first enabled menu item (or the panel itself) on open.
  useEffect(() => {
    if (!isOpen || !panelRef.current) return;
    const panel = panelRef.current;
    if (role === 'menu') {
      const first = enabledMenuItems(panel)[0];
      (first ?? panel).focus();
    } else {
      panel.focus();
    }
  }, [isOpen, role]);

  // Outside click closes without stealing focus back to the trigger.
  useEffect(() => {
    if (!isOpen) return;
    function onPointerDown(event: PointerEvent) {
      if (!wrapperRef.current?.contains(event.target as Node)) setIsOpen(false);
    }
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
  }, [isOpen]);

  function onPanelKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const panel = panelRef.current;
    if (!panel) return;

    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      close(true);
      return;
    }

    if (event.key === 'Tab') {
      // Trap: cycle through everything focusable inside the panel.
      const focusables = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
      return;
    }

    if (role !== 'menu') return;
    const items = enabledMenuItems(panel);
    if (items.length === 0) return;
    const index = items.indexOf(document.activeElement as HTMLElement);

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      items[(index + 1) % items.length].focus();
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      items[(index - 1 + items.length) % items.length].focus();
    } else if (event.key === 'Home') {
      event.preventDefault();
      items[0].focus();
    } else if (event.key === 'End') {
      event.preventDefault();
      items[items.length - 1].focus();
    }
  }

  return (
    <div ref={wrapperRef} className="relative">
      {renderTrigger(
        {
          ref: triggerRef,
          onClick: () => setIsOpen((open) => !open),
          'aria-expanded': isOpen,
          'aria-haspopup': role,
          'aria-controls': panelId,
        },
        isOpen,
      )}
      {isOpen && (
        <div
          ref={panelRef}
          id={panelId}
          role={role}
          aria-label={ariaLabel}
          tabIndex={-1}
          onKeyDown={onPanelKeyDown}
          className={cn(
            'absolute z-30 min-w-full animate-scale-in rounded border border-steel/20 bg-canvas p-1 focus-visible:outline-none',
            side === 'bottom' ? 'top-full mt-1.5' : 'bottom-full mb-1.5',
            align === 'start' ? 'left-0' : 'right-0',
            panelClassName,
          )}
        >
          {children({ close: () => close(true) })}
        </div>
      )}
    </div>
  );
}

interface MenuItemProps {
  icon?: LucideIcon;
  disabled?: boolean;
  /** Right-aligned hint, e.g. "Soon" or "1 workspace per account". */
  hint?: string;
  destructive?: boolean;
  onSelect?: () => void;
  children: ReactNode;
}

/** A row inside a Popover with role="menu". */
export function MenuItem({
  icon: Icon,
  disabled,
  hint,
  destructive,
  onSelect,
  children,
}: MenuItemProps) {
  return (
    <button
      type="button"
      role="menuitem"
      tabIndex={-1}
      aria-disabled={disabled || undefined}
      onClick={disabled ? undefined : onSelect}
      className={cn(
        'flex w-full items-center gap-2.5 rounded px-2.5 py-2 text-left text-sm transition-colors focus-visible:outline-none',
        disabled
          ? 'cursor-default text-steel/60'
          : 'hover:bg-surface focus-visible:bg-surface',
        !disabled && (destructive ? 'text-danger' : 'text-charcoal'),
      )}
    >
      {Icon && <Icon size={16} strokeWidth={2} aria-hidden="true" className="shrink-0" />}
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {hint && <span className="ml-auto shrink-0 text-xs text-steel">{hint}</span>}
    </button>
  );
}
