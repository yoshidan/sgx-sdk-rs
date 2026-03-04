use libc::{self, c_int, c_void, timespec};
use std::slice;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

static GLOBAL_TCS_CACHE: OnceLock<SgxTcsInfoCache> = OnceLock::new();

pub struct SeEvent {
    mutex: Mutex<i32>,
    cond: Condvar,
}

impl Default for SeEvent {
    fn default() -> Self {
        Self {
            mutex: Mutex::new(0),
            cond: Condvar::new(),
        }
    }
}

impl SeEvent {
    pub fn new() -> SeEvent {
        SeEvent::default()
    }

    pub fn wait_timeout(&self, timeout: &timespec) -> i32 {
        let Some(timeout) = timespec_to_duration(timeout) else {
            return libc::EINVAL;
        };

        let mut guard = lock_ignore_poison(&self.mutex);

        *guard -= 1;

        let result;
        (guard, result) = self
            .cond
            .wait_timeout_while(guard, timeout, |g| *g < 0)
            .unwrap_or_else(|e| e.into_inner());
        if result.timed_out() && *guard < 0 {
            *guard = 0;
            return libc::ETIMEDOUT;
        }

        0
    }

    pub fn wait(&self) -> i32 {
        let mut guard = lock_ignore_poison(&self.mutex);

        *guard -= 1;

        while *guard < 0 {
            guard = self.cond.wait(guard).unwrap_or_else(|e| e.into_inner());
        }

        0
    }

    pub fn wake(&self) -> i32 {
        let mut guard = lock_ignore_poison(&self.mutex);

        *guard += 1;

        if *guard == 0 {
            self.cond.notify_one();
        }

        0
    }
}

struct SgxTcsInfo<'a> {
    tcs: usize,
    se_event: &'a SeEvent,
}

struct SgxTcsInfoCache<'a> {
    cache: Mutex<Vec<SgxTcsInfo<'a>>>,
}

impl<'a> SgxTcsInfoCache<'a> {
    fn new() -> SgxTcsInfoCache<'a> {
        SgxTcsInfoCache {
            cache: Mutex::new(Vec::new()),
        }
    }

    pub fn get_event(&self, tcs: usize) -> &SeEvent {
        let v = &mut *self.cache.lock().unwrap();
        let op = v.as_slice().iter().position(|x| x.tcs == tcs);
        match op {
            Some(i) => v[i].se_event,
            None => {
                let event: &SeEvent = unsafe { &*Box::into_raw(Box::new(SeEvent::new())) };
                v.push(SgxTcsInfo {
                    tcs,
                    se_event: event,
                });
                let len = v.len();
                v[len - 1].se_event
            }
        }
    }
}

pub fn get_tcs_event(tcs: usize) -> &'static SeEvent {
    let cache = GLOBAL_TCS_CACHE.get_or_init(SgxTcsInfoCache::new);
    cache.get_event(tcs)
}

#[no_mangle]
pub extern "C" fn u_thread_set_event_ocall(error: *mut c_int, tcs: *const c_void) -> c_int {
    if tcs.is_null() {
        if !error.is_null() {
            unsafe {
                *error = libc::EINVAL;
            }
        }
        return -1;
    }
    let result = get_tcs_event(tcs as usize).wake();
    if result != 0 {
        if !error.is_null() {
            unsafe {
                *error = result;
            }
        }
        -1
    } else {
        if !error.is_null() {
            unsafe {
                *error = 0;
            }
        }
        result as c_int
    }
}

#[no_mangle]
pub extern "C" fn u_thread_wait_event_ocall(
    error: *mut c_int,
    tcs: *const c_void,
    timeout: *const timespec,
) -> c_int {
    if tcs.is_null() {
        if !error.is_null() {
            unsafe {
                *error = libc::EINVAL;
            }
        }
        return -1;
    }

    let result = if timeout.is_null() {
        get_tcs_event(tcs as usize).wait()
    } else {
        get_tcs_event(tcs as usize).wait_timeout(unsafe { &*timeout })
    };
    if result != 0 {
        if !error.is_null() {
            unsafe {
                *error = result;
            }
        }
        -1
    } else {
        if !error.is_null() {
            unsafe {
                *error = 0;
            }
        }
        result as c_int
    }
}

#[no_mangle]
pub extern "C" fn u_thread_set_multiple_events_ocall(
    error: *mut c_int,
    tcss: *const *const c_void,
    total: c_int,
) -> c_int {
    if tcss.is_null() {
        if !error.is_null() {
            unsafe {
                *error = libc::EINVAL;
            }
        }
        return -1;
    }

    let tcss_slice = unsafe { slice::from_raw_parts(tcss, total as usize) };
    let mut result = 0;
    for tcs in tcss_slice.iter() {
        result = get_tcs_event(*tcs as usize).wake();
        if result != 0 {
            if !error.is_null() {
                unsafe { *error = result }
            }
            return -1;
        }
    }

    if !error.is_null() {
        unsafe {
            *error = 0;
        }
    }
    result as c_int
}

#[no_mangle]
pub extern "C" fn u_thread_setwait_events_ocall(
    error: *mut c_int,
    waiter_tcs: *const c_void,
    self_tcs: *const c_void,
    timeout: *const timespec,
) -> c_int {
    let result = u_thread_set_event_ocall(error, waiter_tcs);
    if result < 0 {
        result
    } else {
        u_thread_wait_event_ocall(error, self_tcs, timeout)
    }
}

fn timespec_to_duration(t: &timespec) -> Option<Duration> {
    if (t.tv_nsec < 0) || (t.tv_nsec > 999_999_999) {
        return None;
    }
    let Ok(secs) = t.tv_sec.try_into() else {
        return None;
    };
    let Ok(nsecs) = t.tv_nsec.try_into() else {
        return None;
    };
    Duration::from_secs(secs).checked_add(Duration::from_nanos(nsecs))
}

// Acquire a mutex while ignoring its poisoned state. This is to maintain
// historical behavior.
fn lock_ignore_poison(m: &Mutex<i32>) -> std::sync::MutexGuard<'_, i32> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
