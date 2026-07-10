//! Ambient host telemetry sampled for the `heartbeat` message. Idiomatic
//! `sysinfo` usage: one long-lived `System`, refreshed before every read
//! rather than reconstructed per-sample.

use protocol::RunnerHealth;
use sysinfo::{Disks, System};

pub struct HealthSampler {
    system: System,
}

impl HealthSampler {
    pub fn new() -> Self {
        Self { system: System::new_all() }
    }

    pub async fn sample(&mut self, docker: &Option<bollard::Docker>) -> RunnerHealth {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        let disks = Disks::new_with_refreshed_list();
        let (disk_total, disk_used) = disks.iter().fold((0u64, 0u64), |(total, used), disk| {
            let total_space = disk.total_space();
            let available = disk.available_space();
            (
                total + total_space,
                used + total_space.saturating_sub(available),
            )
        });

        let docker_version = match docker {
            Some(client) => client.version().await.ok().and_then(|v| v.version),
            None => None,
        };

        RunnerHealth {
            cpu_permille: Some((self.system.global_cpu_usage() * 10.0) as u64),
            mem_used_bytes: Some(self.system.used_memory()),
            mem_total_bytes: Some(self.system.total_memory()),
            disk_used_bytes: Some(disk_used),
            disk_total_bytes: Some(disk_total),
            docker_version,
            os: System::long_os_version(),
            uptime_secs: Some(System::uptime()),
        }
    }
}
