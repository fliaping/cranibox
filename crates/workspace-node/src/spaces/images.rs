//! Published space images: one multi-arch manifest per desktop, mirrored to three
//! registries with the same digest. X11 or Wayland is chosen per container.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Registry {
    /// The registry that answers fastest from this device.
    #[default]
    Auto,
    Dockerhub,
    Ghcr,
    Aliyun,
}
const REGISTRIES: [(Registry, &str, &str); 3] = [
    (
        Registry::Dockerhub,
        "docker.io/xuping/agent-workspace",
        "registry-1.docker.io",
    ),
    (
        Registry::Ghcr,
        "ghcr.io/fliaping/agent-workspace",
        "ghcr.io",
    ),
    (
        Registry::Aliyun,
        "registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace",
        "registry.cn-hangzhou.aliyuncs.com",
    ),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Desktop {
    #[default]
    Xfce,
    Kde,
    Lxqt,
}
impl Desktop {
    fn tag(self) -> &'static str {
        match self {
            Desktop::Xfce => "ubuntu-xfce",
            Desktop::Kde => "ubuntu-kde",
            Desktop::Lxqt => "ubuntu-lxqt",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Display {
    /// PixelFlux's native Computer Use interface only runs under Wayland.
    #[default]
    Wayland,
    X11,
}
impl Display {
    pub fn environment(self) -> &'static str {
        match self {
            Display::Wayland => "PIXELFLUX_WAYLAND=true\n",
            Display::X11 => "PIXELFLUX_WAYLAND=false\n",
        }
    }
}

/// Image reference for a concrete registry; `Auto` must be resolved first.
pub fn reference(registry: Registry, desktop: Desktop) -> String {
    let repository = REGISTRIES
        .iter()
        .find(|(id, ..)| *id == registry)
        .map_or(REGISTRIES[0].1, |(_, repository, _)| repository);
    format!("{repository}:{}", desktop.tag())
}

/// Round-trip time to each registry's API root; any HTTP answer counts as reachable.
pub async fn probe() -> Vec<(Registry, Option<Duration>)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build();
    let Ok(client) = client else {
        return REGISTRIES.iter().map(|(id, ..)| (*id, None)).collect();
    };
    let checks = REGISTRIES.iter().map(|(id, _, host)| {
        let client = client.clone();
        async move {
            let started = Instant::now();
            let reached = client
                .get(format!("https://{host}/v2/"))
                .send()
                .await
                .is_ok();
            (*id, reached.then(|| started.elapsed()))
        }
    });
    futures_util::future::join_all(checks).await
}

pub async fn resolve(registry: Registry) -> Registry {
    if registry != Registry::Auto {
        return registry;
    }
    fastest(&probe().await)
}
fn fastest(results: &[(Registry, Option<Duration>)]) -> Registry {
    results
        .iter()
        .filter_map(|(id, time)| time.map(|time| (time, *id)))
        .min_by_key(|(time, _)| *time)
        .map_or(Registry::Dockerhub, |(_, id)| id)
}

/// Docker platform for an engine's `Architecture`; the published manifests carry both.
pub fn platform(architecture: &str) -> Result<&'static str> {
    Ok(match architecture {
        "x86_64" | "amd64" => "linux/amd64",
        "aarch64" | "arm64" => "linux/arm64",
        other => bail!(
            "Cranibox space images are published for x86-64 and ARM64; this engine reports {other}."
        ),
    })
}

/// Choices shown when creating a space.
pub fn catalog() -> Value {
    let desktops = [Desktop::Xfce, Desktop::Kde, Desktop::Lxqt]
        .map(|desktop| json!({"id": desktop, "tag": desktop.tag()}));
    json!({
        "registries": REGISTRIES.iter().map(|(id, repository, _)| json!({"id":id,"repository":repository})).collect::<Vec<_>>(),
        "desktops": desktops,
        "displays": [Display::Wayland, Display::X11],
        "defaults": {"registry":Registry::Auto,"desktop":Desktop::default(),"display":Display::default()},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_and_platforms() {
        assert_eq!(
            reference(Registry::Aliyun, Desktop::Kde),
            "registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace:ubuntu-kde"
        );
        assert_eq!(
            reference(Registry::Dockerhub, Desktop::Xfce),
            "docker.io/xuping/agent-workspace:ubuntu-xfce"
        );
        assert_eq!(platform("aarch64").unwrap(), "linux/arm64");
        assert_eq!(platform("x86_64").unwrap(), "linux/amd64");
        assert!(platform("armv7l").is_err());
    }

    #[test]
    fn auto_prefers_the_fastest_reachable_registry() {
        let ms = Duration::from_millis;
        assert_eq!(
            fastest(&[
                (Registry::Dockerhub, None),
                (Registry::Ghcr, Some(ms(300))),
                (Registry::Aliyun, Some(ms(40)))
            ]),
            Registry::Aliyun
        );
        assert_eq!(
            fastest(&[(Registry::Dockerhub, None), (Registry::Ghcr, None)]),
            Registry::Dockerhub
        );
    }
}
