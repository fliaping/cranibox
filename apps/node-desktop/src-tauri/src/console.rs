//! The app is a Windows GUI program, so Windows gives it no console. Run as the
//! `cranibox` command it writes to the terminal that started it instead; streams the
//! caller redirected (a pipe or a file) are kept.
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
    System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    },
};

pub fn attach() {
    let missing = |id: STD_HANDLE| {
        // SAFETY: GetStdHandle only reads this process's standard handle table.
        let handle = unsafe { GetStdHandle(id) };
        handle.is_null() || handle == INVALID_HANDLE_VALUE
    };
    let streams = [
        (STD_INPUT_HANDLE, "CONIN$"),
        (STD_OUTPUT_HANDLE, "CONOUT$"),
        (STD_ERROR_HANDLE, "CONOUT$"),
    ];
    let missing: Vec<_> = streams.into_iter().filter(|(id, _)| missing(*id)).collect();
    // SAFETY: attaching to the parent's console has no preconditions; it fails
    // harmlessly when the parent has none (started from Explorer).
    if missing.is_empty() || unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    for (id, device) in missing {
        let name: Vec<u16> = device.encode_utf16().chain([0]).collect();
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let handle: HANDLE = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            // SAFETY: the handle is open and owned by this process for its lifetime.
            unsafe { SetStdHandle(id, handle) };
        }
    }
}
