use sysinfo::{Disks, System};

use crate::models::NodeMetrics;

pub struct MetricsCollector {
    system: System,
}

impl MetricsCollector {
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_cpu_usage();

        Self { system }
    }

    pub fn collect(&mut self) -> NodeMetrics {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();

        let disks = Disks::new_with_refreshed_list();
        let (disk_total_bytes, disk_used_bytes) = disks.iter().fold((0i64, 0i64), |acc, disk| {
            let total = disk.total_space() as i64;
            let used = total - disk.available_space() as i64;
            (acc.0 + total, acc.1 + used)
        });

        NodeMetrics {
            cpu_usage_percent: Some(self.system.global_cpu_usage() as f64),
            memory_total_bytes: Some(self.system.total_memory() as i64),
            memory_used_bytes: Some(self.system.used_memory() as i64),
            disk_total_bytes: Some(disk_total_bytes),
            disk_used_bytes: Some(disk_used_bytes),
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}
