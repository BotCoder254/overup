import { ChevronDown, ChevronRight, HardDrive, Server } from 'lucide-react';
import { cn } from '../../../../lib/cn';
import { Button } from '../../../../components/ui/Button';
import { FormField } from '../../../../components/ui/FormField';
import { Input } from '../../../../components/ui/Input';
import type { RunnerResourceProfile } from '../../../../types/runner';
import { HOSTED_QUOTA_COPY } from '../../lib/provisionCopy';

interface OsOption {
  id: string;
  label: string;
  available: boolean;
}

const OS_OPTIONS: OsOption[] = [
  { id: 'linux-x64', label: 'Linux · x64', available: true },
  { id: 'linux-arm64', label: 'Linux · arm64', available: true },
  { id: 'windows-x64', label: 'Windows · x64', available: false },
  { id: 'macos-arm64', label: 'macOS · arm64', available: false },
];

interface ProfileOption {
  id: RunnerResourceProfile;
  label: string;
  detail: string;
}

/** Mirrors the server-side presets in backend services/runner_profiles.rs. */
const PROFILE_OPTIONS: ProfileOption[] = [
  { id: 'small', label: 'Small', detail: '1 CPU / 1 GiB' },
  { id: 'standard', label: 'Standard', detail: '2 CPU / 2 GiB' },
  { id: 'large', label: 'Large', detail: '4 CPU / 4 GiB' },
];

export type RunnerMode = 'hosted' | 'self-hosted';

interface OsArchStepProps {
  name: string;
  onNameChange: (value: string) => void;
  labels: string;
  onLabelsChange: (value: string) => void;
  os: string;
  onOsChange: (id: string) => void;
  mode: RunnerMode;
  onModeChange: (mode: RunnerMode) => void;
  profile: RunnerResourceProfile;
  onProfileChange: (profile: RunnerResourceProfile) => void;
  instances: number;
  onInstancesChange: (instances: number) => void;
  /** Whether this deployment can provision hosted runners itself. */
  hostedAvailable: boolean;
  /** Remaining hosted-runner quota; null while unknown/no provisioner. */
  hostedRemaining: number | null;
  onNext: () => void;
  isLoading?: boolean;
}

/**
 * Details step. Hosted provisioning is the primary, one-click path whenever
 * the deployment supports it; the self-hosted install flow lives behind an
 * "Advanced" disclosure. Without a provisioner (or with the quota full) the
 * form is the classic self-hosted one.
 */
