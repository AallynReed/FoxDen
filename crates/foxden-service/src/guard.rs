//! Keeps an installed service, which runs as LocalSystem, away from files and folders that an
//! ordinary user controls.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GENERIC_ALL, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
    INHERIT_ONLY_ACE, IsWellKnownSid, OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION, PSID,
    SECURITY_ATTRIBUTES, WinBuiltinAdministratorsSid, WinCreatorOwnerSid, WinLocalSystemSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, DELETE, FILE_APPEND_DATA,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA,
    GetFileInformationByHandle, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
};

/// Full control for SYSTEM and Administrators, reading for Users, nothing taken from the folder
/// above.
const PRIVATE: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
/// The account that owns what Windows itself installs.
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
const ACCESS_ALLOWED: u8 = 0;
const ACCESS_DENIED: u8 = 1;

/// Rights that let their holder change a file, or what is in a folder.
const CHANGE: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | FILE_DELETE_CHILD
    | DELETE
    | WRITE_DAC
    | WRITE_OWNER
    | GENERIC_WRITE
    | GENERIC_ALL;
/// Of those, the ones that let a folder on the way to a file be put aside for another.
const REPLACE: u32 = FILE_DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_ALL;

/// What decides whether a file or folder can be trusted.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Access {
    link: bool,
    folder: bool,
    /// Owned by SYSTEM, Administrators or TrustedInstaller.
    owned: bool,
    /// Rights that anyone else has on it.
    here: u32,
    /// Rights that anyone else gets on what is made inside it.
    inside: u32,
}

fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain([0]).collect()
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// SYSTEM, Administrators and TrustedInstaller.
unsafe fn trusted(sid: PSID) -> bool {
    let installer = wide(TRUSTED_INSTALLER);
    let mut known: PSID = null_mut();
    unsafe {
        let named = ConvertStringSidToSidW(installer.as_ptr(), &mut known) != 0;
        let is_installer = named && EqualSid(sid, known) != 0;
        LocalFree(known);
        is_installer
            || IsWellKnownSid(sid, WinLocalSystemSid) != 0
            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
    }
}

/// The stand-in for whoever makes a file. Where nobody else may make files, that is one of
/// the above.
unsafe fn maker(sid: PSID) -> bool {
    unsafe { IsWellKnownSid(sid, WinCreatorOwnerSid) != 0 }
}

/// Reads the owner and the access list of `path` itself, not of what a link there points to.
fn read(path: &Path) -> Result<Access, String> {
    let fail = |what: &str| format!("{}: {what}: {}", path.display(), last_error());
    let name = wide(path);
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let flags = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;
    let opened = unsafe {
        CreateFileW(
            name.as_ptr(),
            READ_CONTROL,
            share,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if opened == INVALID_HANDLE_VALUE {
        return Err(fail("cannot be opened"));
    }
    let file = Handle(opened);

    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.0, &mut info) } == 0 {
        return Err(fail("cannot be examined"));
    }
    let (mut owner, mut list, mut descriptor) = (null_mut(), null_mut::<ACL>(), null_mut());
    let failed = unsafe {
        GetSecurityInfo(
            file.0,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut list,
            null_mut(),
            &mut descriptor,
        )
    };
    if failed != 0 {
        let e = std::io::Error::from_raw_os_error(failed as i32);
        return Err(format!(
            "{}: its access list cannot be read: {e}",
            path.display()
        ));
    }
    let mut access = Access {
        link: info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        folder: info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        owned: unsafe { trusted(owner) },
        here: 0,
        inside: 0,
    };
    let mut size: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    let bytes = size_of::<ACL_SIZE_INFORMATION>() as u32;
    // No list at all lets everyone do everything, and so must a list that cannot be read.
    let listed = !list.is_null()
        && unsafe { GetAclInformation(list, (&raw mut size).cast(), bytes, AclSizeInformation) }
            != 0;
    if !listed {
        (access.here, access.inside) = (u32::MAX, u32::MAX);
    }
    for index in 0..size.AceCount {
        let mut entry = null_mut();
        if unsafe { GetAce(list, index, &mut entry) } == 0 {
            (access.here, access.inside) = (u32::MAX, u32::MAX);
            break;
        }
        let header = unsafe { *entry.cast::<ACE_HEADER>() };
        // An entry of a kind not known here is taken to give everything away.
        let (rights, holder): (u32, PSID) = match header.AceType {
            ACCESS_ALLOWED => {
                let allowed = entry.cast::<ACCESS_ALLOWED_ACE>();
                unsafe { ((*allowed).Mask, (&raw mut (*allowed).SidStart).cast()) }
            }
            ACCESS_DENIED => continue,
            _ => (u32::MAX, null_mut()),
        };
        if !holder.is_null() && unsafe { trusted(holder) || maker(holder) } {
            continue;
        }
        let flags = u32::from(header.AceFlags);
        if flags & INHERIT_ONLY_ACE == 0 {
            access.here |= rights;
        }
        if flags & (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) != 0 {
            access.inside |= rights;
        }
    }
    unsafe { LocalFree(descriptor) };
    Ok(access)
}

