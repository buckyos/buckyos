// Windows console handling for child processes spawned by BuckyOS services.
//
// `DETACHED_PROCESS` must never be used to hide a child process: it leaves the
// child without any console at all, and Windows then allocates a *visible*
// console for every grandchild that is started without explicit creation flags
// (docker credential helpers, docker context helpers, python launchers, ...).
// `CREATE_NO_WINDOW` instead gives the child its own windowless console, which
// grandchildren inherit, so nothing can pop up a black window.
//
// `CREATE_NO_WINDOW` is also silently ignored when combined with
// `DETACHED_PROCESS` or `CREATE_NEW_CONSOLE`, so the two must not be mixed.

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Creation flags for short-lived child processes whose stdio we capture.
#[cfg(windows)]
pub fn windows_no_window_creation_flags() -> u32 {
    CREATE_NO_WINDOW
}

/// Creation flags for long-running background children that must survive the
/// parent and stay isolated from its Ctrl-C / Ctrl-Break signals.
#[cfg(windows)]
pub fn windows_background_creation_flags() -> u32 {
    CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
}

pub fn hide_child_console(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_no_window_creation_flags());
    }
    command
}

pub fn hide_child_console_async(
    command: &mut tokio::process::Command,
) -> &mut tokio::process::Command {
    #[cfg(windows)]
    {
        command.creation_flags(windows_no_window_creation_flags());
    }
    command
}

pub fn hide_background_child_console(
    command: &mut std::process::Command,
) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_background_creation_flags());
    }
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    #[test]
    fn hidden_flags_never_defeat_create_no_window() {
        for flags in [
            windows_no_window_creation_flags(),
            windows_background_creation_flags(),
        ] {
            assert_eq!(flags & CREATE_NO_WINDOW, CREATE_NO_WINDOW);
            assert_eq!(flags & DETACHED_PROCESS, 0);
            assert_eq!(flags & CREATE_NEW_CONSOLE, 0);
        }
    }
}
