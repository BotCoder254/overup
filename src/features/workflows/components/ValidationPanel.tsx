import { AlertTriangle, CheckCircle2, XCircle } from 'lucide-react';
import { Spinner } from '../../../components/ui/Spinner';
import type { Diagnostic } from '../../../types/workflow';

interface ValidationPanelProps {
  diagnostics: Diagnostic[];
  validating: boolean;
  onSelectLine?: (line: number) => void;
}

/** Diagnostics list under the editor, mirrored as Monaco markers. */
export function ValidationPanel({ diagnostics, validating, onSelectLine }: ValidationPanelProps) {
  if (validating && diagnostics.length === 0) {
    return (
      <div className="flex items-center gap-2 px-3 py-2 text-xs text-steel">
        <Spinner className="h-3 w-3" />
        Validating…
      </div>
    );
  }
  if (diagnostics.length === 0) {
    return (
      <div className="flex items-center gap-2 px-3 py-2 text-xs text-steel">
        <CheckCircle2 size={14} className="text-primary" aria-hidden="true" />
        No problems found — the workflow is structurally valid.
      </div>
    );
  }
  return (
    <ul className="max-h-40 divide-y divide-steel/10 overflow-y-auto text-xs" aria-label="Validation problems">
      {diagnostics.map((diagnostic, index) => (
        <li key={`${diagnostic.message}-${index}`}>
          <button
            type="button"
            disabled={!diagnostic.line || !onSelectLine}
            onClick={() => diagnostic.line && onSelectLine?.(diagnostic.line)}
            className="flex w-full items-start gap-2 px-3 py-2 text-left transition-colors enabled:hover:bg-surface disabled:cursor-default"
          >
            {diagnostic.severity === 'error' ? (
              <XCircle size={13} className="mt-0.5 shrink-0 text-danger" aria-hidden="true" />
            ) : (
              <AlertTriangle size={13} className="mt-0.5 shrink-0 text-link" aria-hidden="true" />
            )}
            <span className="min-w-0">
              <span className="text-charcoal">{diagnostic.message}</span>
              {(diagnostic.path || diagnostic.line) && (
                <span className="ml-1.5 font-mono text-steel">
                  {diagnostic.path}
                  {diagnostic.line ? ` (line ${diagnostic.line})` : ''}
                </span>
              )}
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}
