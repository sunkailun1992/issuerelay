//! Windows named-pipe security: only the user the host runs as (and SYSTEM) may open
//! IssueRelay's pipe, and a client talks only to a pipe whose server runs as the same user
//! (design section 10.2). Without a descriptor of its own a pipe lets Everyone read, and any
//! local user could create the pipe first under IssueRelay's name.
use std::ffi::c_void;
use std::io;
use std::os::windows::io::RawHandle;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Full access for the pipe's owner, the user the host runs as (`OW`), and for SYSTEM; nobody
/// else, not even the read access Everyone gets by default. `P` keeps inherited entries out.
pub const PIPE_SDDL: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

/// A security descriptor built from `PIPE_SDDL`, kept for the server's life: every pipe
/// instance is created with it.
pub struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor is a heap block owned by this value and never changed after `new`;
// CreateNamedPipeW only reads it, from whichever thread creates an instance.
unsafe impl Send for PipeSecurity {}
// SAFETY: as for `Send`: nothing writes to the descriptor or the attributes after `new`.
unsafe impl Sync for PipeSecurity {}

impl PipeSecurity {
    /// Builds the descriptor from `PIPE_SDDL`. Fails only when the system cannot parse or
    /// allocate it.
    pub fn new() -> io::Result<Self> {
        let sddl: Vec<u16> = PIPE_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16. On success the system allocates the
        // descriptor with LocalAlloc; `Drop` frees it.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    /// The attributes for `ServerOptions::create_with_security_attributes_raw`. Valid while
    /// `self` lives and is not moved; pass it straight to the call.
    pub fn attributes(&self) -> *mut c_void {
        std::ptr::from_ref(&self.attributes).cast_mut().cast()
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW, freed once.
        unsafe { LocalFree(self.descriptor) };
    }
}

/// The user SID of a process's token, kept in the buffer GetTokenInformation filled.
struct TokenUserSid {
    buffer: Vec<u8>,
}

impl TokenUserSid {
    fn of_process(process: HANDLE) -> io::Result<Self> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: `process` has query access; the token handle is closed below.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = Self::of_token(token);
        // SAFETY: opened above, closed once.
        unsafe { CloseHandle(token) };
        result
    }

    fn of_token(token: HANDLE) -> io::Result<Self> {
        let mut needed = 0_u32;
        // SAFETY: a size query: no buffer, only `needed` is written.
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_u8; needed as usize];
        // SAFETY: `buffer` has `needed` writable bytes.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { buffer })
    }

    fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation(TokenUser) wrote a TOKEN_USER at the start of the buffer,
        // whose `Sid` points into the buffer. A Vec<u8> is not aligned for TOKEN_USER, hence the
        // unaligned read.
        unsafe {
            std::ptr::read_unaligned(self.buffer.as_ptr().cast::<TOKEN_USER>())
                .User
                .Sid
        }
    }
}

/// Whether the process serving the pipe behind `handle` (a client end) runs as the same user as
/// this process. An error means it could not be told, which callers treat like a "no".
// `handle` is a kernel handle value, not a pointer into this process's memory: it is only passed
// to GetNamedPipeServerProcessId, which checks it and fails with ERROR_INVALID_HANDLE when it is
// not an open handle, so taking it in a safe function cannot lead to undefined behaviour.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn server_runs_as_current_user(handle: RawHandle) -> io::Result<bool> {
    let mut pid = 0_u32;
    // SAFETY: the system validates `handle` (see above); `RawHandle` and `HANDLE` are both
    // `*mut c_void`.
    if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle is checked and closed below.
    let server = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if server.is_null() {
        return Err(io::Error::last_os_error());
    }
    let theirs = TokenUserSid::of_process(server);
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(server) };
    let theirs = theirs?;
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no closing.
    let ours = TokenUserSid::of_process(unsafe { GetCurrentProcess() })?;
    // SAFETY: both SIDs point into buffers that live until the end of this function.
    Ok(unsafe { EqualSid(theirs.sid(), ours.sid()) } != 0)
}
