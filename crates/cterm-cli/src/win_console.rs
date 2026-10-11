//! Windows host-console raw mode for the wrapper.
//!
//! On Windows the pseudo-console (ConPTY) is the *child's* terminal. The *host*
//! console — our own stdin/stdout — must be configured separately, or input
//! stays line-buffered and echoed, and ConPTY's VT output bytes print literally
//! instead of rendering. This is the Windows analogue of a Unix tty raw mode,
//! and it is a host-console concern, independent of which PTY backend we use.
//!
//! ## Attribution
//!
//! The console-mode flag choices here are adapted from **zhiburt/conpty**
//! (MIT License, Copyright (c) 2021 Maxim Zhiburt),
//! <https://github.com/zhiburt/conpty> — specifically `src/console.rs`
//! (`Console::set_raw` / `set_raw_stdin`) for the stdin raw flags and
//! `src/process.rs` (`enableVirtualTerminalSequenceProcessing`) for enabling
//! `ENABLE_VIRTUAL_TERMINAL_PROCESSING` on stdout. We call the Win32 console API
//! directly via `windows-sys` rather than vendoring their code.
//!
//! ### Limitations observed in zhiburt/conpty (and inherited considerations)
//!
//! * `Console::set_raw` calls `GetConsoleMode`/`SetConsoleMode` unconditionally;
//!   if a standard stream is redirected (not a real console) those calls fail.
//!   We instead treat failure as "not a console" and no-op, so a wrapped shell
//!   run with piped I/O (e.g. in tests or scripts) still works.
//! * It enables `ENABLE_QUICK_EDIT_MODE`/`ENABLE_INSERT_MODE`; for a transparent
//!   passthrough we leave those off (with `ENABLE_EXTENDED_FLAGS`) so mouse and
//!   paste reach the child.
//! * Its `process.rs` notes VT-sequence setup is silently ignored for
//!   `#![windows_subsystem = "windows"]` processes (no console), and that
//!   `set_echo` is of uncertain usefulness — neither affects a normal CLI shim.
//! * Window-resize events (the SIGWINCH analogue) are not handled here yet.
#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, DISABLE_NEWLINE_AUTO_RETURN,
    ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_MOUSE_INPUT,
    ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

/// Restores the host console's original stdin/stdout modes when dropped.
pub struct WinConsoleGuard {
    stdin: HANDLE,
    stdin_mode: CONSOLE_MODE,
    stdout: HANDLE,
    stdout_mode: CONSOLE_MODE,
}

/// Reads a console mode, returning `None` if the handle is not a console
/// (e.g. redirected to a pipe or file).
unsafe fn console_mode(handle: HANDLE) -> Option<CONSOLE_MODE> {
    let mut mode: CONSOLE_MODE = 0;
    if GetConsoleMode(handle, &mut mode) != 0 {
        Some(mode)
    } else {
        None
    }
}

impl WinConsoleGuard {
    /// Puts stdin into raw/VT-input mode and enables VT processing on stdout.
    /// Returns `None` (a no-op) when either stream is not a real console, so
    /// piped/redirected invocations are unaffected.
    pub fn enable() -> Option<Self> {
        unsafe {
            let stdin = GetStdHandle(STD_INPUT_HANDLE);
            let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
            if stdin == INVALID_HANDLE_VALUE || stdout == INVALID_HANDLE_VALUE {
                return None;
            }

            let stdin_mode = console_mode(stdin)?;
            let stdout_mode = console_mode(stdout)?;

            // Raw input: no echo, no line buffering, no input processing, no
            // mouse records; deliver keys as a VT stream to forward to the PTY.
            let mut raw_in = stdin_mode;
            raw_in &= !(ENABLE_ECHO_INPUT
                | ENABLE_LINE_INPUT
                | ENABLE_PROCESSED_INPUT
                | ENABLE_MOUSE_INPUT);
            raw_in |= ENABLE_EXTENDED_FLAGS | ENABLE_VIRTUAL_TERMINAL_INPUT;
            if SetConsoleMode(stdin, raw_in) == 0 {
                return None;
            }

            // Render the child's VT output instead of printing escape bytes.
            let mut vt_out = stdout_mode;
            vt_out |= ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN;
            if SetConsoleMode(stdout, vt_out) == 0 {
                let _ = SetConsoleMode(stdin, stdin_mode); // restore before bailing
                return None;
            }

            Some(Self {
                stdin,
                stdin_mode,
                stdout,
                stdout_mode,
            })
        }
    }
}

impl Drop for WinConsoleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SetConsoleMode(self.stdin, self.stdin_mode);
            let _ = SetConsoleMode(self.stdout, self.stdout_mode);
        }
    }
}
