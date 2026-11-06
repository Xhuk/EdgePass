//! The DACL that goes on the IPC pipe.
//!
//! `CreateNamedPipe` without a `SECURITY_ATTRIBUTES` leaves the object at the
//! mercy of the logon session's default DACL, which on a shared machine (or a
//! machine where other accounts share groups with this one) grants more than
//! this single user. The pipe is the widget's control surface: `run` hands
//! decrypted secrets to a child process's environment and `seal`/`unseal` move
//! the state of a vault every other client also depends on, so "anyone in this
//! session" is too broad.
//!
//! What is built here is one ACE — full access for the SID of the user running
//! the widget — and nothing else: no `Everyone`, no `Authenticated Users`, no
//! inherited ACEs (`SE_DACL_PROTECTED`).
//!
//! Failing *closed* would mean "no pipe at all", which is worse than the
//! documented status quo: [`UserOnly::new`] returns `None` and the caller
//! creates the unrestricted pipe while saying so in the log. Hardening is
//! never silently claimed.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows_sys::Win32::Security::{
    AddAccessAllowedAce, GetLengthSid, GetTokenInformation, InitializeAcl,
    InitializeSecurityDescriptor, SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
    TokenUser, ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

// Only the read-back in tests needs these: production hands the descriptor to
// `CreateNamedPipe` and lets the OS enforce it.
#[cfg(test)]
use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
#[cfg(test)]
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
#[cfg(test)]
use windows_sys::Win32::Security::{GetAce, DACL_SECURITY_INFORMATION};

/// `ACCESS_ALLOWED_ACE_TYPE` from `winnt.h`. windows-sys 0.61 links the
/// functions but does not export this constant.
#[cfg(test)]
pub const ACCESS_ALLOWED_ACE_TYPE: u8 = 0x00;

/// `SECURITY_DESCRIPTOR_REVISION` from `winnt.h`; also not exported.
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

/// `FILE_ALL_ACCESS`, which is what a client's `GENERIC_READ | GENERIC_WRITE`
/// is mapped against when it opens the pipe. Granted to the user SID only.
/// Spelled out rather than `GENERIC_ALL` so the ACE says what it means.
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

/// Owns every buffer a `SECURITY_ATTRIBUTES` points at.
///
/// `SECURITY_ATTRIBUTES` is a bare pointer plus a length; the descriptor, the
/// ACL and the SID all have to stay alive (and unmoved) until the pipe has
/// been created, so they live in here and the caller just keeps this value.
pub struct UserOnly {
    // `ACL`/`ACCESS_ALLOWED_ACE` require DWORD alignment, so these are word
    // buffers: allocator luck is not an alignment guarantee.
    _sd: Box<SECURITY_DESCRIPTOR>,
    _acl: Vec<u64>,
    attrs: SECURITY_ATTRIBUTES,
}

impl UserOnly {
    /// A descriptor granting this user — and only this user — access to the
    /// pipe.
    ///
    /// `None` if the token, the SID or the ACL could not be assembled; the
    /// caller then creates the unrestricted pipe. The reason goes to the log:
    /// a silently unhardened pipe is the one outcome worth avoiding.
    pub fn new() -> Option<Self> {
        match Self::build() {
            Ok(s) => Some(s),
            Err(e) => {
                crate::log_warn!(
                    "ipc: no pude construir la DACL de usuario ({e}); el pipe queda sin restringir"
                );
                None
            }
        }
    }

