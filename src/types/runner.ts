/** Runner Management API types — mirror the backend camelCase DTOs. */

export type RunnerStatus = 'offline' | 'idle' | 'busy' | 'disabled';

/** Server-side sizing preset for hosted runners. */
export type RunnerResourceProfile = 'small' | 'standard' | 'large';

/** Ambient host telemetry sampled on every heartbeat, server-validated. */
export interface RunnerHealth {
  cpuPermille?: number;
  memUsedBytes?: number;
  memTotalBytes?: number;
  diskUsedBytes?: number;
  diskTotalBytes?: number;
  dockerVersion?: string;
  os?: string;
  uptimeSecs?: number;
}

export interface Runner {
  id: string;
  name: string;
  labels: string[];
  status: RunnerStatus;
  version: string | null;
  lastSeenAt: string | null;
  createdAt: string;
  revoked: boolean;
  lastHealth: RunnerHealth | null;
  draining: boolean;
  /** Hosted runner provisioned by the control plane itself. */
  managed: boolean;
  /**
   * Static failure category when background provisioning of a hosted runner
   * failed (image_pull_failed, container_create_failed,
   * container_start_failed, provision_timeout); null otherwise.
   */
  provisionError: string | null;
  /** Sizing preset for hosted runners; null for self-hosted rows. */
  resourceProfile: RunnerResourceProfile | null;
}
