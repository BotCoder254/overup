import { NavLink } from 'react-router-dom';
import { NAV_GROUPS, workspacePath } from '../../app/navigation';
import { cn } from '../../lib/cn';

interface SidebarNavProps {
  slug: string;
  /** Called after a link is activated (closes the mobile drawer). */
  onNavigate?: () => void;
}

/** Grouped workspace navigation, generated from the shared nav config. */
export function SidebarNav({ slug, onNavigate }: SidebarNavProps) {
  return (
    <div className="space-y-0">
      {NAV_GROUPS.map((group) => (
        <div key={group.label}>
          <div className="px-2.5 pb-1 pt-5 text-[11px] font-medium uppercase tracking-wider text-steel first:pt-0">
            {group.label}
          </div>
          <div className="space-y-0.5">
            {group.items.map((item) => (
              <NavLink
                key={item.segment || 'dashboard'}
                to={workspacePath(slug, item.segment)}
                end={item.segment === ''}
                onClick={onNavigate}
                className={({ isActive }) =>
                  cn(
                    'group flex items-center gap-2.5 rounded px-2.5 py-1.5 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                    isActive
                      ? 'bg-primary/10 font-medium text-primary'
                      : 'text-charcoal/80 hover:bg-charcoal/5 hover:text-charcoal',
                  )
                }
              >
                {({ isActive }) => (
                  <>
                    <item.icon
                      size={16}
                      strokeWidth={2}
                      aria-hidden="true"
                      className={cn(
                        'shrink-0',
                        isActive ? 'text-primary' : 'text-steel group-hover:text-charcoal',
                      )}
                    />
                    {item.label}
                  </>
                )}
              </NavLink>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