export function OsArchStep({
  name,
  onNameChange,
  labels,
  onLabelsChange,
  os,
  onOsChange,
  mode,
  onModeChange,
  profile,
  onProfileChange,
  instances,
  onInstancesChange,
  hostedAvailable,
  hostedRemaining,
  onNext,
  isLoading,
}: OsArchStepProps) {
  const hostedQuotaReached = hostedRemaining === 0;
  const hostedUsable = hostedAvailable && !hostedQuotaReached;
  const hosted = hostedUsable && mode === 'hosted';
  const selfHosted = !hosted;
  const maxInstances = Math.max(1, hostedRemaining ?? 1);

  const clampInstances = (raw: number) => {
    if (Number.isNaN(raw)) return 1;
    return Math.min(Math.max(Math.trunc(raw), 1), maxInstances);
  };

  return (
    <div className="mt-4 space-y-5">
      {hosted && (
        <div className="flex items-start gap-2 rounded border border-steel/20 bg-surface px-3 py-2.5">
          <Server size={15} aria-hidden="true" className="mt-0.5 shrink-0 text-primary" />
          <p className="text-xs leading-relaxed text-steel">
            <span className="font-medium text-charcoal">Hosted on this server.</span> The
            platform provisions and manages the runner automatically — nothing to install,
            no tokens, no configuration.
          </p>
        </div>
      )}
      {hostedAvailable && hostedQuotaReached && (
        <div className="flex items-start gap-2 rounded border border-steel/20 bg-surface px-3 py-2.5">
          <Server size={15} aria-hidden="true" className="mt-0.5 shrink-0 text-steel" />
          <p className="text-xs leading-relaxed text-steel">{HOSTED_QUOTA_COPY}</p>
        </div>
      )}

      <FormField id="wizard-runner-name" label="Name">
        {(aria) => (
          <Input
            {...aria}
            autoFocus
            placeholder="build-box-01"
            value={name}
            onChange={(event) => onNameChange(event.target.value)}
          />
        )}
      </FormField>
      <FormField
        id="wizard-runner-labels"
        label="Labels"
        optional
        hint="Comma-separated, e.g. self-hosted, linux, x64"
      >
        {(aria) => (
          <Input
            {...aria}
            placeholder="self-hosted, linux, x64"
            value={labels}
            onChange={(event) => onLabelsChange(event.target.value)}
          />
        )}
      </FormField>

      {hosted && (
        <>
          <div className="space-y-2">
            <span className="text-sm font-medium text-charcoal">Size</span>
            <div className="grid grid-cols-3 gap-2">
              {PROFILE_OPTIONS.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => onProfileChange(option.id)}
                  className={cn(
                    'rounded border px-3 py-2.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                    profile === option.id
                      ? 'border-primary bg-primary/5'
                      : 'border-steel/20 hover:bg-surface',
                  )}
                >
                  <span className="block text-sm font-medium text-charcoal">{option.label}</span>
                  <span className="mt-0.5 block text-xs text-steel">{option.detail}</span>
                </button>
              ))}
            </div>
          </div>

          <FormField
            id="wizard-runner-instances"
            label="Instances"
            hint={
              hostedRemaining !== null
                ? `${hostedRemaining} hosted slot${hostedRemaining === 1 ? '' : 's'} remaining. Each instance runs one job at a time.`
                : 'Each instance runs one job at a time.'
            }
          >
            {(aria) => (
              <Input
                {...aria}
                type="number"
                min={1}
                max={maxInstances}
                value={instances}
                onChange={(event) => onInstancesChange(clampInstances(event.target.valueAsNumber))}
              />
            )}
          </FormField>
        </>
      )}

      {selfHosted && (
        <div className="space-y-2">
          <span className="text-sm font-medium text-charcoal">Operating system</span>
          <div className="grid grid-cols-2 gap-2">
            {OS_OPTIONS.map((option) => (
              <button
                key={option.id}
                type="button"
                disabled={!option.available}
                onClick={() => onOsChange(option.id)}
                className={cn(
                  'rounded border px-3 py-2.5 text-left text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                  !option.available && 'cursor-not-allowed border-steel/10 text-steel/50',
                  option.available &&
                    (os === option.id
                      ? 'border-primary bg-primary/5 text-primary'
                      : 'border-steel/20 text-charcoal hover:bg-surface'),
                )}
              >
                {option.label}
                {!option.available && <span className="ml-1.5 text-xs">Soon</span>}
              </button>
            ))}
          </div>
        </div>
      )}

      {hostedUsable && (
        <button
          type="button"
          onClick={() => onModeChange(hosted ? 'self-hosted' : 'hosted')}
          className="flex items-center gap-1.5 text-xs font-medium text-steel transition-colors hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          {hosted ? (
            <ChevronRight size={13} aria-hidden="true" />
          ) : (
            <ChevronDown size={13} aria-hidden="true" />
          )}
          {hosted ? (
            <>
              <HardDrive size={13} aria-hidden="true" />
              Advanced: run on your own machine
            </>
          ) : (
            <>
              <Server size={13} aria-hidden="true" />
              Back to a hosted runner
            </>
          )}
        </button>
      )}

      <div className="flex justify-end">
        <Button
          size="sm"
          isLoading={isLoading}
          disabled={!name.trim() || (selfHosted && !os)}
          onClick={onNext}
        >
          {selfHosted
            ? 'Continue'
            : instances > 1
              ? `Create ${instances} runners`
              : 'Create runner'}
        </Button>
      </div>
    </div>
  );
}
