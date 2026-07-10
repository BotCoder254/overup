import { Copy } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '../../../../components/ui/Button';
import { env } from '../../../../lib/env';

interface InstallCommandStepProps {
  token: string;
  labels: string[];
  onNext: () => void;
}

function serverOrigin(): string {
  return env.apiOrigin || window.location.origin;
}

function buildCommand(token: string, labels: string[]): string {
  const labelArg = labels.length > 0 ? labels.join(',') : 'self-hosted';
  return [
    `docker run -d --name overup-runner --restart unless-stopped \\`,
    `  -v /var/run/docker.sock:/var/run/docker.sock \\`,
    `  -e OVERUP_URL=${serverOrigin()} \\`,
    `  -e RUNNER_TOKEN=${token} \\`,
    `  -e RUNNER_JOB_SIGNING_KEY=<same as the control plane> \\`,
    `  -e RUNNER_LABELS=${labelArg} \\`,
    `  -e RUNNER_TOKEN_FILE=/data/runner-token \\`,
    `  -v overup-runner-data:/data \\`,
    `  ghcr.io/overup/runner:latest`,
  ].join('\n');
}

/** Linux/Docker is the only path the reference runner actually ships today. */
export function InstallCommandStep({ token, labels, onNext }: InstallCommandStepProps) {
  const command = buildCommand(token, labels);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command);
      toast.success('Command copied to clipboard.');
    } catch {
      toast.error('Could not access the clipboard.');
    }
  };

  return (
    <div className="mt-4 space-y-4">
      <p className="text-sm leading-relaxed text-steel">
        Run this on the target machine. The token is single-use and expires in one hour — the
        runner exchanges it for a permanent credential on its first connection.
      </p>
      <div className="relative">
        <pre className="overflow-x-auto rounded border border-steel/20 bg-surface p-3.5 font-mono text-xs leading-relaxed text-charcoal">
          {command}
        </pre>
        <button
          type="button"
          onClick={() => void copy()}
          aria-label="Copy install command"
          title="Copy install command"
          className="absolute right-2 top-2 rounded border border-steel/20 bg-canvas p-1.5 text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <Copy size={14} aria-hidden="true" />
        </button>
      </div>
      <div className="flex justify-end">
        <Button size="sm" onClick={onNext}>
          I've run this — continue
        </Button>
      </div>
    </div>
  );
}
