import { Check, X } from 'lucide-react';
import { Spinner } from '../../../components/ui/Spinner';
import type { WorkspaceAvailabilityState } from '../hooks/useWorkspaceAvailability';

/**
 * Inline availability feedback for the workspace name field — a spinner
 * while checking, a green check when the derived slug is free, and a steel
 * note ("will be saved as …") when it's taken. Advisory only; it never
 * gates the form. Shared by onboarding and Settings › Workspace.
 */
export function AvailabilityIndicator({ state }: { state: WorkspaceAvailabilityState }) {
  if (state.status === 'idle' || state.status === 'invalid') return null;

  if (state.status === 'checking') {
    return (
      <p className="flex items-center gap-1.5 text-xs text-steel" aria-live="polite">
        <Spinner className="h-3.5 w-3.5 text-steel" />
        Checking availability…
      </p>
    );
  }

  if (state.status === 'available') {
    return (
      <p className="flex items-center gap-1.5 text-xs text-primary" aria-live="polite">
        <Check size={14} aria-hidden="true" />
        <span className="font-mono">/w/{state.slug}</span> is available
      </p>
    );
  }

  // taken
  return (
    <p className="flex flex-wrap items-center gap-1.5 text-xs text-steel" aria-live="polite">
      <X size={14} aria-hidden="true" className="text-steel" />
      <span className="font-mono">/w/{state.slug}</span> is taken
      {state.adjustedSlug && (
        <>
          — will be saved as <span className="font-mono text-charcoal">/w/{state.adjustedSlug}</span>
        </>
      )}
    </p>
  );
}
