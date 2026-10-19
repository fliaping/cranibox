//! Local, owner-only Windows named pipe; remote SMB clients are rejected by the OS.
use super::{LocalMgmt, router};
use cranibox_contracts::llm::MgmtError;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio_util::sync::CancellationToken;
fn name(path: &Path) -> String {
    format!(
        r"\\.\pipe\cranibox-llm-{:x}",
        Sha256::digest(path.to_string_lossy().as_bytes())
    )
}
pub(crate) struct Listener {
    inner: NamedPipeServer,
    name: String,
}
fn create(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
    };
    // Owner Rights (OW) grants access only to the owner, with DACL inheritance disabled.
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)\0".encode_utf16().collect();
    let mut descriptor = std::ptr::null_mut();
    // SAFETY: NUL-terminated SDDL and valid output pointer; LocalFree releases the descriptor.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: descriptor and attributes stay live until CreateNamedPipeW has copied them.
    let result = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, &attributes as *const _ as *mut _)
    };
    unsafe {
        LocalFree(descriptor as _);
    }
    result
}
impl Listener {
    pub async fn bind(path: &Path) -> anyhow::Result<Self> {
        let name = name(path);
        Ok(Self {
            inner: create(&name, true)?,
            name,
        })
    }
}
pub(crate) async fn serve(listener: Listener, mgmt: LocalMgmt, cancel: CancellationToken) {
    let Listener { mut inner, name } = listener;
    let app = router(mgmt);
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _=cancel.cancelled()=>break,
            result=inner.connect()=>{
                if result.is_err(){break;}
                let next=match create(&name,false){Ok(next)=>next,Err(_)=>break};
                let connected=std::mem::replace(&mut inner,next);
                let app=app.clone();let stop=cancel.clone();
                connections.spawn(async move{
                    let service=hyper_util::service::TowerToHyperService::new(app);
                    let connection=hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(connected),service);
                    tokio::pin!(connection);
                    tokio::select!{_=stop.cancelled()=>{},_= &mut connection=>{}}
                });
            },
            Some(_)=connections.join_next(),if !connections.is_empty()=>{}
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}
pub(super) async fn connect(path: &Path) -> Result<NamedPipeClient, MgmtError> {
    let name = name(path);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match ClientOptions::new().open(&name) {
            Ok(client) => return Ok(client),
            Err(e)
                if matches!(e.raw_os_error(), Some(2 | 231))
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(_) => {
                return Err(MgmtError::Other(
                    "management named pipe is unavailable".into(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;

    #[tokio::test]
    async fn management_pipe_rejects_duplicate_server_and_has_protected_owner_acl() {
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::{
                    ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo,
                    SE_KERNEL_OBJECT,
                },
                DACL_SECURITY_INFORMATION,
            },
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("management");
        let listener = Listener::bind(&path).await.unwrap();
        assert!(Listener::bind(&path).await.is_err());
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: live pipe handle and valid output pointer. The allocated descriptor
        // and its textual conversion are released with LocalFree before asserting.
        let status = unsafe {
            GetSecurityInfo(
                listener.inner.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, 0);
        let mut sddl = std::ptr::null_mut();
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                1,
                DACL_SECURITY_INFORMATION,
                &mut sddl,
                std::ptr::null_mut(),
            )
        };
        unsafe {
            LocalFree(descriptor);
        }
        assert_ne!(converted, 0);
        let text = unsafe {
            let mut len = 0;
            while *sddl.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(sddl, len));
            LocalFree(sddl as _);
            text
        };
        assert!(text.starts_with("D:P"), "DACL must be protected: {text}");
        assert_eq!(text.matches("(A;").count(), 1, "only one allow ACE: {text}");
        assert!(
            text.ends_with(";;;OW)"),
            "only the owner is granted access: {text}"
        );
    }
}
