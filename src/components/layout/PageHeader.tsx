import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { useMe } from '../../features/auth/hooks/useAuth';
import { workspacePath } from '../../app/navigation';

interface PageHeaderProps {
  title: string;
  description?: string;
  /** Right-aligned page actions (buttons, filters). */
  actions?: ReactNode;
  /** Optional intermediate breadcrumb for detail pages (workspace / parent / title). */
  parent?: { label: string; to: string };
}

/**
 * Contextual header rendered at the top of every page inside the content
 * canvas: breadcrumb (workspace / page), title, optional description and
 * actions. Branding lives in the sidebar, so this stays purely contextual.
 */
export function PageHeader({ title, description, actions, parent }: PageHeaderProps) {
  const { data: me } = useMe();

  return (
    <header className="mb-6 animate-fade-in space-y-1.5 border-b border-steel/10 pb-5">
      {me?.workspace && (
        <nav aria-label="Breadcrumb" className="flex items-center gap-1.5 text-xs text-steel">
          <Link
            to={workspacePath(me.workspace.slug)}
            className="max-w-[16rem] truncate transition-colors hover:text-charcoal"
          >
            {me.workspace.name}
          </Link>
          <span aria-hidden="true">/</span>
          {parent && (
            <>
              <Link
                to={parent.to}
                className="max-w-[12rem] truncate transition-colors hover:text-charcoal"
              >
                {parent.label}
              </Link>
              <span aria-hidden="true">/</span>
            </>
          )}
          <span className="max-w-[20rem] truncate text-charcoal">{title}</span>
        </nav>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-xl font-semibold tracking-tight sm:text-2xl">{title}</h1>
        {actions && <div className="ml-auto flex items-center gap-2">{actions}</div>}
      </div>
      {description && (
        <p className="max-w-2xl text-sm leading-relaxed text-steel">{description}</p>
      )}
    </header>
  );
}
