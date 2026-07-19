#![allow(dead_code)] // This shared module is compiled separately by the library and binary crates.

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

type Handle = *mut c_void;

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;
const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
const ACL_REVISION: u32 = 2;
const OBJECT_INHERIT_ACE: u32 = 0x01;
const CONTAINER_INHERIT_ACE: u32 = 0x02;
const FILE_ALL_ACCESS: u32 = 0x001f_01ff;
const SE_FILE_OBJECT: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x0000_0001;
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const SE_DACL_PROTECTED: u16 = 0x1000;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_ALLOWED_OBJECT_ACE_TYPE: u8 = 5;
const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u8 = 9;
const ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE: u8 = 11;
#[cfg(test)]
const WIN_WORLD_SID: i32 = 1;
const WIN_CREATOR_OWNER_SID: i32 = 3;
const WIN_LOCAL_SYSTEM_SID: i32 = 22;
const WIN_BUILTIN_ADMINISTRATORS_SID: i32 = 26;
const DELETE: u32 = 0x0001_0000;
const FILE_DELETE_CHILD: u32 = 0x0000_0040;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const FILE_SHARE_DELETE: u32 = 0x0000_0004;
const CREATE_NEW: u32 = 1;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: i32,
}

#[repr(C)]
struct AclHeader {
    revision: u8,
    sbz1: u8,
    size: u16,
    ace_count: u16,
    sbz2: u16,
}

#[repr(C)]
struct AceHeader {
    ace_type: u8,
    ace_flags: u8,
    ace_size: u16,
}

#[repr(C)]
struct AccessAllowedAce {
    header: AceHeader,
    mask: u32,
    sid_start: u32,
}

#[derive(Default)]
#[repr(C)]
struct WindowsFileTime {
    low: u32,
    high: u32,
}

#[derive(Default)]
#[repr(C)]
struct ByHandleFileInformation {
    file_attributes: u32,
    creation_time: WindowsFileTime,
    last_access_time: WindowsFileTime,
    last_write_time: WindowsFileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

struct OwnedHandle(Handle);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            close_handle(self.0);
        }
    }
}

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct CurrentUser {
    _token_words: Vec<usize>,
    sid: *mut c_void,
}

struct PrivateAcl {
    _words: Vec<u32>,
    pointer: *mut c_void,
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(
        process_handle: Handle,
        desired_access: u32,
        token_handle: *mut Handle,
    ) -> i32;
    fn GetTokenInformation(
        token_handle: Handle,
        token_information_class: u32,
        token_information: *mut c_void,
        token_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
    fn GetLengthSid(sid: *mut c_void) -> u32;
    fn InitializeAcl(acl: *mut c_void, acl_length: u32, acl_revision: u32) -> i32;
    fn AddAccessAllowedAceEx(
        acl: *mut c_void,
        ace_revision: u32,
        ace_flags: u32,
        access_mask: u32,
        sid: *mut c_void,
    ) -> i32;
    fn InitializeSecurityDescriptor(security_descriptor: *mut c_void, revision: u32) -> i32;
    fn SetSecurityDescriptorDacl(
        security_descriptor: *mut c_void,
        dacl_present: i32,
        dacl: *mut c_void,
        dacl_defaulted: i32,
    ) -> i32;
    fn SetSecurityDescriptorControl(
        security_descriptor: *mut c_void,
        control_bits_of_interest: u16,
        control_bits_to_set: u16,
    ) -> i32;
    fn GetSecurityDescriptorControl(
        security_descriptor: *mut c_void,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
    fn GetNamedSecurityInfoW(
        object_name: *mut u16,
        object_type: u32,
        security_info: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        security_descriptor: *mut *mut c_void,
    ) -> u32;
    fn GetAce(acl: *mut c_void, ace_index: u32, ace: *mut *mut c_void) -> i32;
    fn EqualSid(first: *mut c_void, second: *mut c_void) -> i32;
    fn IsWellKnownSid(sid: *mut c_void, sid_type: i32) -> i32;
    fn ConvertStringSidToSidW(string_sid: *const u16, sid: *mut *mut c_void) -> i32;
    #[cfg(test)]
    fn CreateWellKnownSid(
        sid_type: i32,
        domain_sid: *mut c_void,
        sid: *mut c_void,
        sid_size: *mut u32,
    ) -> i32;
    fn SetNamedSecurityInfoW(
        object_name: *mut u16,
        object_type: u32,
        security_info: u32,
        owner: *mut c_void,
        group: *mut c_void,
        dacl: *mut c_void,
        sacl: *mut c_void,
    ) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: Handle,
    ) -> Handle;
    fn CreateDirectoryW(path_name: *const u16, security_attributes: *mut SecurityAttributes)
        -> i32;
    fn GetFileInformationByHandle(file: Handle, information: *mut ByHandleFileInformation) -> i32;
    fn MoveFileW(existing_name: *const u16, new_name: *const u16) -> i32;
    fn MoveFileExW(existing_name: *const u16, new_name: *const u16, flags: u32) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    #[link_name = "CloseHandle"]
    fn close_handle(handle: Handle) -> i32;
}

#[allow(dead_code)] // Used by the binary crate; the library crate only needs `protect`.
pub(crate) fn move_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let from = from
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[allow(dead_code)] // Used by each crate's private ingestion-module copy.
pub(crate) fn replace_existing(from: &Path, to: &Path) -> io::Result<()> {
    let from = from
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn current_user() -> io::Result<CurrentUser> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle(token);

    let mut required = 0u32;
    let first_result = unsafe {
        GetTokenInformation(token.0, TOKEN_USER_CLASS, ptr::null_mut(), 0, &mut required)
    };
    if first_result != 0
        || required == 0
        || io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER)
    {
        return Err(io::Error::last_os_error());
    }

