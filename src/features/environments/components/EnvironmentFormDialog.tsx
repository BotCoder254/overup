import { useCallback, useEffect, useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import { Textarea } from '../../../components/ui/Textarea';
import type { Environment } from '../../../types/environment';
import { useCreateEnvironment, useUpdateEnvironment } from '../hooks/useEnvironments';

/**
 * Client-side mirror of the server's validation (the server remains the
 * authority): slug-safe names, 100 chars max.
 */
const NAME_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

function nameError(name: string): string | undefined {
  if (!name) return undefined;
  if (name.length > 100) return 'At most 100 characters.';
  if (!NAME_PATTERN.test(name)) {
    return "Letters, digits, '.', '_', and '-' only, starting with a letter or digit.";
  }
  return undefined;
}

interface EnvironmentFormDialogProps {
  open: boolean;
  onClose: () => void;
  /** When set, the dialog edits this environment instead of creating. */
  editTarget?: Environment | null;
}

/**
 * Create or edit an environment. Workflow YAML binds by NAME and resolves
 * live at dispatch, so the edit variant spells out that renaming changes
 * which jobs pick up this environment's secrets going forward.
 */
export function EnvironmentFormDialog({ open, onClose, editTarget }: EnvironmentFormDialogProps) {
  const create = useCreateEnvironment();
  const update = useUpdateEnvironment();
  const editing = Boolean(editTarget);

  const [name, setName] = useState('');
  const [description, setDescription] = useState('');

  // Seed the fields from the edit target whenever the dialog opens.
  useEffect(() => {
    if (open) {
      setName(editTarget?.name ?? '');
      setDescription(editTarget?.description ?? '');
    }
  }, [open, editTarget]);

  const close = useCallback(() => {
    setName('');
    setDescription('');
    onClose();
  }, [onClose]);

  const nameProblem = nameError(name);
  const invalid = Boolean(nameProblem) || !name || description.length > 500;
  const pending = create.isPending || update.isPending;

  const submit = () => {
    if (invalid || pending) return;
    if (editing && editTarget) {
      update.mutate(
        {
          environmentId: editTarget.id,
          name: name !== editTarget.name ? name : undefined,
          description: description.trim() || undefined,
        },
        { onSuccess: close },
      );
      return;
    }
    create.mutate(
      { name, description: description.trim() || undefined },
      { onSuccess: close },
    );
  };

  return (
    <Dialog
      open={open}
      onClose={close}
      title={editing ? `Edit ${editTarget?.name}` : 'New environment'}
      description={
        editing
          ? 'Workflow jobs bind to environments by name at dispatch time — renaming changes which jobs receive this environment’s secrets from the next run onward.'
          : 'A named secrets scope for deployments. Reference it from workflow YAML with `environment: <name>` — its secrets are injected with the highest precedence.'
      }
      className="max-w-xl"
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button size="sm" disabled={invalid} isLoading={pending} onClick={submit}>
            {editing ? 'Save changes' : 'Create environment'}
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <FormField
          id="environment-name"
          label="Name"
          hint="Matched case-insensitively against workflow `environment:` values."
          error={nameProblem}
        >
          {(aria) => (
            <Input
              {...aria}
              autoFocus
              autoComplete="off"
              spellCheck={false}
              placeholder="production"
              maxLength={100}
              className="font-mono"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          )}
        </FormField>

        <FormField
          id="environment-description"
          label="Description"
          optional
          hint="What deploys here and who owns it."
          error={description.length > 500 ? 'At most 500 characters.' : undefined}
        >
          {(aria) => (
            <Textarea
              {...aria}
              rows={2}
              maxLength={600}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          )}
        </FormField>
      </div>
    </Dialog>
  );
}
