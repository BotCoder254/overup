import { formatDistanceToNow } from 'date-fns';
import { KeyRound, MoreVertical, Pencil, Trash2 } from 'lucide-react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import { MenuItem, Popover } from '../../../components/ui/Popover';
import { TBody, Table, Td, Th, THead, Tr } from '../../../components/ui/Table';
import type { Runner } from '../../../types/runner';
import { RunnerStatusBadge } from './RunnerStatusBadge';

interface RunnersTableProps {
  slug: string;
  runners: Runner[];
  onRename: (runner: Runner) => void;
  onRegenerateToken: (runner: Runner) => void;
  onRevoke: (runner: Runner) => void;
}

export function RunnersTable({
  slug,
  runners,
  onRename,
  onRegenerateToken,
  onRevoke,
}: RunnersTableProps) {
  return (
    <Table>
      <THead>
        <Tr>
          <Th>Name</Th>
          <Th>Status</Th>
          <Th className="hidden md:table-cell">Labels</Th>
          <Th className="hidden lg:table-cell">Version</Th>
          <Th className="hidden sm:table-cell">Last seen</Th>
          <Th className="w-10" />
        </Tr>
      </THead>
      <TBody>
        {runners.map((runner) => (
          <Tr key={runner.id} className="hover:bg-surface">
            <Td>
              <Link
                to={workspacePath(slug, `runners/${runner.id}`)}
                className="font-medium text-charcoal hover:text-primary"
              >
                {runner.name}
              </Link>
              {/* On phones the Labels/Last-seen columns are hidden; fold them in here. */}
              <div className="mt-0.5 flex flex-wrap items-center gap-1 md:hidden">
                {runner.labels.map((label) => (
                  <Badge key={label} variant="outline">
                    {label}
                  </Badge>
                ))}
                <span className="text-xs text-steel sm:hidden">
                  {runner.lastSeenAt
                    ? formatDistanceToNow(new Date(runner.lastSeenAt), { addSuffix: true })
                    : 'Never seen'}
                </span>
              </div>
            </Td>
            <Td>
              <RunnerStatusBadge status={runner.status} draining={runner.draining} />
            </Td>
            <Td className="hidden md:table-cell">
              <div className="flex flex-wrap gap-1">
                {runner.labels.length === 0 ? (
                  <span className="text-steel">—</span>
                ) : (
                  runner.labels.map((label) => (
                    <Badge key={label} variant="outline">
                      {label}
                    </Badge>
                  ))
                )}
              </div>
            </Td>
            <Td className="hidden font-mono text-xs text-steel lg:table-cell">
              {runner.version ?? '—'}
            </Td>
            <Td className="hidden text-steel sm:table-cell">
              {runner.lastSeenAt
                ? formatDistanceToNow(new Date(runner.lastSeenAt), { addSuffix: true })
                : 'Never'}
            </Td>
            <Td>
              <Popover
                ariaLabel={`Actions for ${runner.name}`}
                align="end"
                panelClassName="w-48"
                renderTrigger={(triggerProps) => (
                  <button
                    type="button"
                    {...triggerProps}
                    aria-label={`Actions for ${runner.name}`}
                    className="rounded p-1.5 text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                  >
                    <MoreVertical size={16} aria-hidden="true" />
                  </button>
                )}
              >
                {({ close }) => (
                  <>
                    <MenuItem
                      icon={Pencil}
                      onSelect={() => {
                        close();
                        onRename(runner);
                      }}
                    >
                      Rename / relabel
                    </MenuItem>
                    <MenuItem
                      icon={KeyRound}
                      onSelect={() => {
                        close();
                        onRegenerateToken(runner);
                      }}
                    >
                      Regenerate token
                    </MenuItem>
                    <div className="my-1 border-t border-steel/20" aria-hidden="true" />
                    <MenuItem
                      icon={Trash2}
                      destructive
                      onSelect={() => {
                        close();
                        onRevoke(runner);
                      }}
                    >
                      Revoke
                    </MenuItem>
                  </>
                )}
              </Popover>
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
