import {
  Boxes,
  Check,
  Copy,
  Cog,
  Coffee,
  FileCode2,
  GitBranch,
  HardDrive,
  Package,
  Rocket,
  Terminal,
  type LucideIcon,
} from 'lucide-react';
import { useState } from 'react';
import { toast } from 'sonner';
import { Badge } from '../../../components/ui/Badge';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import type { Toolchain } from '../../../types/toolchain';

/** A lucide glyph per toolchain key; a generic box for anything unmapped. */
const ICONS: Record<string, LucideIcon> = {
  act: Boxes,
  rust: Cog,
  js: FileCode2,
  go: Rocket,
  dotnet: Package,
  java: Coffee,
  pwsh: Terminal,
  gh: GitBranch,
  full: HardDrive,
};

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
}

export function ToolchainCard({ toolchain }: ToolchainCardProps) {
  const Icon = ICONS[toolchain.key] ?? Boxes;
  const snippet = `container: ${toolchain.key}`;
  const image = useCopy();
  const usage = useCopy();

  return (
    <Card>
      <CardHeader>
        <div className="flex items-start justify-between gap-2">
          <div className="flex items-center gap-2.5">
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded bg-primary/10 text-primary">
              <Icon size={18} aria-hidden="true" />
            </span>
            <div className="min-w-0">
              <h2 className="truncate text-sm font-semibold text-charcoal">{toolchain.label}</h2>
              <p className="truncate text-xs text-steel">{toolchain.language}</p>
            </div>
          </div>
          <div className="flex shrink-0 flex-wrap justify-end gap-1">
            {toolchain.prewarmed && <Badge variant="success">Prewarmed</Badge>}
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
      </CardBody>
    </Card>
  );
}
