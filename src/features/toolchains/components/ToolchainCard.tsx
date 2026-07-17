import { Check, Copy, Download, RefreshCw, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { toast } from 'sonner';
import { ToolchainLogo } from '../../../components/brand/toolchains/ToolchainLogo';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { Dialog } from '../../../components/ui/Dialog';
import type { Toolchain } from '../../../types/toolchain';
import { useInstallToolchain, useUninstallToolchain } from '../hooks/useToolchains';

/** Copy `text`, flashing a check on the button and a toast. */
function useCopy() {
  const [copied, setCopied] = useState(false);
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
      toast.success('Copied to clipboard.');
    } catch {
      toast.error('Could not access the clipboard.');
    }
  };
  return { copied, copy };
}

interface ToolchainCardProps {
  toolchain: Toolchain;
  /** Whether install/uninstall is possible (hosted provisioner reachable). */
  installSupported: boolean;
}

/** The status badge in the header — install state wins over the prewarm hint. */
function StatusBadge({ toolchain }: { toolchain: Toolchain }) {
  switch (toolchain.installStatus) {
    case 'installed':
      return <Badge variant="success">Installed</Badge>;
    case 'pending':
      return <Badge variant="info">Installing…</Badge>;
    case 'failed':
      return <Badge variant="danger">Failed</Badge>;
    default:
      return toolchain.prewarmed ? <Badge variant="success">Prewarmed</Badge> : null;
  }
}

export function ToolchainCard({ toolchain, installSupported }: ToolchainCardProps) {
  const snippet = `container: ${toolchain.key}`;
  const image = useCopy();
  const usage = useCopy();
  const install = useInstallToolchain();
  const uninstall = useUninstallToolchain();
  const [confirmUninstall, setConfirmUninstall] = useState(false);

  const status = toolchain.installStatus;

  return (
    <Card>
      <CardHeader>
        <div className="flex items-start justify-between gap-2">
          <div className="flex items-center gap-2.5">
            {/* Official brand logo, transparent background (no box). */}
            <ToolchainLogo toolchainKey={toolchain.key} className="h-8 w-8 shrink-0" />
            <div className="min-w-0">
              <h2 className="truncate text-sm font-semibold text-charcoal">{toolchain.label}</h2>
              <p className="truncate text-xs text-steel">{toolchain.language}</p>
            </div>
          </div>
          <div className="flex shrink-0 flex-wrap justify-end gap-1">
            <StatusBadge toolchain={toolchain} />
            {toolchain.large && <Badge variant="danger">Large</Badge>}
          </div>
        </div>
      </CardHeader>
      <CardBody>
        <p className="text-xs leading-relaxed text-steel">{toolchain.description}</p>

        {/* Image reference with copy. */}
        <div className="mt-3">
          <div className="mb-1 text-xs font-medium uppercase tracking-wider text-steel">Image</div>
          <div className="flex items-center gap-1.5 rounded border border-steel/20 bg-surface px-2.5 py-1.5">
            <code className="min-w-0 flex-1 truncate font-mono text-xs text-charcoal">
              {toolchain.imageLatest}
            </code>
            <button
              type="button"
              onClick={() => void image.copy(toolchain.imageLatest)}
              aria-label={`Copy image reference for ${toolchain.label}`}
              title="Copy image reference"
              className="shrink-0 rounded p-1 text-steel transition-colors hover:bg-canvas hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              {image.copied ? (
                <Check size={13} aria-hidden="true" className="text-primary" />
              ) : (
                <Copy size={13} aria-hidden="true" />
              )}
            </button>
          </div>
        </div>

        {/* Tools chips. */}
        {toolchain.tools.length > 0 && (
          <div className="mt-3">
            <div className="mb-1 text-xs font-medium uppercase tracking-wider text-steel">
              Includes
            </div>
            <div className="flex flex-wrap gap-1">
              {toolchain.tools.map((tool) => (
                <Badge key={tool} variant="outline">
                  {tool}
                </Badge>
              ))}
            </div>
          </div>
        )}

        {/* Usage snippet with copy. */}
        <div className="mt-3">
          <div className="mb-1 text-xs font-medium uppercase tracking-wider text-steel">
            Use in a job
          </div>
          <div className="flex items-center gap-1.5 rounded border border-steel/20 bg-surface px-2.5 py-1.5">
            <code className="min-w-0 flex-1 truncate font-mono text-xs text-charcoal">
              {snippet}
            </code>
            <button
              type="button"
              onClick={() => void usage.copy(snippet)}
              aria-label={`Copy usage snippet for ${toolchain.label}`}
              title="Copy usage snippet"
              className="shrink-0 rounded p-1 text-steel transition-colors hover:bg-canvas hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              {usage.copied ? (
                <Check size={13} aria-hidden="true" className="text-primary" />
              ) : (
                <Copy size={13} aria-hidden="true" />
              )}
            </button>
          </div>
        </div>

        {/* Install / Uninstall action row. */}
        <div className="mt-4 border-t border-steel/10 pt-3">
          {!installSupported ? (
            <div>
              <Button size="sm" variant="secondary" disabled title="Requires hosted runners">
                <Download size={14} aria-hidden="true" />
                Install
              </Button>
              <p className="mt-1.5 text-xs text-steel">
                Requires hosted runners (RUNNER_PROVISIONER=docker).
              </p>
            </div>
          ) : status === 'pending' ? (
            <Button size="sm" variant="secondary" isLoading disabled>
              Installing…
            </Button>
          ) : status === 'installed' ? (
            <Button
              size="sm"
              variant="secondary"
              isLoading={uninstall.isPending}
              onClick={() => setConfirmUninstall(true)}
            >
              <Trash2 size={14} aria-hidden="true" />
              Uninstall
            </Button>
          ) : status === 'failed' ? (
            <div>
              <Button
                size="sm"
                variant="secondary"
                isLoading={install.isPending}
                onClick={() => install.mutate(toolchain.key)}
              >
                <RefreshCw size={14} aria-hidden="true" />
                Retry install
              </Button>
              <p className="mt-1.5 text-xs text-danger">Install failed — try again.</p>
            </div>
          ) : (
            <Button
              size="sm"
              isLoading={install.isPending}
              onClick={() => install.mutate(toolchain.key)}
            >
              <Download size={14} aria-hidden="true" />
              Install
            </Button>
          )}
        </div>
      </CardBody>

      <Dialog
        open={confirmUninstall}
        onClose={() => setConfirmUninstall(false)}
        title={`Uninstall ${toolchain.label}?`}
        description={`This removes ${toolchain.imageLatest} from the runner daemon and stops warming it. Jobs can still pull it on demand, and you can reinstall anytime.`}
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setConfirmUninstall(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              isLoading={uninstall.isPending}
              onClick={() =>
                uninstall.mutate(toolchain.key, { onSuccess: () => setConfirmUninstall(false) })
              }
            >
              Uninstall
            </Button>
          </>
        }
      />
    </Card>
  );
}
