import { Check, ChevronsUpDown, Plus } from 'lucide-react';
import type { WorkspaceSummary } from '../../types/workspace';
import { MenuItem, Popover } from '../ui/Popover';
import { cn } from '../../lib/cn';

interface WorkspaceSwitcherProps {
  workspace: WorkspaceSummary;
}

/**
 * Sidebar workspace selector: the active workspace name opens a popover with
 * the workspace list. The backend enforces one workspace per user, so the
 * create action is shown disabled until multi-workspace lands.
 */
export function WorkspaceSwitcher({ workspace }: WorkspaceSwitcherProps) {
  return (
    <div className="min-w-0 flex-1">
      <Popover
        ariaLabel="Workspace switcher"
        side="bottom"
        align="start"
        panelClassName="w-60"
        renderTrigger={(triggerProps, isOpen) => (
          <button
            type="button"
            {...triggerProps}
            className={cn(
              'flex w-full min-w-0 items-center gap-2 rounded px-2 py-1.5 text-left text-sm font-medium text-charcoal transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
              isOpen ? 'bg-charcoal/5' : 'hover:bg-charcoal/5',
            )}
          >
            <span className="min-w-0 flex-1 truncate">{workspace.name}</span>
            <ChevronsUpDown size={14} aria-hidden="true" className="shrink-0 text-steel" />
          </button>
        )}
      >
        {({ close }) => (
          <>
            <button
              type="button"
              role="menuitem"
              tabIndex={-1}
              onClick={close}
              className="flex w-full items-center gap-2.5 rounded px-2.5 py-2 text-left text-sm transition-colors hover:bg-surface focus-visible:bg-surface focus-visible:outline-none"
            >
              <span
                aria-hidden="true"
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded bg-primary/10 text-sm font-semibold text-primary"
              >
                {workspace.name.charAt(0).toUpperCase()}
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate font-medium text-charcoal">{workspace.name}</span>
                <span className="block text-xs text-steel">Owner</span>
              </span>
              <Check size={16} aria-hidden="true" className="ml-auto shrink-0 text-primary" />
            </button>
            <div className="my-1 border-t border-steel/20" aria-hidden="true" />
            <MenuItem icon={Plus} disabled hint="1 workspace per account">
              Create workspace
            </MenuItem>
          </>
        )}
      </Popover>
    </div>
  );
}
