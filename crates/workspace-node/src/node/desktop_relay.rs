use super::*;
use agent_workspace_runtime::peers::desktop::{Event, Stream};

pub(super) async fn run(
    endpoint: &str,
    token: &str,
    id: &str,
    caller: &str,
    ticket: &str,
    local: Arc<crate::local::LocalNode>,
) -> Result<()> {
    // The configured gateway authenticates this identity. It cannot bypass the
    // target-owned grant, sharing epoch or common direct/local controller lease.
    let mut desktop = local.relay_desktop(caller).await?;
    let mut url = config::check_url(endpoint, true, true)?;
    url.set_path(&format!("/ws/desktops/{id}"));
    url.set_query(None);
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse()?);
    request
        .headers_mut()
        .insert("x-cranibox-desktop-ticket", ticket.parse()?);
    let mut relay = Stream::connect_request(request, None).await?;
    loop {
        tokio::select! {
            output = desktop.output.recv() => match output {
                Some(Event::Data(bytes)) => relay.send(bytes)?,
                Some(Event::Error(error)) => anyhow::bail!(error),
                None => break,
            },
            input = relay.output.recv() => match input {
                Some(Event::Data(bytes)) => desktop.send(bytes)?,
                Some(Event::Error(error)) => anyhow::bail!(error),
                None => break,
            },
        }
    }
    Ok(())
}
