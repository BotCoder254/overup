import { useMutation, useQuery } from '@tanstack/react-query';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { getWorkflowDetail, getWorkflows, validateWorkflow } from '../api/workflowsApi';

export const workflowsKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'workflows'] as const;
export const workflowKey = (workspaceId: string, workflowId: string) =>
  ['workspaces', workspaceId, 'workflows', workflowId] as const;

export function useWorkflows() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: workflowsKey(workspaceId ?? ''),
    queryFn: () => getWorkflows(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useWorkflowDetail(workflowId: string | undefined) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: workflowKey(workspaceId ?? '', workflowId ?? ''),
    queryFn: () => getWorkflowDetail(workspaceId!, workflowId!),
    enabled: Boolean(workspaceId && workflowId),
  });
}

/** Debounced by the caller; each call validates one editor snapshot. */
export function useValidateWorkflow() {
  const workspaceId = useWorkspaceId();
  return useMutation({
    mutationFn: (content: string) => validateWorkflow(workspaceId!, content),
  });
}
