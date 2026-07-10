import type { ReactNode } from 'react';

/** Shared definition-list building blocks for the inspector panels. */
export function PanelSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mb-4">
      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-steel">
        {title}
      </h3>
      {children}
    </section>
  );
}

export function FieldList({ children }: { children: ReactNode }) {
  return <dl className="space-y-1.5">{children}</dl>;
}

export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline gap-2 text-sm">
      <dt className="w-32 shrink-0 text-xs text-steel">{label}</dt>
      <dd className="min-w-0 break-all text-charcoal">{children}</dd>
    </div>
  );
}
