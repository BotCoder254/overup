import { useEffect, useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { Spinner } from '../../../components/ui/Spinner';
import type { RetentionPolicy } from '../../../types/artifact';
import { useRetentionPolicies, useSaveRetentionPolicies } from '../hooks/useArtifactsCatalog';
import { ARTIFACT_KIND_LABELS } from './ArtifactFilters';

/** 'default' first, then every classifier kind — the editable row order. */
const POLICY_KINDS: RetentionPolicy['kind'][] = [
  'default',
  'package',
  'report',
  'docs',
  'archive',
  'binary',
  'image',
  'log',
  'other',
];

interface RowState {
  enabled: boolean;
  days: string;
}

interface RetentionPolicyDialogProps {
  open: boolean;
  onClose: () => void;
}

/**
 * Per-kind retention policy editor (the GitHub model: a workspace default
 * plus per-kind overrides, 1–400 days). Policies apply to future uploads;
 * existing artifacts keep the expiry stamped at upload time.
 */
export function RetentionPolicyDialog({ open, onClose }: RetentionPolicyDialogProps) {
  const policies = useRetentionPolicies();
  const save = useSaveRetentionPolicies();
  const [rows, setRows] = useState<Record<string, RowState>>({});

  // Seed the form from the server whenever the dialog opens.
  useEffect(() => {
    if (!open || !policies.data) return;
    const next: Record<string, RowState> = {};
    for (const kind of POLICY_KINDS) {
      const existing = policies.data.policies.find((policy) => policy.kind === kind);
      next[kind] = existing
        ? { enabled: true, days: String(existing.retentionDays) }
        : { enabled: false, days: '' };
    }
    setRows(next);
  }, [open, policies.data]);

  const invalid = POLICY_KINDS.some((kind) => {
    const row = rows[kind];
    if (!row?.enabled) return false;
    const days = Number(row.days);
    return !Number.isInteger(days) || days < 1 || days > 400;
  });

  const onSave = () => {
    const payload: RetentionPolicy[] = POLICY_KINDS.filter((kind) => rows[kind]?.enabled).map(
      (kind) => ({ kind, retentionDays: Number(rows[kind].days) }),
    );
    save.mutate(payload, { onSuccess: onClose });
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Artifact retention"
      description={
        policies.data
          ? `Days each kind stays stored (1–400). Unset kinds fall back to the workspace default, then the server default of ${policies.data.globalDefaultDays} days. Applies to future uploads.`
          : 'Days each kind stays stored (1–400). Applies to future uploads.'
      }
      className="max-w-lg"
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" isLoading={save.isPending} disabled={invalid} onClick={onSave}>
            Save
          </Button>
        </>
      }
    >
      {policies.isLoading ? (
        <div className="mt-4 flex justify-center py-6">
          <Spinner className="h-5 w-5 text-steel" />
        </div>
      ) : (
        <div className="mt-4 max-h-80 space-y-2 overflow-y-auto pr-1">
          {POLICY_KINDS.map((kind) => {
            const row = rows[kind] ?? { enabled: false, days: '' };
            const label = kind === 'default' ? 'Workspace default' : ARTIFACT_KIND_LABELS[kind];
            return (
              <div key={kind} className="flex items-center justify-between gap-3">
                <label className="flex min-w-0 items-center gap-2 text-sm text-charcoal">
                  <input
                    type="checkbox"
                    className="h-4 w-4 rounded border-steel/40 text-primary focus-visible:ring-2 focus-visible:ring-primary"
                    checked={row.enabled}
                    onChange={(event) =>
                      setRows((current) => ({
                        ...current,
                        [kind]: {
                          enabled: event.target.checked,
                          days: current[kind]?.days || '30',
                        },
                      }))
                    }
                  />
                  <span className={kind === 'default' ? 'font-medium' : undefined}>{label}</span>
                </label>
                <div className="flex items-center gap-1.5">
                  <input
                    type="number"
                    min={1}
                    max={400}
                    disabled={!row.enabled}
                    aria-label={`${label} retention days`}
                    className="h-8 w-20 rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal disabled:bg-surface disabled:text-steel focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                    value={row.days}
                    onChange={(event) =>
                      setRows((current) => ({
                        ...current,
                        [kind]: { ...current[kind], days: event.target.value },
                      }))
                    }
                  />
                  <span className="text-xs text-steel">days</span>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </Dialog>
  );
}