/// Why a folder with this access cannot hold the service's files, if it cannot.
fn unsafe_folder(access: Access) -> Option<&'static str> {
    if access.link {
        Some("it is a link to somewhere else")
    } else if !access.folder {
        Some("it is not a folder")
    } else if !access.owned {
        Some("it does not belong to SYSTEM or Administrators")
    } else if (access.here | access.inside) & CHANGE != 0 {
        Some("ordinary users can write to it")
    } else {
        None
    }
}

/// Makes the data folder of an installed service, open to SYSTEM and Administrators only, or
/// checks that one which is already there is as closed as that.
pub fn data_dir(dir: &Path) -> Result<(), String> {
    let (name, list) = (wide(dir), wide(PRIVATE));
    let mut descriptor = null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            list.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    };
    if converted == 0 {
        return Err(format!("cannot build an access list: {}", last_error()));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let made = unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } != 0;
    let error = unsafe { GetLastError() };
    unsafe { LocalFree(descriptor) };
    // Made here and now, with that list, it needs no second look.
    if made {
        return Ok(());
    }
    if error != ERROR_ALREADY_EXISTS {
        let e = std::io::Error::from_raw_os_error(error as i32);
        return Err(format!("{}: cannot be made: {e}", dir.display()));
    }
    match unsafe_folder(read(dir)?) {
        Some(why) => Err(format!(
            "{} cannot hold the files of a service that runs as LocalSystem: {why}. Remove it, \
             or hand it to Administrators and close it to everyone else, then try again.",
            dir.display()
        )),
        None => Ok(()),
    }
}

