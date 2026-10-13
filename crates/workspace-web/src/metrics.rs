use serde_json::{Value, json};
use std::time::Instant;

pub struct Metrics {
    started: Instant,
    sample_at: Instant,
    cpu_ticks: Option<u64>,
    pub collections: u64,
    pub collection_ms: u64,
    pub maximum_ms: u64,
    pub cache_hits: u64,
    pub stream_messages: u64,
    pub stream_bytes: u64,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            sample_at: Instant::now(),
            cpu_ticks: ticks(),
            collections: 0,
            collection_ms: 0,
            maximum_ms: 0,
            cache_hits: 0,
            stream_messages: 0,
            stream_bytes: 0,
        }
    }
}
fn ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}
impl Metrics {
    pub fn sample(&mut self, unified: bool) -> Value {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let rss = status
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .map(|v| v * 1024);
        let current = ticks();
        #[cfg(unix)]
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
        #[cfg(not(unix))]
        let hz = 100.0;
        let cpu = current
            .zip(self.cpu_ticks)
            .filter(|_| hz > 0.0)
            .map(|(now, prev)| {
                now.saturating_sub(prev) as f64
                    / hz
                    / self.sample_at.elapsed().as_secs_f64().max(0.001)
                    * 100.0
            });
        self.cpu_ticks = current;
        self.sample_at = Instant::now();
        let scope = if unified {
            "cranibox server process including Web and node gateway; excludes child processes and managed services"
        } else {
            "workspace-web process; excludes child processes, gateway and managed services"
        };
        json!({"rss_bytes":rss,"cpu_percent":cpu,"uptime_seconds":self.started.elapsed().as_secs(),"collections":self.collections,"collection_ms":self.collection_ms,"maximum_collection_ms":self.maximum_ms,"cache_hits":self.cache_hits,"stream_messages":self.stream_messages,"stream_bytes":self.stream_bytes,"scope":scope,"unified_server":unified})
    }
}
