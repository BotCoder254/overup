import { Copy } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';

interface TokenRevealDialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description: string;
  token: string | null;
}

/** Shows a registration/rotation token exactly once, with a copy action. */
export function TokenRevealDialog({ open, onClose, title, description, token }: TokenRevealDialogProps) {
  const copy = async () => {
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      toast.success('Token copied to clipboard.');
    } catch {
      toast.error('Could not access the clipboard.');
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={title}
      description={description}
      className="max-w-lg"
      footer={
        <Button size="sm" onClick={onClose}>
          Done
        </Button>
      }
    >
      <div className="mt-4 flex items-center gap-2">
        <code className="flex-1 truncate rounded border border-steel/20 bg-surface px-3 py-2.5 font-mono text-xs text-charcoal">
          {token}
        </code>
        <button
          type="button"
          onClick={() => void copy()}
          aria-label="Copy token"
          title="Copy token"
          className="shrink-0 rounded border border-steel/20 p-2.5 text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <Copy size={14} aria-hidden="true" />
        </button>
      </div>
    </Dialog>
  );
}
