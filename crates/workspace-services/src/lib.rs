//! Linux service manager embedded in Cranibox.
#[cfg(target_os = "linux")]
mod manager;
pub fn run(args: Vec<String>) -> anyhow::Result<i32> {
    #[cfg(target_os = "linux")]
    {
        manager::entry(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("workspace-services requires Linux and s6; use the platform service manager")
    }
}