    /// The same thing, but each failure names the call that gave up and its
    /// `GetLastError`: the log line is only useful if it says where, and tests
    /// need the reason even with no logger installed.
    pub(crate) fn build() -> Result<Self, String> {
        let sid = current_user_sid().ok_or("no pude leer el SID del usuario")?;

        // sizeof(ACL) + sizeof(ACCESS_ALLOWED_ACE) - sizeof(the SID's first
        // DWORD, already counted inside ACCESS_ALLOWED_ACE) + sizeof(SID).
        let acl_bytes = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            - std::mem::size_of::<u32>()
            + sid.byte_len();
        let mut acl = vec![0u64; acl_bytes.div_ceil(8)];
        let pacl = acl.as_mut_ptr() as *mut ACL;

        let mut sd = Box::new(SECURITY_DESCRIPTOR::default());
        let psd = sd.as_mut() as *mut SECURITY_DESCRIPTOR as PSECURITY_DESCRIPTOR;

        // SAFETY: every pointer handed to the API points at a buffer owned
        // above (alive for the whole call and beyond) or is null where the API
        // allows null.
        unsafe {
            if InitializeAcl(pacl, acl_bytes as u32, ACL_REVISION) == 0 {
                return Err(format!("InitializeAcl: error {}", last_error()));
            }
            if AddAccessAllowedAce(pacl, ACL_REVISION, FILE_ALL_ACCESS, sid.as_ptr()) == 0 {
                return Err(format!("AddAccessAllowedAce: error {}", last_error()));
            }
            if InitializeSecurityDescriptor(psd, SECURITY_DESCRIPTOR_REVISION) == 0 {
                return Err(format!(
                    "InitializeSecurityDescriptor: error {}",
                    last_error()
                ));
            }
            if SetSecurityDescriptorDacl(psd, 1, pacl as *const ACL, 0) == 0 {
                return Err(format!("SetSecurityDescriptorDacl: error {}", last_error()));
            }
            // Without this the descriptor is "unprotected": anything
            // inheritable could still be merged in at creation time, and the
            // ACE list would no longer be the whole story.
            if SetSecurityDescriptorControl(psd, SE_DACL_PROTECTED, SE_DACL_PROTECTED) == 0 {
                return Err(format!(
                    "SetSecurityDescriptorControl: error {}",
                    last_error()
                ));
            }
        }

        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: psd,
            // No inheriting handles: a `run` child must not inherit the pipe.
            bInheritHandle: 0,
        };
        Ok(Self {
            _sd: sd,
            _acl: acl,
            attrs,
        })
    }

    /// The `SECURITY_ATTRIBUTES` to hand to the pipe creation. Valid only for
    /// as long as `self` lives.
    pub fn as_ptr(&mut self) -> *mut c_void {
        &mut self.attrs as *mut SECURITY_ATTRIBUTES as *mut c_void
    }
}

/// `GetLastError` as a number, for the failure messages above.
fn last_error() -> u32 {
    // SAFETY: no preconditions; the value belongs to this thread.
    unsafe { GetLastError() }
}

/// A SID copied into a word-aligned buffer.
///
/// `Vec<u8>` would be the obvious container, but the ACL APIs require DWORD
/// alignment and an allocator is not obliged to hand it out. The buffer is
/// rounded up to whole words, so the SID's real length is carried separately:
/// reading the word count as a byte count is precisely the mistake that makes
/// `AddAccessAllowedAce` fail with `ERROR_ALLOTTED_SPACE_EXCEEDED` (1344).
pub struct AlignedSid {
    words: Vec<u64>,
    byte_len: usize,
}

impl AlignedSid {
    /// The pointer the Win32 SID-taking functions want.
    pub fn as_ptr(&self) -> PSID {
        self.words.as_ptr() as PSID
    }

    /// The SID's own length in bytes — not the padded buffer's.
    pub fn byte_len(&self) -> usize {
        self.byte_len
    }
}

/// The SID of the user this process runs as, copied into its own aligned
/// buffer so it outlives the token query.
///
/// Reachable from the rest of the crate because the pipe's DACL test has to
/// compare the ACE against it.
pub fn current_user_sid() -> Option<AlignedSid> {
    // SAFETY: the token handle is closed on every path below, and the buffer
    // handed to `GetTokenInformation` is sized by that call's own report.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let sid = token_sid(token);
        CloseHandle(token);
        sid
    }
}

/// # Safety
/// `token` must be a live token handle opened with `TOKEN_QUERY`.
unsafe fn token_sid(token: HANDLE) -> Option<AlignedSid> {
    let mut needed = 0u32;
    GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u8; needed as usize];
    if GetTokenInformation(
        token,
        TokenUser,
        buf.as_mut_ptr() as *mut c_void,
        needed,
        &mut needed,
    ) == 0
    {
        return None;
    }
    let sid = (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid;
    if sid.is_null() {
        return None;
    }
    let byte_len = GetLengthSid(sid) as usize;
    if byte_len == 0 {
        return None;
    }
    let mut words = vec![0u64; byte_len.div_ceil(8)];
    std::ptr::copy_nonoverlapping(sid as *const u8, words.as_mut_ptr() as *mut u8, byte_len);
    Some(AlignedSid { words, byte_len })
}

