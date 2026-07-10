import { Command } from 'cmdk';
import { useNavigate } from 'react-router-dom';
import { useHotkeys } from 'react-hotkeys-hook';
import { NAV_GROUPS, workspacePath } from '../../app/navigation';
import { useMe } from '../../features/auth/hooks/useAuth';

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/**
 * Global navigation palette (Ctrl/Cmd+K). Items come from the shared nav
 * config; cmdk provides the portal, dialog semantics, filtering, and Escape.
 */
export function CommandPalette({ open, onOpenChange }: CommandPaletteProps) {
  const { data: me } = useMe();
  const navigate = useNavigate();

  useHotkeys('mod+k', () => onOpenChange(!open), {
    enableOnFormTags: true,
    preventDefault: true,
  });

  const slug = me?.workspace?.slug;
  if (!slug) return null;

  return (
    <Command.Dialog
      open={open}
      onOpenChange={onOpenChange}
      label="Search"
      overlayClassName="fixed inset-0 z-50 bg-navy/40 animate-fade-in"
      contentClassName="fixed left-1/2 top-24 z-50 w-[calc(100%-2rem)] max-w-md -translate-x-1/2 rounded border border-steel/20 bg-canvas p-2 animate-scale-in"
    >
      <Command.Input
        placeholder="Go to…"
        className="w-full border-b border-steel/20 bg-transparent px-2.5 pb-2.5 pt-1 text-sm text-charcoal placeholder:text-steel focus:outline-none"
      />
      <Command.List className="max-h-72 overflow-y-auto pt-1">
        <Command.Empty className="px-2.5 py-6 text-center text-sm text-steel">
          No results found.
        </Command.Empty>
        {NAV_GROUPS.map((group) => (
          <Command.Group
            key={group.label}
            heading={group.label}
            className="[&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:pt-2.5 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wider [&_[cmdk-group-heading]]:text-steel"
          >
            {group.items.map((item) => (
              <Command.Item
                key={item.segment || 'dashboard'}
                value={item.label}
                onSelect={() => {
                  navigate(workspacePath(slug, item.segment));
                  onOpenChange(false);
                }}
                className="flex cursor-pointer items-center gap-2.5 rounded px-2.5 py-2 text-sm text-charcoal data-[selected=true]:bg-primary/10 data-[selected=true]:text-primary"
              >
                <item.icon size={16} strokeWidth={2} aria-hidden="true" className="shrink-0" />
                {item.label}
              </Command.Item>
            ))}
          </Command.Group>
        ))}
      </Command.List>
    </Command.Dialog>
  );
}
