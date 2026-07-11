//! Resource profiles for hosted runners. Fixed server-side presets — users
//! pick a slug, never raw limits. The limits bound the runner service
//! container's own cgroup AND are forwarded to its job containers via the
//! runner crate's `RUNNER_JOB_*` env knobs (job containers run as siblings
//! on the host daemon, outside the runner's cgroup — the forwarded env is
//! what actually bounds jobs).

/// Validated sizing preset for a hosted runner (`resource_profile` column;
/// CHECK-constrained to the same three slugs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceProfile {
    Small,
    Standard,
    Large,
}

/// Container limits a profile maps to.
#[derive(Clone, Copy, Debug)]
pub struct ResourceLimits {
    pub memory_bytes: i64,
    pub nano_cpus: i64,
    pub pids_limit: i64,
}

const GIB: i64 = 1024 * 1024 * 1024;
const CPU: i64 = 1_000_000_000;

impl ResourceProfile {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "small" => Some(Self::Small),
            "standard" => Some(Self::Standard),
            "large" => Some(Self::Large),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Standard => "standard",
            Self::Large => "large",
        }
    }

    /// Standard matches the runner crate's own job-container defaults
    /// (2 CPU / 2 GiB / 512 pids), so the default profile changes nothing
    /// for existing deployments.
    pub fn limits(self) -> ResourceLimits {
        match self {
            Self::Small => ResourceLimits {
                memory_bytes: GIB,
                nano_cpus: CPU,
                pids_limit: 256,
            },
            Self::Standard => ResourceLimits {
                memory_bytes: 2 * GIB,
                nano_cpus: 2 * CPU,
                pids_limit: 512,
            },
            Self::Large => ResourceLimits {
                memory_bytes: 4 * GIB,
                nano_cpus: 4 * CPU,
                pids_limit: 1024,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_slugs() {
        for profile in [
            ResourceProfile::Small,
            ResourceProfile::Standard,
            ResourceProfile::Large,
        ] {
            assert_eq!(ResourceProfile::from_str(profile.as_str()), Some(profile));
        }
        assert_eq!(ResourceProfile::from_str("xlarge"), None);
        assert_eq!(ResourceProfile::from_str(""), None);
    }

    #[test]
    fn standard_matches_runner_job_defaults() {
        let limits = ResourceProfile::Standard.limits();
        assert_eq!(limits.memory_bytes, 2_147_483_648);
        assert_eq!(limits.nano_cpus, 2_000_000_000);
        assert_eq!(limits.pids_limit, 512);
    }
}