    let word = std::mem::size_of::<usize>();
    let mut token_words = vec![0usize; (required as usize).div_ceil(word)];
    if unsafe {
        GetTokenInformation(
            token.0,
            TOKEN_USER_CLASS,
            token_words.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token_user = unsafe { &*(token_words.as_ptr().cast::<SidAndAttributes>()) };
    let sid_length = unsafe { GetLengthSid(token_user.sid) } as usize;
    if sid_length == 0 {
        return Err(io::Error::last_os_error());
    }
    let sid = token_user.sid;
    Ok(CurrentUser {
        _token_words: token_words,
        sid,
    })
}

fn private_acl(user: &CurrentUser, ace_flags: u32) -> io::Result<PrivateAcl> {
    let sid_length = unsafe { GetLengthSid(user.sid) } as usize;
    if sid_length == 0 {
        return Err(io::Error::last_os_error());
    }
    // ACL header (8 bytes) plus ACCESS_ALLOWED_ACE without its placeholder SID
    // (8 bytes), followed by the complete variable-length SID.
    let acl_bytes = 16usize
        .checked_add(sid_length)
        .ok_or_else(|| io::Error::other("private artifact ACL is too large"))?;
    let mut acl_words = vec![0u32; acl_bytes.div_ceil(std::mem::size_of::<u32>())];
    let acl = acl_words.as_mut_ptr().cast();
    let acl_length = u32::try_from(acl_words.len() * std::mem::size_of::<u32>())
        .map_err(|_| io::Error::other("private artifact ACL is too large"))?;
    if unsafe { InitializeAcl(acl, acl_length, ACL_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, ace_flags, FILE_ALL_ACCESS, user.sid) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(PrivateAcl {
        _words: acl_words,
        pointer: acl,
    })
}

pub(crate) fn create_private_new(path: &Path) -> io::Result<()> {
    let user = current_user()?;
    let acl = private_acl(&user, 0)?;
    let mut descriptor_words = vec![0usize; 8];
    let descriptor = descriptor_words.as_mut_ptr().cast::<c_void>();
    if unsafe { InitializeSecurityDescriptor(descriptor, SECURITY_DESCRIPTOR_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetSecurityDescriptorDacl(descriptor, 1, acl.pointer, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetSecurityDescriptorControl(descriptor, SE_DACL_PROTECTED, SE_DACL_PROTECTED) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SecurityAttributes {
        length: u32::try_from(std::mem::size_of::<SecurityAttributes>())
            .map_err(|_| io::Error::other("Windows security attributes are too large"))?,
        security_descriptor: descriptor,
        inherit_handle: 0,
    };
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            (&mut attributes as *mut SecurityAttributes).cast(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    drop(OwnedHandle(handle));
    Ok(())
}

pub(crate) fn file_identity(path: &Path) -> io::Result<(u32, u64)> {
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = OwnedHandle(handle);
    let mut information = ByHandleFileInformation::default();
    if unsafe { GetFileInformationByHandle(handle.0, &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        information.volume_serial_number,
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low),
    ))
}

pub(crate) fn create_private_directory_new(path: &Path) -> io::Result<()> {
    let user = current_user()?;
    let acl = private_acl(&user, OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE)?;
    let mut descriptor_words = vec![0usize; 8];
    let descriptor = descriptor_words.as_mut_ptr().cast::<c_void>();
    if unsafe { InitializeSecurityDescriptor(descriptor, SECURITY_DESCRIPTOR_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetSecurityDescriptorDacl(descriptor, 1, acl.pointer, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetSecurityDescriptorControl(descriptor, SE_DACL_PROTECTED, SE_DACL_PROTECTED) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SecurityAttributes {
        length: u32::try_from(std::mem::size_of::<SecurityAttributes>())
            .map_err(|_| io::Error::other("Windows security attributes are too large"))?,
        security_descriptor: descriptor,
        inherit_handle: 0,
    };
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { CreateDirectoryW(wide_path.as_ptr(), &mut attributes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn is_protected_for_current_user(path: &Path) -> io::Result<bool> {
    let user = current_user()?;
    let mut wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    if descriptor.is_null() {
        return Err(io::Error::other(
            "Windows returned no security descriptor for the store parent",
        ));
    }
    let _descriptor = LocalAllocation(descriptor);
    if dacl.is_null() {
        return Ok(false);
    }
    let mut control = 0u16;
    let mut revision = 0u32;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 {
        return Ok(false);
    }
    let header = unsafe { &*(dacl.cast::<AclHeader>()) };
    if header.ace_count == 0 {
        return Ok(false);
    }
    for index in 0..u32::from(header.ace_count) {
        let mut raw_ace = ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if raw_ace.is_null() {
            return Ok(false);
        }
        let header = unsafe { &*(raw_ace.cast::<AceHeader>()) };
        if header.ace_type != ACCESS_ALLOWED_ACE_TYPE
            || usize::from(header.ace_size) < std::mem::size_of::<AccessAllowedAce>()
        {
            return Ok(false);
        }
        let ace = raw_ace.cast::<AccessAllowedAce>();
        let mask = unsafe { (*ace).mask };
        let sid = unsafe { ptr::addr_of_mut!((*ace).sid_start).cast::<c_void>() };
        if mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS || unsafe { EqualSid(sid, user.sid) } == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sid_can_mutate_ancestor(sid: *mut c_void, user: &CurrentUser) -> bool {
    (unsafe { EqualSid(sid, user.sid) }) != 0
        || (unsafe { IsWellKnownSid(sid, WIN_CREATOR_OWNER_SID) }) != 0
        || (unsafe { IsWellKnownSid(sid, WIN_LOCAL_SYSTEM_SID) }) != 0
        || (unsafe { IsWellKnownSid(sid, WIN_BUILTIN_ADMINISTRATORS_SID) }) != 0
        || is_trusted_installer_sid(sid)
}

fn is_trusted_installer_sid(sid: *mut c_void) -> bool {
    let trusted_installer = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut parsed = ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(trusted_installer.as_ptr(), &mut parsed) } == 0
        || parsed.is_null()
    {
        return false;
    }
    let parsed = LocalAllocation(parsed);
    (unsafe { EqualSid(sid, parsed.0) }) != 0
}

fn ancestor_security_is_safe(
    owner: *mut c_void,
    dacl: *mut c_void,
    user: &CurrentUser,
) -> io::Result<bool> {
    if owner.is_null() || !sid_can_mutate_ancestor(owner, user) || dacl.is_null() {
        return Ok(false);
    }
    let dangerous_rights = DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | GENERIC_ALL;
    let header = unsafe { &*(dacl.cast::<AclHeader>()) };
    for index in 0..u32::from(header.ace_count) {
        let mut raw_ace = ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if raw_ace.is_null() {
            return Ok(false);
        }
        let header = unsafe { &*(raw_ace.cast::<AceHeader>()) };
        if usize::from(header.ace_size) < std::mem::size_of::<AccessAllowedAce>() {
            return Ok(false);
        }
        let ace = raw_ace.cast::<AccessAllowedAce>();
        let mask = unsafe { (*ace).mask };
        if mask & dangerous_rights == 0 {
            continue;
        }
        if matches!(
            header.ace_type,
            ACCESS_ALLOWED_OBJECT_ACE_TYPE
                | ACCESS_ALLOWED_CALLBACK_ACE_TYPE
                | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE
        ) {
            return Ok(false);
        }
        if header.ace_type != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        let sid = unsafe { ptr::addr_of_mut!((*ace).sid_start).cast::<c_void>() };
        if !sid_can_mutate_ancestor(sid, user) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn ancestor_acl_is_safe(path: &Path, user: &CurrentUser) -> io::Result<bool> {
    let mut wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    if descriptor.is_null() {
        return Err(io::Error::other(
            "Windows returned no security descriptor for a store ancestor",
        ));
    }
    let _descriptor = LocalAllocation(descriptor);
    ancestor_security_is_safe(owner, dacl, user)
}

pub(crate) fn ancestor_chain_is_safe(path: &Path) -> io::Result<bool> {
    let user = current_user()?;
    for ancestor in path.ancestors() {
        if !ancestor_acl_is_safe(ancestor, &user)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
pub(crate) fn grant_untrusted_delete_child_for_test(path: &Path) -> io::Result<()> {
    let user = current_user()?;
    let mut world_words = vec![0usize; 16];
    let mut world_bytes = u32::try_from(world_words.len() * std::mem::size_of::<usize>())
        .map_err(|_| io::Error::other("test SID storage is too large"))?;
    if unsafe {
        CreateWellKnownSid(
            WIN_WORLD_SID,
            ptr::null_mut(),
            world_words.as_mut_ptr().cast(),
            &mut world_bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let world_sid = world_words.as_mut_ptr().cast::<c_void>();
    let user_sid_bytes = unsafe { GetLengthSid(user.sid) } as usize;
    let world_sid_bytes = unsafe { GetLengthSid(world_sid) } as usize;
    let acl_bytes = 8usize
        .checked_add(8 + user_sid_bytes)
        .and_then(|bytes| bytes.checked_add(8 + world_sid_bytes))
        .ok_or_else(|| io::Error::other("test ACL is too large"))?;
    let mut acl_words = vec![0u32; acl_bytes.div_ceil(std::mem::size_of::<u32>())];
    let acl = acl_words.as_mut_ptr().cast();
    let acl_length = u32::try_from(acl_words.len() * std::mem::size_of::<u32>())
        .map_err(|_| io::Error::other("test ACL is too large"))?;
    if unsafe { InitializeAcl(acl, acl_length, ACL_REVISION) } == 0
        || unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, 0, FILE_ALL_ACCESS, user.sid) } == 0
        || unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, 0, FILE_DELETE_CHILD, world_sid) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

pub(crate) fn protect(path: &Path) -> io::Result<()> {
    let user = current_user()?;
    let acl = private_acl(&user, OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE)?;

    let mut wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl.pointer,
            ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn private_artifacts_have_a_protected_current_user_acl_from_creation() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-windows-private-acl-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create ACL test root");
        protect(&root).expect("protect ACL test root");
        assert!(is_protected_for_current_user(&root).expect("inspect protected root"));
        assert!(ancestor_chain_is_safe(&root).expect("inspect protected ancestor chain"));

        let directory = root.join("private-directory");
        create_private_directory_new(&directory).expect("create private directory atomically");
        assert!(is_protected_for_current_user(&directory).expect("inspect private directory"));

        let file = directory.join("private-file");
        create_private_new(&file).expect("create private file atomically");
        assert!(is_protected_for_current_user(&file).expect("inspect private file"));
        assert_eq!(
            create_private_new(&file)
                .expect_err("create-new must reject an existing file")
                .kind(),
            io::ErrorKind::AlreadyExists
        );

        let user = current_user().expect("read current test user");
        let benign_dacl = private_acl(&user, OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE)
            .expect("build a victim-only DACL");
        let mut attacker_sid_words = vec![0usize; 16];
        let mut attacker_sid_bytes =
            u32::try_from(attacker_sid_words.len() * std::mem::size_of::<usize>())
                .expect("size attacker SID buffer");
        assert_ne!(
            unsafe {
                CreateWellKnownSid(
                    WIN_WORLD_SID,
                    ptr::null_mut(),
                    attacker_sid_words.as_mut_ptr().cast(),
                    &mut attacker_sid_bytes,
                )
            },
            0,
            "create synthetic attacker SID"
        );
        assert!(
            !ancestor_security_is_safe(
                attacker_sid_words.as_mut_ptr().cast(),
                benign_dacl.pointer,
                &user,
            )
            .expect("validate synthetic owner and DACL"),
            "an untrusted owner must fail even when the visible DACL grants only the victim access"
        );

        grant_untrusted_delete_child_for_test(&directory)
            .expect("grant an untrusted principal delete-child for the regression");
        assert!(
            !ancestor_chain_is_safe(&directory).expect("inspect attacker-writable ancestor"),
            "an untrusted delete-child ACE must make the ancestor chain unsafe"
        );

        fs::remove_dir_all(root).expect("remove ACL test root");
    }
}
