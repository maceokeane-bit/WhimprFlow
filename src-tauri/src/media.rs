//! Pause/resume the system "now playing" app while dictating, via the macOS
//! MediaRemote private framework.
//!
//! This controls **whatever app is currently the system now-playing app**
//! (Chrome/YouTube, Spotify, Apple Music, VLC, …) uniformly — unlike the old
//! `osascript` path which only spoke Spotify and Apple Music. It also needs no
//! Apple-Events/Automation TCC permission, so it survives rebuilds.
//!
//! Behavior contract (user choice: "only resume what was playing"):
//! - On dictation start: if the now-playing app is **playing**, send Pause and
//!   record that we paused it.
//! - On dictation stop: if **we** paused it, send Play; otherwise do nothing.
//! - Never starts audio that wasn't playing, never resumes something we didn't
//!   pause.
//!
//! Threading/race model: `on_dictation_start` runs in a `run_on_main_thread`
//! closure (main queue). It asks MediaRemote for `isPlaying`, delivering the
//! answer via a block on the main queue (FIFO, after the start closure). When
//! the block fires it sends Pause and sets `PAUSED`. `on_dictation_stop`'s
//! main-queue closure runs after the start closure; in the common case the
//! IsPlaying block has already fired (tens of ms) so `PAUSED` is set and stop
//! resumes correctly. If the block runs late (very short dictation / slow
//! getter), stop sees `PAUSED=false` and does nothing → audio stays paused,
//! the **safe** failure direction (never a wrong resume). Nothing blocks the
//! main thread.

use std::sync::atomic::{AtomicBool, Ordering};

/// True iff *this* dictation session paused the now-playing app.
static PAUSED: AtomicBool = AtomicBool::new(false);

/// Called when microphone capture begins (must run on the main thread).
pub fn on_dictation_start() {
    #[cfg(target_os = "macos")]
    {
        // Clear any stale resume flag from a previous interrupted session.
        PAUSED.store(false, Ordering::SeqCst);

        // Ask the system "is the now-playing app playing right now?" The
        // answer arrives asynchronously on the main queue as a block; since we
        // are already on the main thread, the block fires FIFO after this
        // closure returns. We do nothing synchronously here.
        let handler = block2::RcBlock::new(move |playing: i8| {
            let is_playing = playing != 0;
            if is_playing {
                // SAFETY: SendCommand is safe to call on any thread; we're on
                // the main queue. NULL options (no playback dict).
                unsafe { mac::send_command(mac::COMMAND_PAUSE) };
                PAUSED.store(true, Ordering::SeqCst);
                eprintln!("[whimpr] now-playing playing → paused");
            } else {
                eprintln!("[whimpr] now-playing not playing → untouched");
            }
        });

        // SAFETY: main_queue is the libdispatch main-queue global; handler is a
        // valid, retained (heap) block that lives until MediaRemote is done
        // with it (the framework retains the block for the async call).
        unsafe {
            mac::get_now_playing_is_playing(mac::main_queue(), block2::RcBlock::as_ptr(&handler));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = PAUSED;
    }
}

/// Called when capture ends (finalize, discard, or cancel). Main thread only.
/// Called unconditionally — it no-ops when nothing was paused — so every pause
/// is paired with a resume and a stale flag from a prior session can't survive
/// to unpause audio you didn't have playing this time. Resume only ever targets
/// what `on_dictation_start` paused, i.e. what was actively playing.
pub fn on_dictation_stop() {
    if PAUSED.swap(false, Ordering::SeqCst) {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: SendCommand on the main queue, NULL options.
            unsafe { mac::send_command(mac::COMMAND_PLAY) };
            eprintln!("[whimpr] resumed now-playing");
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    use block2::Block;

    /// MediaRemote `MRMediaRemoteSendCommand` command enum values.
    ///
    /// These are private, undocumented constants (no public header). They were
    /// verified empirically against the running system: Pause=2 stops the
    /// now-playing app, Play=1 resumes it. ABI has been stable since ~10.10.
    pub(super) const COMMAND_PAUSE: isize = 2;
    pub(super) const COMMAND_PLAY: isize = 1;

    #[link(name = "MediaRemote", kind = "framework")]
    extern "C" {
        // void MRMediaRemoteGetNowPlayingApplicationIsPlaying(dispatch_queue_t,
        //                                                      void (^)(BOOL));
        // BOOL is a 1-byte integer (signed char on x86_64, bool on arm64) —
        // ABI-identical to i8 for block argument passing, so the block is
        // typed with i8 and we treat nonzero as "playing".
        fn MRMediaRemoteGetNowPlayingApplicationIsPlaying(
            queue: *mut c_void,
            handler: *mut Block<dyn Fn(i8)>,
        );

        // void MRMediaRemoteSendCommand(NSInteger command, NSDictionary *options);
        fn MRMediaRemoteSendCommand(command: isize, options: *mut c_void);
    }

    // libdispatch main queue global. `dispatch_get_main_queue()` is a header
    // macro/inline that returns `&_dispatch_main_q`, so there is no function
    // symbol to link against — we reference the global directly.
    extern "C" {
        static _dispatch_main_q: c_void;
    }

    /// Return the libdispatch main queue (the object `dispatch_get_main_queue()`
    /// yields). We pass this to MediaRemote so the IsPlaying block fires on the
    /// main queue, FIFO with our `run_on_main_thread` closures.
    pub(super) fn main_queue() -> *mut c_void {
        core::ptr::addr_of!(_dispatch_main_q) as *mut c_void
    }

    /// Send a MediaRemote command (Pause/Play) to the current now-playing app.
    ///
    /// # Safety
    /// No precondition beyond linking the framework (always satisfied on
    /// macOS). `command` must be a valid MRMediaRemoteCommand; the constants
    /// above are verified. `NULL` options is the documented "no extras" value.
    pub(super) unsafe fn send_command(command: isize) {
        MRMediaRemoteSendCommand(command, core::ptr::null_mut());
    }

    /// Ask "is the now-playing app playing right now?" The block is invoked on
    /// `queue` with `YES`/`NO`.
    ///
    /// # Safety
    /// `queue` must be a valid dispatch queue. `handler` must be a valid block
    /// (e.g. produced by `RcBlock::new`); the framework retains it for the
    /// duration of the async fetch, so a retained/heap block (RcBlock) is
    /// required.
    pub(super) unsafe fn get_now_playing_is_playing(
        queue: *mut c_void,
        handler: *mut Block<dyn Fn(i8)>,
    ) {
        MRMediaRemoteGetNowPlayingApplicationIsPlaying(queue, handler);
    }
}