//! Who a uid is: the login name from the passwd database. The lock screen
//! authenticates as the current user, user switching compares against it,
//! and the polkit agent names the identity a dialog asks for.

/// The login name of `uid`, from its passwd entry.
pub fn name_of(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 1024];
    // SAFETY: a zeroed passwd is a valid out-parameter; getpwuid_r fills it
    // with pointers into `buf`, which outlives every read below.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(
            uid as libc::uid_t,
            &mut pwd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
        return None;
    }
    // SAFETY: pw_name points into `buf` and is NUL-terminated.
    let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
    Some(name.to_string_lossy().into_owned()).filter(|n| !n.is_empty())
}

/// The user running this process: the passwd entry for our uid
/// (authoritative), `$USER` as the fallback for odd environments.
pub fn current() -> Option<String> {
    // SAFETY: getuid cannot fail.
    name_of(unsafe { libc::getuid() })
        .or_else(|| std::env::var("USER").ok().filter(|u| !u.is_empty()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn root_is_root_and_we_are_someone() {
        assert_eq!(super::name_of(0).as_deref(), Some("root"));
        assert!(super::current().is_some());
    }
}
