//! Windows equivalent of private Unix state permissions, checked on opened handles.
use std::{
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{addr_of_mut, null_mut},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid,
        WinLocalSystemSid,
    },
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, GetFileInformationByHandle},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(super) fn create_directory(path: &Path) -> io::Result<()> {
    let mut path: Vec<u16> = path.as_os_str().encode_wide().collect();
    if path.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "private directory path contains NUL"));
    }
    path.push(0);
    // Protected inheritance: only the object owner, SYSTEM, and Administrators get access.
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)\0".encode_utf16().collect();
    let mut descriptor = null_mut();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result =
            if CreateDirectoryW(path.as_ptr(), &attributes) == 0 { Err(io::Error::last_os_error()) } else { Ok(()) };
        LocalFree(descriptor);
        result
    }
}

pub(super) fn validate_private(file: &File) -> io::Result<()> {
    let denied = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private state must be owned by this user, SYSTEM, or Administrators and must not grant access to other users",
        )
    };
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(token);
        let mut length = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut length);
        let mut user = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(token.as_raw_handle(), TokenUser, user.as_mut_ptr().cast(), length, &mut length) == 0 {
            return Err(io::Error::last_os_error());
        }
        let user_sid = (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let (mut owner, mut acl, mut descriptor) = (null_mut(), null_mut(), null_mut());
        let code = GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        );
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let result = (|| {
            if owner.is_null()
                || acl.is_null()
                || (EqualSid(owner, user_sid) == 0
                    && IsWellKnownSid(owner, WinLocalSystemSid) == 0
                    && IsWellKnownSid(owner, WinBuiltinAdministratorsSid) == 0)
            {
                return Err(denied());
            }
            for index in 0..(*acl).AceCount {
                let mut ace = null_mut();
                if GetAce(acl, u32::from(index), &mut ace) == 0 {
                    return Err(io::Error::last_os_error());
                }
                match (*ace.cast::<ACE_HEADER>()).AceType {
                    1 => continue, // ACCESS_DENIED_ACE_TYPE only removes access.
                    0 => {}        // ACCESS_ALLOWED_ACE_TYPE
                    _ => return Err(denied()),
                }
                let allowed = &mut *ace.cast::<ACCESS_ALLOWED_ACE>();
                let sid = addr_of_mut!(allowed.SidStart).cast();
                if allowed.Mask != 0
                    && EqualSid(sid, user_sid) == 0
                    && [WinLocalSystemSid, WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid]
                        .iter()
                        .all(|kind| IsWellKnownSid(sid, *kind) == 0)
                {
                    return Err(denied());
                }
            }
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
                return Err(io::Error::last_os_error());
            }
            if info.nNumberOfLinks != 1 {
                return Err(denied());
            }
            Ok(())
        })();
        LocalFree(descriptor);
        result
    }
}
