import { api } from '../../../lib/api';
import type { ToolchainsResponse } from '../../../types/toolchain';

export async function getToolchains(workspaceId: string): Promise<ToolchainsResponse> {
  return api.get(`/api/workspaces/${workspaceId}/toolchains`).json<ToolchainsResponse>();
}

/** Install (pull + warm) a toolchain image on the hosted-runner daemon. */
export async function installToolchain(workspaceId: string, key: string): Promise<void> {
  await api.post(`/api/workspaces/${workspaceId}/toolchains/${key}/install`);
}

/** Uninstall a toolchain: drop it from the prewarm list and remove the image. */
export async function uninstallToolchain(workspaceId: string, key: string): Promise<void> {
  await api.delete(`/api/workspaces/${workspaceId}/toolchains/${key}`);
}
