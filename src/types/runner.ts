/** Runner Management API types — mirror the backend camelCase DTOs. */

export type RunnerStatus = 'offline' | 'idle' | 'busy' | 'disabled';

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
}
