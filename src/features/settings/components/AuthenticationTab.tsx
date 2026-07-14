import { useQueryClient } from '@tanstack/react-query';
import { formatDistanceToNow } from 'date-fns';
import { MonitorSmartphone, ShieldAlert } from 'lucide-react';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { Dialog } from '../../../components/ui/Dialog';
import { Spinner } from '../../../components/ui/Spinner';
import { TBody, Table, Td, Th, THead, Tr } from '../../../components/ui/Table';
import type { Me } from '../../../types/user';
import type { UserSession } from '../../../types/session';
import { ME_QUERY_KEY } from '../../auth/hooks/useAuth';
import { deviceLabel } from '../lib/userAgent';
import { useRevokeAllSessions, useRevokeSession, useSessions } from '../hooks/useSettings';
import { DeleteAccountDialog } from './DeleteAccountDialog';

function relative(timestamp: string | null): string {
  if (!timestamp) return '—';
  return formatDistanceToNow(new Date(timestamp), { addSuffix: true });
}

/**
 * Active sessions (server-side revocation — a revoked token is rejected on
 * its very next request) plus the account danger zone.
 */
export function AuthenticationTab({ me }: { me: Me }) {
  const sessions = useSessions();
  const revoke = useRevokeSession();
  const revokeAll = useRevokeAllSessions();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [deleteOpen, setDeleteOpen] = useState(false);
  // The current session gets a confirm step — revoking it signs you out.
  const [confirmCurrent, setConfirmCurrent] = useState<UserSession | null>(null);

  const rows = sessions.data ?? [];
  const others = rows.filter((session) => !session.current).length;

  const revokeSession = (session: UserSession) => {
    revoke.mutate(session.id, {
      onSuccess: () => {
        if (session.current) {
          // The cookie is dead server-side: mirror useLogout so the client
          // doesn't linger as a zombie signed-in shell.
          queryClient.clear();
          queryClient.setQueryData(ME_QUERY_KEY, null);
          navigate('/', { replace: true });
        }
      },
    });
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="flex-wrap">
          <MonitorSmartphone size={16} className="text-steel" aria-hidden="true" />
          <h2 className="text-sm font-semibold text-charcoal">Active sessions</h2>
          <div className="ml-auto">
            <Button
              size="sm"
              variant="secondary"
              onClick={() => revokeAll.mutate()}
              isLoading={revokeAll.isPending}
              disabled={others === 0}
              title={others === 0 ? 'No other sessions are signed in.' : undefined}
            >
              Revoke all other sessions
            </Button>
          </div>
        </CardHeader>
        <CardBody className="p-0">
          {sessions.isLoading ? (
            <div className="flex items-center justify-center gap-2 p-8 text-sm text-steel">
              <Spinner className="h-4 w-4" />
              Loading sessions…
            </div>
          ) : sessions.isError ? (
            <p className="p-8 text-center text-sm text-steel">
              Could not load your sessions. Try again in a moment.
            </p>
          ) : (
            <Table className="border-0">
              <THead>
                <Tr>
                  <Th>Device</Th>
                  <Th>Signed in</Th>
                  <Th>Last seen</Th>
                  <Th>IP address</Th>
                  <Th className="text-right">Actions</Th>
                </Tr>
              </THead>
              <TBody>
                {rows.map((session) => (
                  <Tr key={session.id} className="hover:bg-surface/60">
                    <Td>
                      <div className="flex items-center gap-2">
                        <span className="text-sm">{deviceLabel(session.userAgent)}</span>
                        {session.current && <Badge variant="primary">Current</Badge>}
                      </div>
                    </Td>
                    <Td className="text-xs text-steel">{relative(session.createdAt)}</Td>
                    <Td className="text-xs text-steel">{relative(session.lastSeenAt)}</Td>
                    <Td>
                      <span className="font-mono text-xs text-steel">
                        {session.ip ?? 'Unknown'}
                      </span>
                    </Td>
                    <Td className="text-right">
                      <Button
                        size="sm"
                        variant="ghost"
                        className="text-danger hover:bg-danger/10"
                        isLoading={revoke.isPending && revoke.variables === session.id}
                        onClick={() =>
                          session.current ? setConfirmCurrent(session) : revokeSession(session)
                        }
                        title={
                          session.current
                            ? 'Revoking your current session signs you out immediately.'
                            : undefined
                        }
                      >
                        Revoke
                      </Button>
                    </Td>
                  </Tr>
                ))}
              </TBody>
            </Table>
          )}
        </CardBody>
      </Card>

      <Card className="border-danger/30">
        <CardHeader>
          <ShieldAlert size={16} className="text-danger" aria-hidden="true" />
          <h2 className="text-sm font-semibold text-charcoal">Danger zone</h2>
        </CardHeader>
        <CardBody>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0 text-sm text-steel">
              <p className="font-medium text-charcoal">Delete account</p>
              <p className="mt-0.5">
                Permanently removes your account, your workspace, and everything in it.
                Every session is revoked and there is no recovery.
              </p>
            </div>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              onClick={() => setDeleteOpen(true)}
            >
              Delete account
            </Button>
          </div>
        </CardBody>
      </Card>

      <DeleteAccountDialog open={deleteOpen} onClose={() => setDeleteOpen(false)} me={me} />

      <Dialog
        open={confirmCurrent !== null}
        onClose={() => setConfirmCurrent(null)}
        title="Revoke current session?"
        description="This is the session you are using right now. Revoking it signs you out immediately and returns you to the sign-in page."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmCurrent(null)}>
              Cancel
            </Button>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              isLoading={revoke.isPending}
              onClick={() => {
                if (confirmCurrent) {
                  revokeSession(confirmCurrent);
                  setConfirmCurrent(null);
                }
              }}
            >
              Sign out this session
            </Button>
          </>
        }
      />
    </div>
  );
}
