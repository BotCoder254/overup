import { api } from '../../../lib/api';
import type { ValidateResponse, WorkflowDetail, WorkflowSummary } from '../../../types/workflow';

export async function getWorkflows(workspaceId: string): Promise<WorkflowSummary[]> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/workflows`)
    .json<{ workflows: WorkflowSummary[] }>();
  return body.workflows;
}

export async function getWorkflowDetail(
  workspaceId: string,
  workflowId: string,
): Promise<WorkflowDetail> {
  return api.get(`/api/workspaces/${workspaceId}/workflows/${workflowId}`).json<WorkflowDetail>();
}

/** Offline validation: the same parser sync uses, no side effects. */
export async function validateWorkflow(
  workspaceId: string,
  content: string,
): Promise<ValidateResponse> {
  return api
    .post(`/api/workspaces/${workspaceId}/workflows/validate`, { json: { content } })
    .json<ValidateResponse>();
}
