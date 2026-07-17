import { Boxes, Info } from 'lucide-react';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { ToolchainCard } from '../components/ToolchainCard';
import { ToolchainsSummaryStrip } from '../components/ToolchainsSummaryStrip';
import { useToolchains } from '../hooks/useToolchains';

/**
 * Read-only catalog of the language-toolchain images overup understands.
 * Authors select one from a job with `container: <key>` (e.g. `container: rust`);
 * overup expands it to the matching catthehacker image at pipeline-creation time.
 */
export function ToolchainsPage() {
  const query = useToolchains();
  const toolchains = query.data?.toolchains ?? [];

  return (
    <>
      <PageHeader
        title="Toolchains"
        description="Pre-built language images your jobs can run against. Add `container: <name>` to a job and overup resolves it to the matching image."
      />

      <ToolchainsSummaryStrip
        data={query.data}
        loading={query.isLoading}
        error={query.isError}
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0">
          {query.isLoading ? (
            <div className="flex min-h-[40vh] items-center justify-center">
              <Spinner className="h-6 w-6 text-steel" />
            </div>
          ) : toolchains.length === 0 ? (
            <EmptyState
              icon={Boxes}
              title="No toolchains available"
              description="The toolchain catalog could not be loaded."
            />
          ) : (
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              {toolchains.map((toolchain) => (
                <ToolchainCard
                  key={toolchain.key}
                  toolchain={toolchain}
                  installSupported={query.data?.installSupported ?? false}
                />
              ))}
            </div>
          )}
        </div>

        <div className="min-w-0 space-y-4">
          <Card>
            <CardHeader>
              <h2 className="text-sm font-semibold text-charcoal">How toolchains work</h2>
            </CardHeader>
            <CardBody>
              <ul className="space-y-3 text-xs leading-relaxed text-steel">
                <li className="flex items-start gap-2">
                  <Info size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-primary" />
                  <span>
                    Add{' '}
                    <code className="font-mono text-charcoal">container: rust</code> to a job to run
                    it in the Rust toolchain image. The full reference (
                    <code className="font-mono text-charcoal">catthehacker/ubuntu:rust-latest</code>
                    ) also works.
                  </span>
                </li>
                <li className="flex items-start gap-2">
                  <Info size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-primary" />
                  <span>
                    A toolchain pairs with the job&apos;s Ubuntu version — pair{' '}
                    <code className="font-mono text-charcoal">container: rust</code> with{' '}
                    <code className="font-mono text-charcoal">runs-on: ubuntu-22.04</code> to get the
                    22.04 flavor.
                  </span>
                </li>
                <li className="flex items-start gap-2">
                  <Info size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-primary" />
                  <span>
                    Prewarmed images are pulled onto runners ahead of time via{' '}
                    <code className="font-mono text-charcoal">RUNNER_PREPULL_IMAGES</code>; others
                    pull on first use.
                  </span>
                </li>
                <li className="flex items-start gap-2">
                  <Info size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-primary" />
                  <span>
                    <strong className="font-medium text-charcoal">Install</strong> a toolchain to
                    pull it onto the runner daemon now and keep it warmed — no env editing.{' '}
                    <strong className="font-medium text-charcoal">Uninstall</strong> removes the
                    image and stops warming it. Requires hosted runners (
                    <code className="font-mono text-charcoal">RUNNER_PROVISIONER=docker</code>).
                  </span>
                </li>
                {query.data?.allowlistEnabled && (
                  <li className="flex items-start gap-2">
                    <Info size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-danger" />
                    <span>
                      An image allow-list is active — a{' '}
                      <code className="font-mono text-charcoal">container:</code> outside it falls
                      back to the default image.
                    </span>
                  </li>
                )}
              </ul>
            </CardBody>
          </Card>
        </div>
      </div>
    </>
  );
}
