import { type KeyboardEvent, type ReactNode, useId, useRef } from 'react';
import { cn } from '../../lib/cn';

export interface TabItem {
  id: string;
  label: string;
  /** Small trailing adornment, e.g. a count badge. */
  adornment?: ReactNode;
}

interface TabsProps {
  tabs: TabItem[];
  active: string;
  onChange: (id: string) => void;
  ariaLabel: string;
  className?: string;
}

/**
 * Underline tabs with roving arrow-key focus. Controlled: the parent owns
 * the active id and renders the matching panel (give it
 * `role="tabpanel"` + `id={panelId(activeTabId)}` for the aria wiring).
 */
export function Tabs({ tabs, active, onChange, ariaLabel, className }: TabsProps) {
  const baseId = useId();
  const refs = useRef<Map<string, HTMLButtonElement>>(new Map());

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') return;
    event.preventDefault();
    const index = tabs.findIndex((tab) => tab.id === active);
    const delta = event.key === 'ArrowRight' ? 1 : -1;
    const next = tabs[(index + delta + tabs.length) % tabs.length];
    onChange(next.id);
    refs.current.get(next.id)?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      onKeyDown={onKeyDown}
      className={cn('flex items-center gap-1 border-b border-steel/10', className)}
    >
      {tabs.map((tab) => {
        const selected = tab.id === active;
        return (
          <button
            key={tab.id}
            ref={(node) => {
              if (node) refs.current.set(tab.id, node);
              else refs.current.delete(tab.id);
            }}
            type="button"
            role="tab"
            id={`${baseId}-tab-${tab.id}`}
            aria-selected={selected}
            aria-controls={`${baseId}-panel-${tab.id}`}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(tab.id)}
            className={cn(
              '-mb-px inline-flex items-center gap-1.5 border-b-2 px-3 py-2 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
              selected
                ? 'border-primary font-medium text-charcoal'
                : 'border-transparent text-steel hover:border-steel/30 hover:text-charcoal',
            )}
          >
            {tab.label}
            {tab.adornment}
          </button>
        );
      })}
    </div>
  );
}
