import { useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Spinner } from '../../../components/ui/Spinner';
import { Tabs } from '../../../components/ui/Tabs';
import { useMe } from '../../auth/hooks/useAuth';
import { AuthenticationTab } from '../components/AuthenticationTab';
import { ProfileTab } from '../components/ProfileTab';
import { WorkspaceTab } from '../components/WorkspaceTab';

type TabId = 'profile' | 'authentication' | 'workspace';

const TAB_IDS: TabId[] = ['profile', 'authentication', 'workspace'];

function tabFromParams(params: URLSearchParams): TabId {
  const raw = params.get('tab');
  return TAB_IDS.includes(raw as TabId) ? (raw as TabId) : 'profile';
}

/**
 * Settings: a centered, calm management surface inside the app shell.
 * Sub-tabs sync to `?tab=` so the UserFooter shortcuts (and shared URLs)
 * land on the right section.
 */
export function SettingsPage() {
  const me = useMe();
  const [searchParams, setSearchParams] = useSearchParams();
  const tab = tabFromParams(searchParams);

  // Normalize an invalid ?tab= away so the URL always reflects the view.
  useEffect(() => {
    const raw = searchParams.get('tab');
    if (raw && !TAB_IDS.includes(raw as TabId)) {
      setSearchParams({ tab: 'profile' }, { replace: true });
    }
  }, [searchParams, setSearchParams]);

  const onChangeTab = (id: string) => {
    setSearchParams({ tab: id }, { replace: true });
  };

  return (
    <>
      <PageHeader
        title="Settings"
        description="Manage your profile, authentication, and workspace configuration."
      />
      <div className="mx-auto w-full max-w-3xl">
        <Tabs
          ariaLabel="Settings sections"
          active={tab}
          onChange={onChangeTab}
          className="mb-6"
          tabs={[
            { id: 'profile', label: 'Profile' },
            { id: 'authentication', label: 'Authentication' },
            { id: 'workspace', label: 'Workspace' },
          ]}
        />
        {me.isLoading || !me.data ? (
          <div className="flex min-h-[30vh] items-center justify-center">
            <Spinner className="h-6 w-6 text-steel" />
          </div>
        ) : (
          <>
            {tab === 'profile' && <ProfileTab me={me.data} />}
            {tab === 'authentication' && <AuthenticationTab me={me.data} />}
            {tab === 'workspace' && <WorkspaceTab me={me.data} />}
          </>
        )}
      </div>
    </>
  );
}
