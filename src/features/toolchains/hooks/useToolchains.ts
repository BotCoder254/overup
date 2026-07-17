import { useQuery } from '@tanstack/react-query';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { getToolchains } from '../api/toolchainsApi';

export { useWorkspaceId };

export const toolchainsKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'toolchains'] as const;

export function useToolchains() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: toolchainsKey(workspaceId ?? ''),
    queryFn: () => getToolchains(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}
