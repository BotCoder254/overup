import { useMe } from '../../features/auth/hooks/useAuth';
import { Logo } from '../brand/Logo';
import { SidebarNav } from './SidebarNav';
import { UserFooter } from './UserFooter';
import { WorkspaceSwitcher } from './WorkspaceSwitcher';

interface SidebarProps {
  /** Called after a nav link is activated (closes the mobile drawer). */
  onNavigate?: () => void;
}

/**
 * The persistent navigation frame: workspace identity + switcher fixed at the
 * top, an independently scrollable nav list in the middle, and the user
 * footer fixed at the bottom. Rendered both as the desktop rail and inside
 * the mobile drawer. Search and the notification bell live in the shell's
 * top bar (TopBar on desktop, MobileTopBar below lg), not here.
 */
export function Sidebar({ onNavigate }: SidebarProps) {
  const { data: me } = useMe();
  if (!me?.workspace) return null; // guarded by WorkspaceRoute; satisfies types

  return (
    <div className="flex h-full flex-col">
      <div className="shrink-0 px-3 pt-3">
        <div className="flex items-center gap-1.5">
          <Logo size="sm" withWordmark={false} className="shrink-0 px-1 text-charcoal" />
          <WorkspaceSwitcher workspace={me.workspace} />
        </div>
      </div>
      <nav aria-label="Workspace" className="mt-4 min-h-0 flex-1 overflow-y-auto px-3 pb-4">
        <SidebarNav slug={me.workspace.slug} onNavigate={onNavigate} />
      </nav>
      <div className="shrink-0">
        <UserFooter me={me} />
      </div>
    </div>
  );
}
