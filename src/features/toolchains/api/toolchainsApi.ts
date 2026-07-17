import { api } from '../../../lib/api';
import type { ToolchainsResponse } from '../../../types/toolchain';

export async function getToolchains(workspaceId: string): Promise<ToolchainsResponse> {
  return api.get(`/api/workspaces/${workspaceId}/toolchains`).json<ToolchainsResponse>();
}