/// Checks that only administrators can change the program at `exe` or put another in its
/// place: the file, its folder and every folder above that.
pub fn program(exe: &Path) -> Result<(), String> {
    let refuse = |path: &Path, why: &str| {
        Err(format!(
            "{} cannot be registered as a service: {} {why}. Copy foxden-service.exe to a folder \
             that only administrators can write to, such as one under Program Files, and run \
             install from there.",
            exe.display(),
            path.display()
        ))
    };
    for (depth, path) in exe.ancestors().enumerate() {
        let access = read(path)?;
        // Changing the file or its folder is enough; further up it takes replacing a folder.
        let rights = if depth < 2 { CHANGE } else { REPLACE };
        if access.link {
            return refuse(path, "is a link");
        } else if !access.owned {
            return refuse(path, "does not belong to SYSTEM or Administrators");
        } else if access.here & rights != 0 {
            return refuse(path, "can be changed by ordinary users");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;

    use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;

    use super::*;
    use crate::host::tests::scratch;

    const READ_AND_RUN: u32 = 0x1200a9;

    fn closed() -> Access {
        Access {
            link: false,
            folder: true,
            owned: true,
            here: READ_AND_RUN,
            inside: READ_AND_RUN,
        }
    }

    #[test]
    fn a_folder_is_safe_only_when_nobody_else_can_change_it() {
        assert_eq!(unsafe_folder(closed()), None);
        type Spoil = fn(&mut Access);
        let cases: [(&str, Spoil); 8] = [
            ("link", |a| a.link = true),
            ("not a folder", |a| a.folder = false),
            ("belong", |a| a.owned = false),
            ("write", |a| a.here |= FILE_WRITE_DATA),
            ("write", |a| a.here |= FILE_APPEND_DATA),
            ("write", |a| a.here |= FILE_DELETE_CHILD),
            ("write", |a| a.here |= WRITE_DAC),
            // New files would be open to others even though the folder is not.
            ("write", |a| a.inside |= GENERIC_ALL),
        ];
        for (word, spoil) in cases {
            let mut access = closed();
            spoil(&mut access);
            let why = unsafe_folder(access).unwrap_or_else(|| panic!("{word} was accepted"));
            assert!(why.contains(word), "{why}");
        }
    }

    /// Removes a folder that was closed to the user who runs the tests, who still owns it.
    struct Reopened(PathBuf);

    impl Drop for Reopened {
        fn drop(&mut self) {
            let name = wide(&self.0);
            // No access list at all, which lets it be removed.
            unsafe {
                SetNamedSecurityInfoW(
                    name.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    null(),
                    null(),
                );
            }
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn a_new_data_folder_is_closed_to_ordinary_users() {
        let dir = scratch("guard-new").join("data");
        assert_eq!(data_dir(&dir), Ok(()));
        let _tidy = Reopened(dir.clone());
        let access = read(&dir).unwrap();
        assert!(access.folder && !access.link);
        assert_eq!(access.here & CHANGE, 0, "{:#x}", access.here);
        assert_eq!(access.inside & CHANGE, 0, "{:#x}", access.inside);
        // Users may still read it, and no more.
        assert_eq!(access.here, READ_AND_RUN);
    }

    /// A folder that everyone may do anything in.
    fn open_folder(dir: &Path) {
        let (name, list) = (wide(dir), wide("D:P(A;OICI;FA;;;WD)"));
        let mut descriptor = null_mut();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                list.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            );
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            assert_ne!(CreateDirectoryW(name.as_ptr(), &attributes), 0);
            LocalFree(descriptor);
        }
    }

    #[test]
    fn a_data_folder_that_others_can_write_to_is_refused() {
        let dir = scratch("guard-open").join("data");
        open_folder(&dir);
        let access = read(&dir).unwrap();
        assert_eq!(
            access.here & CHANGE,
            CHANGE & !(GENERIC_WRITE | GENERIC_ALL)
        );
        assert_ne!(access.inside & CHANGE, 0);
        let refused = data_dir(&dir).unwrap_err();
        assert!(refused.contains("LocalSystem"), "{refused}");
        // Whoever runs the tests owns it: an elevated run as Administrators, any other as itself.
        let why = if access.owned {
            "can write"
        } else {
            "does not belong"
        };
        assert!(refused.contains(why), "{refused}");
    }

    #[test]
    fn a_link_is_refused_as_a_data_folder() {
        let base = scratch("guard-link");
        let (target, link) = (base.join("target"), base.join("link"));
        fs::create_dir(&target).unwrap();
        // A junction needs no special rights to make.
        let shell = std::env::var("ComSpec").unwrap_or_else(|_| "cmd".into());
        let made = Command::new(shell)
            .args(["/C", "mklink", "/J"])
            .args([&link, &target])
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        assert!(read(&link).unwrap().link);
        assert!(!read(&target).unwrap().link);
        let refused = data_dir(&link).unwrap_err();
        assert!(refused.contains("link"), "{refused}");
        fs::remove_dir(&link).unwrap();
    }

    #[test]
    fn a_program_of_windows_itself_may_be_registered() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let shell = PathBuf::from(root).join("System32").join("cmd.exe");
        assert_eq!(program(&shell), Ok(()));
    }

    #[test]
    fn a_program_that_others_can_replace_may_not() {
        let dir = scratch("guard-program").join("open");
        open_folder(&dir);
        let exe = dir.join("foxden-service.exe");
        fs::write(&exe, b"not a program").unwrap();
        let refused = program(&exe).unwrap_err();
        assert!(refused.contains("cannot be registered"), "{refused}");
        assert!(refused.contains("Program Files"), "{refused}");
    }
}
