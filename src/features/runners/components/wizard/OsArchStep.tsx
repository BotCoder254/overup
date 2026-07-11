import { HardDrive, Server } from 'lucide-react';
import { cn } from '../../../../lib/cn';
import { Button } from '../../../../components/ui/Button';
import { FormField } from '../../../../components/ui/FormField';
import { Input } from '../../../../components/ui/Input';

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
  /** Whether this deployment can provision hosted runners itself. */
  hostedAvailable: boolean;
  onNext: () => void;
  isLoading?: boolean;
}

export function OsArchStep({
  name,
  onNameChange,
  labels,
  onLabelsChange,
  os,
  onOsChange,
  mode,
  onModeChange,
  hostedAvailable,
  onNext,
  isLoading,
}: OsArchStepProps) {
  const selfHosted = mode === 'self-hosted';
  return (
    <div className="mt-4 space-y-5">
      {hostedAvailable && (
        <div className="space-y-2">
          <span className="text-sm font-medium text-charcoal">Where should it run?</span>
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <button
              type="button"
              onClick={() => onModeChange('hosted')}
              className={cn(
                'rounded border px-3 py-2.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                mode === 'hosted'
                  ? 'border-primary bg-primary/5'
                  : 'border-steel/20 hover:bg-surface',
              )}
            >
              <span className="flex items-center gap-2 text-sm font-medium text-charcoal">
                <Server size={15} aria-hidden="true" className="shrink-0 text-primary" />
                Hosted on this server
              </span>
              <span className="mt-1 block text-xs leading-relaxed text-steel">
                Provisioned automatically — create and wait, nothing to install.
              </span>
            </button>
            <button
              type="button"
              onClick={() => onModeChange('self-hosted')}
              className={cn(
                'rounded border px-3 py-2.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                selfHosted ? 'border-primary bg-primary/5' : 'border-steel/20 hover:bg-surface',
              )}
            >
              <span className="flex items-center gap-2 text-sm font-medium text-charcoal">
                <HardDrive size={15} aria-hidden="true" className="shrink-0 text-primary" />
                Self-hosted machine
              </span>
              <span className="mt-1 block text-xs leading-relaxed text-steel">
                Run one copy-paste command on your own machine.
              </span>
            </button>
          </div>
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

      <div className="flex justify-end">
        <Button
          size="sm"
          isLoading={isLoading}
          disabled={!name.trim() || (selfHosted && !os)}
          onClick={onNext}
        >
          {selfHosted ? 'Continue' : 'Create runner'}
        </Button>
      </div>
    </div>
  );
}
