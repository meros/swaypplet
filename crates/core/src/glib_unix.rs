//! Local re-implementation of `glib::unix_signal_add_local` and
//! `glib::unix_fd_add_local`, which gtk-rs-core removed in glib 0.21 without
//! a safe replacement. The C symbols still live in the libglib-2.0 that
//! glib-sys links, so only the safe wrappers (trampoline + `ThreadGuard`,
//! same shape as glib 0.20's `source.rs`) are needed here.

use std::cell::RefCell;
use std::os::fd::RawFd;

use glib::ffi::{G_PRIORITY_DEFAULT, gboolean, gpointer};
use glib::thread_guard::ThreadGuard;
use glib::translate::{IntoGlib, from_glib};
use glib::{ControlFlow, IOCondition, MainContext, SourceId};

unsafe extern "C" {
    fn g_unix_signal_add_full(
        priority: std::ffi::c_int,
        signum: std::ffi::c_int,
        handler: Option<unsafe extern "C" fn(gpointer) -> gboolean>,
        data: gpointer,
        notify: Option<unsafe extern "C" fn(gpointer)>,
    ) -> std::ffi::c_uint;

    fn g_unix_fd_add_full(
        priority: std::ffi::c_int,
        fd: std::ffi::c_int,
        condition: glib::ffi::GIOCondition,
        handler: Option<
            unsafe extern "C" fn(std::ffi::c_int, glib::ffi::GIOCondition, gpointer) -> gboolean,
        >,
        data: gpointer,
        notify: Option<unsafe extern "C" fn(gpointer)>,
    ) -> std::ffi::c_uint;
}

unsafe extern "C" fn signal_trampoline<F: FnMut() -> ControlFlow + 'static>(
    func: gpointer,
) -> gboolean {
    let func = unsafe { &*(func as *const ThreadGuard<RefCell<F>>) };
    (*func.get_ref().borrow_mut())().into_glib()
}

unsafe extern "C" fn fd_trampoline<F: FnMut(RawFd, IOCondition) -> ControlFlow + 'static>(
    fd: std::ffi::c_int,
    condition: glib::ffi::GIOCondition,
    func: gpointer,
) -> gboolean {
    let func = unsafe { &*(func as *const ThreadGuard<RefCell<F>>) };
    (*func.get_ref().borrow_mut())(fd, unsafe { from_glib(condition) }).into_glib()
}

unsafe extern "C" fn destroy_closure<F>(ptr: gpointer) {
    let _ = unsafe { Box::<ThreadGuard<RefCell<F>>>::from_raw(ptr as *mut _) };
}

fn into_raw<F>(func: F) -> gpointer {
    Box::into_raw(Box::new(ThreadGuard::new(RefCell::new(func)))) as gpointer
}

/// Call `func` on the default main loop whenever UNIX signal `signum` is
/// raised, until it returns `ControlFlow::Break`. Must be called from the
/// thread owning the default main context (panics otherwise).
pub fn signal_add_local<F>(signum: i32, func: F) -> SourceId
where
    F: FnMut() -> ControlFlow + 'static,
{
    unsafe {
        let context = MainContext::default();
        let _acquire = context
            .acquire()
            .expect("default main context already acquired by another thread");
        from_glib(g_unix_signal_add_full(
            G_PRIORITY_DEFAULT,
            signum,
            Some(signal_trampoline::<F>),
            into_raw(func),
            Some(destroy_closure::<F>),
        ))
    }
}

/// Call `func` on the default main loop whenever `fd` matches `condition`,
/// until it returns `ControlFlow::Break`. Must be called from the thread
/// owning the default main context (panics otherwise).
pub fn fd_add_local<F>(fd: RawFd, condition: IOCondition, func: F) -> SourceId
where
    F: FnMut(RawFd, IOCondition) -> ControlFlow + 'static,
{
    unsafe {
        let context = MainContext::default();
        let _acquire = context
            .acquire()
            .expect("default main context already acquired by another thread");
        from_glib(g_unix_fd_add_full(
            G_PRIORITY_DEFAULT,
            fd,
            condition.into_glib(),
            Some(fd_trampoline::<F>),
            into_raw(func),
            Some(destroy_closure::<F>),
        ))
    }
}
