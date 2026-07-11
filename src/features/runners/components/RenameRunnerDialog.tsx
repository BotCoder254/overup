import { useEffect, useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import type { Runner } from '../../../types/runner';
import { useUpdateRunner } from '../hooks/useRunners';

interface RenameRunnerDialogProps {
  runner: Runner | null;
  onClose: () => void;
}

function parseLabels(value: string): string[] {
  return Array.from(
    new Set(
      value
        .split(',')
        .map((label) => label.trim())
        .filter(Boolean),
    ),
  );
}

export function RenameRunnerDialog({ runner, onClose }: RenameRunnerDialogProps) {
  const [name, setName] = useState('');
  const [labels, setLabels] = useState('');
  const update = useUpdateRunner();

  useEffect(() => {
    if (runner) {
      setName(runner.name);
      setLabels(runner.labels.join(', '));
    }
  }, [runner]);

  if (!runner) return null;

  const onSubmit = () => {
    update.mutate(
      { runnerId: runner.id, name: name.trim(), labels: parseLabels(labels) },
      { onSuccess: onClose },
    );
  };

  return (
    <Dialog
      open={runner !== null}
      onClose={onClose}
      title={`Rename ${runner.name}`}
      description="Update the display name and labels used to match workflow runs-on values."
      className="max-w-lg"
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" isLoading={update.isPending} disabled={!name.trim()} onClick={onSubmit}>
            Save
          </Button>
        </>
      }
    >
      <div className="mt-4 space-y-4">
        <FormField id="runner-rename-name" label="Name">
          {(aria) => (
            <Input {...aria} autoFocus value={name} onChange={(event) => setName(event.target.value)} />
          )}
        </FormField>
        <FormField
          id="runner-rename-labels"
          label="Labels"
          optional
          hint="Comma-separated, e.g. self-hosted, linux, x64"
        >
          {(aria) => (
            <Input {...aria} value={labels} onChange={(event) => setLabels(event.target.value)} />
          )}
        </FormField>
      </div>
    </Dialog>
  );
}
