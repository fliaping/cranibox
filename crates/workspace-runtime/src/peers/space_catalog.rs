//! Read-only metadata from a space's managing device. This grants no container control.
use super::{Store, config::Config};
use crate::{config, protocol::valid_node_id};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub id: Uuid,
    pub name: String,
    pub node: String,
    pub provider: String,
    pub phase: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub owner: String,
    pub spaces: Vec<Summary>,
}
impl Catalog {
    pub fn validate(&self, owner: &str) -> Result<()> {
        ensure!(
            self.owner == owner && valid_node_id(owner),
            "unexpected space catalog owner"
        );
        ensure!(self.spaces.len() <= 32, "space catalog limit exceeded");
        let mut ids = BTreeSet::new();
        let mut nodes = BTreeSet::new();
        for space in &self.spaces {
            ensure!(
                !space.id.is_nil()
                    && ids.insert(space.id)
                    && valid_node_id(&space.node)
                    && space.node != owner
                    && nodes.insert(&space.node),
                "invalid space identity"
            );
            ensure!(
                !space.name.trim().is_empty()
                    && space.name.len() <= 120
                    && !space.name.chars().any(char::is_control),
                "invalid space name"
            );
            ensure!(
                matches!(
                    space.provider.as_str(),
                    "docker" | "colima" | "orbstack" | "wsl"
                ),
                "invalid space provider"
            );
            ensure!(
                matches!(
                    space.phase.as_str(),
                    "preparing"
                        | "pulling"
                        | "creating"
                        | "starting"
                        | "enrolling"
                        | "ready"
                        | "stopping"
                        | "stopped"
                        | "removing"
                        | "archived"
                        | "interrupted"
                        | "failed"
                ),
                "invalid space phase"
            );
        }
        Ok(())
    }
}

impl Store {
    pub async fn space_catalog(&self, node: &str) -> Result<Value> {
        ensure!(valid_node_id(node), "invalid node ID");
        // Retain actual credential bytes so rotation at the same filename also
        // invalidates a response that was started by the old connection.
        let path = self.path.clone();
        let target = node.to_owned();
        let (route, binding) = tokio::task::spawn_blocking(move || -> Result<_> {
            let _lock = config::lock_credentials(&path)?;
            let configuration = Config::read(&path)?;
            let route = configuration
                .routes
                .get(&target)
                .context("direct route is not configured")?;
            let binding = serde_json::to_vec(&(
                &configuration.node_id,
                route.binding_view(),
                config::read_token(&route.token_file)?,
            ))?;
            Ok((route.clone(), binding))
        })
        .await??;
        let value = route
            .api(node)?
            .get(&format!("/api/v1/nodes/{node}/spaces"))
            .await?;
        let catalog: Catalog = serde_json::from_value(value)?;
        catalog.validate(node)?;
        let path = self.path.clone();
        let target = node.to_owned();
        tokio::task::spawn_blocking(move || {
            let _lock = config::lock_credentials(&path)?;
            let current = Config::read(&path)?;
            let route = current
                .routes
                .get(&target)
                .context("direct route was removed")?;
            ensure!(
                binding
                    == serde_json::to_vec(&(
                        &current.node_id,
                        route.binding_view(),
                        config::read_token(&route.token_file)?
                    ))?,
                "direct connection changed; refresh the space catalog"
            );
            Ok(serde_json::to_value(catalog)?)
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_catalog_rejects_wrong_owners_duplicates_and_private_fields() {
        let mut catalog = Catalog {
            owner: "manager".into(),
            spaces: vec![Summary {
                id: Uuid::new_v4(),
                name: "Agent lab".into(),
                node: "space-node".into(),
                provider: "docker".into(),
                phase: "ready".into(),
            }],
        };
        assert!(catalog.validate("manager").is_ok());
        assert!(catalog.validate("another-manager").is_err());
        let mut value = serde_json::to_value(&catalog).unwrap();
        value["spaces"][0]["password"] = "never export".into();
        assert!(serde_json::from_value::<Catalog>(value).is_err());
        catalog.spaces.push(catalog.spaces[0].clone());
        assert!(catalog.validate("manager").is_err());
        catalog.spaces.pop();
        catalog.spaces[0].node = "manager".into();
        assert!(catalog.validate("manager").is_err());
        catalog.spaces[0].node = "space-node".into();
        catalog.spaces[0].phase = "unknown".into();
        assert!(catalog.validate("manager").is_err());
    }
}
