import { useCallback, useEffect, useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import type { Me } from '../../../types/user';
import { useDeleteAccount } from '../hooks/useSettings';

interface DeleteAccountDialogProps {
  open: boolean;
  onClose: () => void;
  me: Me;
}

/**
 * Irreversible: deletes the account AND its workspace (repositories,
 * pipelines, secrets, artifacts, runners — everything cascades). Requires
 * retyping the exact username; the server re-checks it.
 */
export function DeleteAccountDialog({ open, onClose, me }: DeleteAccountDialogProps) {
  const deleteAccount = useDeleteAccount();
  const [confirm, setConfirm] = useState('');

  useEffect(() => {
    if (open) setConfirm('');
  }, [open]);

  const close = useCallback(() => {
    if (deleteAccount.isPending) return;
    setConfirm('');
    onClose();
  }, [deleteAccount.isPending, onClose]);

  const matches = confirm === me.username;

  return (
    <Dialog
      open={open}
      onClose={close}
      title="Delete account"
      description={
        me.workspace
          ? `This permanently deletes your account AND the "${me.workspace.name}" workspace — every repository connection, pipeline, artifact, secret, environment, and runner in it. There is no undo.`
          : 'This permanently deletes your account. There is no undo.'
      }
      className="max-w-lg"
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={close} disabled={deleteAccount.isPending}>
            Cancel
          </Button>
          <Button
            size="sm"
            className="bg-danger text-white hover:bg-danger/80"
            disabled={!matches}
            isLoading={deleteAccount.isPending}
            onClick={() => matches && deleteAccount.mutate(confirm)}
          >
            Delete my account
          </Button>
        </>
      }
    >
      <FormField
        id="delete-account-confirm"
        label={`Type your username (@${me.username}) to confirm`}
      >
        {(aria) => (
          <Input
            {...aria}
            autoFocus
            autoComplete="off"
            spellCheck={false}
            placeholder={me.username}
            className="font-mono"
            value={confirm}
            onChange={(event) => setConfirm(event.target.value)}
          />
        )}
      </FormField>
    </Dialog>
  );
}
