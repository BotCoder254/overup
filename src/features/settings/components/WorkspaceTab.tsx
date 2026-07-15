import { useQueryClient } from '@tanstack/react-query';
import { format } from 'date-fns';
import { Trash2, Upload, Users } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import { Spinner } from '../../../components/ui/Spinner';
import { TBody, Table, Td, Th, THead, Tr } from '../../../components/ui/Table';
import type { Me } from '../../../types/user';
import { AvailabilityIndicator } from '../../workspace/components/AvailabilityIndicator';
import { useWorkspaceAvailability } from '../../workspace/hooks/useWorkspaceAvailability';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import {
  logoKey,
  useRemoveWorkspaceLogo,
  useUpdateWorkspace,
  useUploadWorkspaceLogo,
  useWorkspaceLogoUrl,
  useWorkspaceMembers,
} from '../hooks/useSettings';

/** Client pre-check only — the server is the authority (magic bytes + cap). */
const MAX_LOGO_BYTES = 2 * 1024 * 1024;
const ACCEPTED_TYPES = 'image/png,image/jpeg,image/webp,image/gif';

export function WorkspaceTab({ me }: { me: Me }) {
  const workspace = me.workspace;
  const updateWorkspace = useUpdateWorkspace();
  const logoUrl = useWorkspaceLogoUrl();
  const uploadLogo = useUploadWorkspaceLogo();
  const removeLogo = useRemoveWorkspaceLogo();
  const members = useWorkspaceMembers();

  const queryClient = useQueryClient();
  const workspaceId = useWorkspaceId();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState(workspace?.name ?? '');
  // Presigned URLs expire after ~10 min: on the first <img> error re-mint
  // the URL once (ref-guarded so a genuinely broken object can't loop) and
  // show the monogram fallback meanwhile.
  const [logoBroken, setLogoBroken] = useState(false);
  const remintedRef = useRef(false);

  useEffect(() => {
    setName(workspace?.name ?? '');
  }, [workspace?.name]);

  useEffect(() => {
    // A fresh URL gets a fresh chance.
    setLogoBroken(false);
  }, [logoUrl.data]);

  const onLogoError = () => {
    setLogoBroken(true);
    if (!remintedRef.current && workspaceId) {
      remintedRef.current = true;
      void queryClient.invalidateQueries({ queryKey: logoKey(workspaceId) });
    }
  };

  // Called before the early return so the hook runs unconditionally. The
  // check only matters when the name actually changed — otherwise the user's
  // own (existing) slug would report as "taken".
  const nameChanged = Boolean(workspace) && name.trim() !== workspace?.name;
  const availability = useWorkspaceAvailability(nameChanged ? name : '');

  if (!workspace) return null;

  const trimmed = name.trim();
  const nameProblem =
    trimmed.length > 0 && (trimmed.length < 2 || trimmed.length > 80)
      ? 'Between 2 and 80 characters.'
      : undefined;
  const dirty = trimmed !== workspace.name;

  const onPickFile = (file: File | undefined) => {
    if (!file) return;
    if (file.size > MAX_LOGO_BYTES) {
      toast.error('The image is too large (2 MiB max).');
      return;
    }
    uploadLogo.mutate(file);
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <h2 className="text-sm font-semibold text-charcoal">Workspace logo</h2>
        </CardHeader>
        <CardBody>
          <div className="flex items-start gap-4">
            {logoUrl.data && !logoBroken ? (
              <img
                src={logoUrl.data}
                alt=""
                onError={onLogoError}
                className="h-20 w-20 shrink-0 rounded border border-steel/20 object-cover"
              />
            ) : (
              <span
                aria-hidden="true"
                className="flex h-20 w-20 shrink-0 items-center justify-center rounded border border-steel/20 bg-primary/10 text-2xl font-semibold text-primary"
              >
                {workspace.name.charAt(0).toUpperCase()}
              </span>
            )}
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <input
                  ref={fileInputRef}
                  type="file"
                  accept={ACCEPTED_TYPES}
                  className="hidden"
                  onChange={(event) => {
                    onPickFile(event.target.files?.[0]);
                    event.target.value = '';
                  }}
                />
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => fileInputRef.current?.click()}
                  isLoading={uploadLogo.isPending}
                >
                  <Upload size={14} aria-hidden="true" />
                  Upload
                </Button>
                {logoUrl.data && (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="text-danger hover:bg-danger/10"
                    aria-label="Remove workspace logo"
                    onClick={() => removeLogo.mutate()}
                    isLoading={removeLogo.isPending}
                  >
                    <Trash2 size={14} aria-hidden="true" />
                  </Button>
                )}
              </div>
              <p className="mt-2 text-xs text-steel">
                PNG, JPEG, GIF, or WebP up to 2 MiB. The file is verified server-side and
                stored in the workspace's object storage.
              </p>
            </div>
          </div>
        </CardBody>
      </Card>

      <Card>
        <CardHeader>
          <h2 className="text-sm font-semibold text-charcoal">Workspace name</h2>
        </CardHeader>
        <CardBody>
          <div className="space-y-4">
            <div className="space-y-1.5">
              <FormField id="workspace-name" label="Name" error={nameProblem}>
                {(aria) => (
                  <Input
                    {...aria}
                    maxLength={120}
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                  />
                )}
              </FormField>
              {!nameProblem && <AvailabilityIndicator state={availability} />}
            </div>
            <FormField
              id="workspace-slug"
              label="Slug"
              hint="The slug anchors URLs and cannot be changed."
            >
              {(aria) => (
                <Input {...aria} value={workspace.slug} disabled className="font-mono" />
              )}
            </FormField>
            <div className="flex justify-end">
              <Button
                size="sm"
                onClick={() => dirty && !nameProblem && updateWorkspace.mutate({ name: trimmed })}
                disabled={!dirty || Boolean(nameProblem) || trimmed.length < 2}
                isLoading={updateWorkspace.isPending}
              >
                Save changes
              </Button>
            </div>
          </div>
        </CardBody>
      </Card>

      <Card>
        <CardHeader>
          <Users size={16} className="text-steel" aria-hidden="true" />
          <h2 className="text-sm font-semibold text-charcoal">Members</h2>
        </CardHeader>
        <CardBody className="p-0">
          <p className="border-b border-steel/10 px-4 py-2 text-xs text-steel">
            Invites are not available yet — the member list is read-only.
          </p>
          {members.isLoading ? (
            <div className="flex items-center justify-center gap-2 p-8 text-sm text-steel">
              <Spinner className="h-4 w-4" />
              Loading members…
            </div>
          ) : members.isError ? (
            <p className="p-8 text-center text-sm text-steel">
              Could not load the member list.
            </p>
          ) : (
            <Table className="border-0">
              <THead>
                <Tr>
                  <Th>Member</Th>
                  <Th>Email</Th>
                  <Th>Role</Th>
                  <Th>Joined</Th>
                </Tr>
              </THead>
              <TBody>
                {(members.data ?? []).map((member) => (
                  <Tr key={member.userId} className="hover:bg-surface/60">
                    <Td>
                      <div className="flex items-center gap-2.5">
                        {member.avatarUrl ? (
                          <img
                            src={member.avatarUrl}
                            alt=""
                            referrerPolicy="no-referrer"
                            className="h-7 w-7 shrink-0 rounded object-cover"
                          />
                        ) : (
                          <span
                            aria-hidden="true"
                            className="flex h-7 w-7 shrink-0 items-center justify-center rounded bg-primary/10 text-xs font-semibold text-primary"
                          >
                            {(member.displayName ?? member.username).charAt(0).toUpperCase()}
                          </span>
                        )}
                        <div className="min-w-0">
                          <div className="truncate text-sm font-medium text-charcoal">
                            {member.displayName ?? member.username}
                          </div>
                          <div className="truncate text-xs text-steel">@{member.username}</div>
                        </div>
                      </div>
                    </Td>
                    <Td className="text-xs text-steel">{member.email ?? '—'}</Td>
                    <Td>
                      <Badge variant={member.roleKey === 'owner' ? 'primary' : 'neutral'}>
                        {member.roleName}
                      </Badge>
                    </Td>
                    <Td className="text-xs text-steel">
                      {format(new Date(member.joinedAt), 'PP')}
                    </Td>
                  </Tr>
                ))}
              </TBody>
            </Table>
          )}
        </CardBody>
      </Card>
    </div>
  );
}
