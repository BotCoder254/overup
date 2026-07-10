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

interface OsArchStepProps {
  name: string;
  onNameChange: (value: string) => void;
  labels: string;
  onLabelsChange: (value: string) => void;
  os: string;
  onOsChange: (id: string) => void;
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
  onNext,
  isLoading,
}: OsArchStepProps) {
  return (
    <div className="mt-4 space-y-5">
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

      <div className="flex justify-end">
        <Button size="sm" isLoading={isLoading} disabled={!name.trim() || !os} onClick={onNext}>
          Continue
        </Button>
      </div>
    </div>
  );
}
