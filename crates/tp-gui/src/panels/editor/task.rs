use gtk4::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Run a blocking task on a background thread and deliver the result back on the GTK main loop.
///
/// This keeps UI code single-threaded while allowing filesystem/git/SSH work to happen off-thread.
/// Global counter of live `run_blocking` tasks (spawn ↑ / completion ↓).
/// Used only for diagnostic tracing of thread churn around hot paths.
static IN_FLIGHT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn run_blocking<T, F, C>(task: F, on_done: C)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let slot = Arc::new(Mutex::new(None::<T>));
    let slot_thread = slot.clone();
    let callback = Rc::new(RefCell::new(Some(on_done)));

    let in_flight = IN_FLIGHT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    tracing::debug!("task.run_blocking: thread spawn, in_flight={}", in_flight);
    std::thread::spawn(move || {
        let result = task();
        *slot_thread.lock().unwrap() = Some(result);
        let remaining = IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) - 1;
        tracing::debug!("task.run_blocking: thread done, in_flight={}", remaining);
    });

    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let result = slot.lock().unwrap().take();
        match result {
            Some(value) => {
                if let Some(cb) = callback.borrow_mut().take() {
                    cb(value);
                }
                glib::ControlFlow::Break
            }
            None => glib::ControlFlow::Continue,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn run_blocking_delivers_result_on_main_loop() {
        crate::test_support::run_on_gtk_thread(|| {
            let main_loop = glib::MainLoop::new(None, false);
            let observed = Rc::new(RefCell::new(None));
            let observed_c = observed.clone();
            let main_loop_c = main_loop.clone();

            run_blocking(
                || 42,
                move |value| {
                    *observed_c.borrow_mut() = Some(value);
                    main_loop_c.quit();
                },
            );

            main_loop.run();
            assert_eq!(*observed.borrow(), Some(42));
        });
    }
}
