use crate::models::{DiskSample, SystemSample};
use chrono::Utc;
use std::net::UdpSocket;
use sysinfo::{Disks, Networks, System};

pub fn hostname() -> String {
    System::host_name().unwrap_or_else(|| "LightMonitor Host".to_string())
}

/// Return the local address selected for outbound IPv4 traffic.
///
/// UDP connect only asks the OS to select a route; it does not send a packet.
pub fn local_ip() -> Option<String> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_loopback() && !address.is_unspecified()).then(|| address.to_string())
}

pub struct LocalCollector {
    system: System,
    disks: Disks,
    networks: Networks,
}

impl LocalCollector {
    pub fn new() -> Self {
        Self {
            system: System::new_all(),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
        }
    }

    pub fn collect(&mut self) -> SystemSample {
        self.system.refresh_all();
        self.disks.refresh(true);
        self.networks.refresh(true);

        let load = System::load_average();
        let (network_rx_bytes, network_tx_bytes) =
            self.networks.iter().fold((0, 0), |(rx, tx), (_, network)| {
                (
                    rx + network.total_received(),
                    tx + network.total_transmitted(),
                )
            });

        SystemSample {
            hostname: hostname(),
            os: System::long_os_version()
                .or_else(System::name)
                .unwrap_or_else(|| "unknown".to_string()),
            kernel: System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
            uptime_seconds: System::uptime(),
            cpu_cores: self.system.cpus().len() as u32,
            cpu_percent: self.system.global_cpu_usage(),
            memory_total_bytes: self.system.total_memory(),
            memory_used_bytes: self.system.used_memory(),
            swap_total_bytes: self.system.total_swap(),
            swap_used_bytes: self.system.used_swap(),
            load_average: [load.one, load.five, load.fifteen],
            network_rx_bytes,
            network_tx_bytes,
            disks: self
                .disks
                .iter()
                .map(|disk| DiskSample {
                    name: disk.name().to_string_lossy().to_string(),
                    mount_point: disk.mount_point().to_string_lossy().to_string(),
                    total_bytes: disk.total_space(),
                    available_bytes: disk.available_space(),
                })
                .collect(),
            collected_at: Utc::now(),
        }
    }
}
