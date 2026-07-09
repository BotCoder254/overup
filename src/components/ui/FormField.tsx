import type { ReactNode } from 'react';

interface FieldAria {
  id: string;
  'aria-invalid'?: boolean;
  'aria-describedby'?: string;
}

interface FormFieldProps {
  id: string;
  label: string;
  optional?: boolean;
  hint?: string;
  error?: string;
  /** Render prop so any control receives the correct label/error wiring. */
  children: (aria: FieldAria) => ReactNode;
}

/**
 * Accessible field wrapper: associates the label, exposes validation state
 * via aria-invalid/aria-describedby, and announces errors to assistive
 * technology through role="alert".
 */
export function FormField({ id, label, optional, hint, error, children }: FormFieldProps) {
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [errorId, hintId].filter(Boolean).join(' ') || undefined;

  return (
    <div className="space-y-2">
      <div className="flex items-baseline justify-between">
        <label htmlFor={id} className="text-sm font-medium text-charcoal">
          {label}
        </label>
        {optional && <span className="text-xs text-steel">Optional</span>}
      </div>
      {children({
        id,
        'aria-invalid': error ? true : undefined,
        'aria-describedby': describedBy,
      })}
      {hint && !error && (
        <p id={hintId} className="text-sm text-steel">
          {hint}
        </p>
      )}
      {error && (
        <p id={errorId} role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