/// The ACE list the kernel actually put on a live handle.
///
/// Only tests ask: production hands the descriptor to `CreateNamedPipe` and
/// trusts the OS from there. Kept out of release builds entirely.
#[cfg(test)]
pub struct ReadBack {
    /// `(AceType, SID)` in the order the kernel stored them.
    pub aces: Vec<(u8, PSID)>,
    sd: PSECURITY_DESCRIPTOR,
}

#[cfg(test)]
impl ReadBack {
    /// # Safety
    /// `handle` must be a live kernel handle of an object carrying a DACL.
    pub unsafe fn from_handle(handle: HANDLE) -> Option<Self> {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let rc = GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        );
        if rc != 0 || dacl.is_null() {
            return None;
        }

        let mut aces = Vec::with_capacity((*dacl).AceCount as usize);
        for i in 0..(*dacl).AceCount as u32 {
            let mut ace: *mut c_void = std::ptr::null_mut();
            if GetAce(dacl, i, &mut ace) == 0 || ace.is_null() {
                continue;
            }
            let allowed = ace as *const ACCESS_ALLOWED_ACE;
            // The SID starts at `SidStart`, whose *value* is the SID's first
            // DWORD — the address is what identifies it.
            let sid = &(*allowed).SidStart as *const u32 as PSID;
            aces.push(((*allowed).Header.AceType, sid));
        }
        Some(Self { aces, sd })
    }
}

#[cfg(test)]
impl Drop for ReadBack {
    fn drop(&mut self) {
        if !self.sd.is_null() {
            // SAFETY: the descriptor came from `GetSecurityInfo`, which
            // documents `LocalFree` as its release.
            unsafe {
                LocalFree(self.sd as HLOCAL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{EqualSid, GetSecurityDescriptorDacl, IsValidSid};

    #[test]
    fn the_current_user_sid_is_readable_and_valid() {
        let sid = current_user_sid().expect("token SID");
        assert_ne!(
            unsafe { IsValidSid(sid.as_ptr()) },
            0,
            "the copied buffer must hold a valid SID"
        );
        // 68 bytes is the documented maximum; anything larger is a bad copy.
        assert!(
            sid.byte_len() <= 68,
            "SID too large: {} bytes",
            sid.byte_len()
        );
    }

    #[test]
    fn the_descriptor_carries_one_non_inheritable_ace_for_this_user() {
        let mut security = UserOnly::build().expect("descriptor");

        // Read it back through the same API a client's access check uses.
        let attrs = unsafe { &*(security.as_ptr() as *const SECURITY_ATTRIBUTES) };
        assert_eq!(
            attrs.nLength as usize,
            std::mem::size_of::<SECURITY_ATTRIBUTES>()
        );
        assert_eq!(attrs.bInheritHandle, 0, "the pipe must not be inheritable");

        let psd = attrs.lpSecurityDescriptor as PSECURITY_DESCRIPTOR;
        let mut present = 0;
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut defaulted = 0;
        assert_ne!(
            unsafe { GetSecurityDescriptorDacl(psd, &mut present, &mut dacl, &mut defaulted) },
            0,
            "GetSecurityDescriptorDacl"
        );
        assert_ne!(present, 0, "the descriptor must carry a DACL");
        assert_eq!(defaulted, 0, "the DACL must not be the default one");
        assert_eq!(unsafe { (*dacl).AceCount }, 1, "one ACE, not a list");

        let mut ace: *mut c_void = std::ptr::null_mut();
        assert_ne!(unsafe { GetAce(dacl, 0, &mut ace) }, 0);
        let allowed = ace as *const ACCESS_ALLOWED_ACE;
        let header = unsafe { (*allowed).Header };
        assert_eq!(header.AceType, ACCESS_ALLOWED_ACE_TYPE);
        assert_eq!(header.AceFlags, 0, "nothing inheritable");

        let sid = current_user_sid().expect("token SID");
        let ace_sid = unsafe { &(*allowed).SidStart as *const u32 as PSID };
        assert_ne!(
            unsafe { EqualSid(ace_sid, sid.as_ptr()) },
            0,
            "the single ACE must be this user"
        );
    }
}
